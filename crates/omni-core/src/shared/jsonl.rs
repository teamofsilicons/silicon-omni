//! Append-only JSONL, the one storage primitive omni uses.
//!
//! Reads tolerate a torn final line: a reader must never crash on a file that
//! is being appended to right now.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

/// One writer at a time, so a line is never interleaved with another's.
static WRITE: Mutex<()> = Mutex::new(());

/// A decoded JSONL record and the physical line it came from.
#[derive(Debug)]
pub struct Record {
    pub line: usize,
    pub value: Value,
}

/// Why a checked read could not consume the complete durable prefix.
#[derive(Debug)]
pub enum ReadError {
    Io {
        line: Option<usize>,
        source: io::Error,
    },
    Malformed {
        line: usize,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io {
                line: Some(line),
                source,
            } => {
                write!(formatter, "could not read JSONL line {line}: {source}")
            }
            ReadError::Io { line: None, source } => {
                write!(formatter, "could not open JSONL file: {source}")
            }
            ReadError::Malformed { line, source } => {
                write!(
                    formatter,
                    "malformed JSON on complete line {line}: {source}"
                )
            }
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReadError::Io { source, .. } => Some(source),
            ReadError::Malformed { source, .. } => Some(source),
        }
    }
}

/// Result of a checked read.
///
/// Missing and empty files are both healthy, but `missing` keeps them
/// distinguishable. `records` is the valid prefix before `error`, if any. An
/// invalid unterminated final line is treated as a torn write, not corruption;
/// [`extend`] removes that fragment before the next append.
#[derive(Debug, Default)]
pub struct ReadReport {
    pub missing: bool,
    pub records: Vec<Record>,
    pub error: Option<ReadError>,
}

pub fn append(path: &Path, value: &Value) -> std::io::Result<()> {
    extend(path, std::slice::from_ref(value))
}

/// One exclusive, long-lived writer for a JSONL file.
///
/// The generic [`append`] and [`extend`] helpers reopen and revalidate a file
/// because they cannot know who else may have touched it between calls. A live
/// session has a stronger invariant: its conductor is the only writer. Keeping
/// that descriptor avoids an open, chmod, stat, seek, and tail read for every
/// event while retaining the same `sync_data` durability boundary.
///
/// This is crate-private deliberately. Two persistent appenders for one path
/// would violate the exclusive-writer invariant; callers without that guarantee
/// must use [`append`] or [`extend`].
pub(crate) struct Appender {
    path: PathBuf,
    file: Option<File>,
    prefix_newline: bool,
}

impl Appender {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let _guard = WRITE.lock().unwrap_or_else(|poison| poison.into_inner());
        let existed = path.exists();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        // `mode` only applies on creation. Repair logs written by an older
        // release once when this live writer takes ownership of them.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        let prefix_newline = repair_tail(&mut file)?;
        if !existed {
            if let Some(parent) = path.parent() {
                // The descriptor will sync every event's contents. Make its
                // directory entry durable once, when the descriptor is born.
                File::open(parent)?.sync_all()?;
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(file),
            prefix_newline,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn append(&mut self, value: &Value) -> io::Result<()> {
        let mut body = value.to_string();
        body.push('\n');
        if self.prefix_newline {
            body.insert(0, '\n');
        }

        // Once a write has begun, an error leaves its extent uncertain. Poison
        // this descriptor rather than letting a caller append another record
        // behind a possibly partial line. The conductor treats any such failure
        // as terminal; a cold reopen can perform ordinary torn-tail recovery.
        self.prefix_newline = false;
        let result = match self.file.as_mut() {
            Some(file) => file
                .write_all(body.as_bytes())
                .and_then(|()| file.sync_data()),
            None => Err(io::Error::other("JSONL appender is no longer writable")),
        };
        if result.is_err() {
            self.file = None;
        }
        result
    }
}

/// One write, so a reader never sees half a batch.
pub fn extend(path: &Path, values: &[Value]) -> std::io::Result<()> {
    if values.is_empty() {
        return Ok(());
    }
    let mut body = String::new();
    for value in values {
        body.push_str(&value.to_string());
        body.push('\n');
    }
    let _guard = WRITE.lock().unwrap_or_else(|poison| poison.into_inner());
    let existed = path.exists();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    // `mode` only applies when the file is first created. Repair older state
    // as it is touched too, since histories can contain credentials and tool
    // output even when the enclosing directory is already private.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    if repair_tail(&mut file)? {
        body.insert(0, '\n');
    }
    file.write_all(body.as_bytes())?;
    // A successful append is the durability boundary used by the conductor:
    // it may publish or acknowledge an event only after this returns.
    file.sync_data()?;
    if !existed {
        if let Some(parent) = path.parent() {
            // File data without its directory entry is not a durable append on
            // a newly-created session. Existing logs need only the data sync.
            File::open(parent)?.sync_all()?;
        }
    }
    Ok(())
}

/// Repair a partial last record, or report that a valid last record merely
/// needs its missing delimiter before the next append.
fn repair_tail(file: &mut File) -> io::Result<bool> {
    // A process can die halfway through its last line. Preserve it when it is
    // complete JSON that merely lacks its delimiter. Otherwise remove only
    // that partial tail; keeping it as a complete malformed middle line would
    // make corruption look like an intentionally skipped record forever.
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0_u8; 1];
    file.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(false);
    }
    let (tail_start, tail) = unterminated_tail(file, len)?;
    if serde_json::from_slice::<Value>(&tail).is_ok() {
        Ok(true)
    } else {
        file.set_len(tail_start)?;
        Ok(false)
    }
}

/// Locate and copy only the bytes after the final newline, reading backwards
/// in bounded chunks so appending does not rescan a long session log.
fn unterminated_tail(file: &mut File, len: u64) -> io::Result<(u64, Vec<u8>)> {
    const CHUNK: u64 = 8 * 1024;

    let mut cursor = len;
    let mut later_chunks: Vec<Vec<u8>> = Vec::new();
    loop {
        let start = cursor.saturating_sub(CHUNK);
        let mut chunk = vec![0; (cursor - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;

        if let Some(newline) = chunk.iter().rposition(|byte| *byte == b'\n') {
            let mut tail = chunk[newline + 1..].to_vec();
            for later in later_chunks.iter().rev() {
                tail.extend_from_slice(later);
            }
            return Ok((start + newline as u64 + 1, tail));
        }

        later_chunks.push(chunk);
        if start == 0 {
            let mut tail = Vec::new();
            for part in later_chunks.iter().rev() {
                tail.extend_from_slice(part);
            }
            return Ok((0, tail));
        }
        cursor = start;
    }
}

/// Read the durable JSONL prefix and report complete-line corruption.
///
/// A missing file is `missing = true`; an existing empty file has no records
/// and `missing = false`. I/O failures and malformed newline-terminated lines
/// are retained in `error`. A malformed final line without a newline is the
/// one tolerated case because it can only be an interrupted append.
pub fn read_checked(path: &Path) -> ReadReport {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return ReadReport {
                missing: true,
                ..ReadReport::default()
            };
        }
        Err(source) => {
            return ReadReport {
                error: Some(ReadError::Io { line: None, source }),
                ..ReadReport::default()
            };
        }
    };

    let mut report = ReadReport::default();
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    let mut line = 0;
    loop {
        bytes.clear();
        let read = match reader.read_until(b'\n', &mut bytes) {
            Ok(read) => read,
            Err(source) => {
                report.error = Some(ReadError::Io {
                    line: Some(line + 1),
                    source,
                });
                break;
            }
        };
        if read == 0 {
            break;
        }
        line += 1;
        let complete = bytes.last() == Some(&b'\n');
        match serde_json::from_slice(&bytes) {
            Ok(value) => report.records.push(Record { line, value }),
            Err(_) if !complete => break,
            Err(source) => {
                report.error = Some(ReadError::Malformed { line, source });
                break;
            }
        }
    }
    report
}

/// Best-effort compatibility read for provider-native transcript files.
///
/// Session storage uses [`read_checked`] so complete corruption is surfaced.
/// Claude's own evolving JSONL format only needs every record it understands,
/// so this intentionally retains the historical skip-unknown behavior.
pub fn read(path: &Path) -> Vec<Value> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(line.trim()).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("omni-jsonl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("log.jsonl")
    }

    #[test]
    fn a_missing_file_is_an_empty_log_not_an_error() {
        let report = read_checked(Path::new("/nowhere/at/all.jsonl"));
        assert!(report.missing);
        assert!(report.records.is_empty());
        assert!(report.error.is_none());

        let path = scratch("empty");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
        let report = read_checked(&path);
        assert!(!report.missing);
        assert!(report.records.is_empty());
        assert!(report.error.is_none());
    }

    #[test]
    fn what_goes_in_comes_back_in_order() {
        let path = scratch("order");
        append(&path, &json!({"n": 1})).unwrap();
        extend(&path, &[json!({"n": 2}), json!({"n": 3})]).unwrap();
        let seen: Vec<i64> = read(&path)
            .iter()
            .map(|v| v["n"].as_i64().unwrap())
            .collect();
        assert_eq!(seen, vec![1, 2, 3]);
    }

    #[test]
    fn a_persistent_appender_keeps_writing_one_ordered_log() {
        let path = scratch("persistent-order");
        let mut appender = Appender::open(&path).unwrap();

        appender.append(&json!({"n": 1})).unwrap();
        appender.append(&json!({"n": 2})).unwrap();
        appender.append(&json!({"n": 3})).unwrap();

        let seen: Vec<i64> = read(&path)
            .iter()
            .map(|value| value["n"].as_i64().unwrap())
            .collect();
        assert_eq!(seen, vec![1, 2, 3]);
    }

    #[test]
    fn a_persistent_appender_preserves_valid_json_missing_its_delimiter() {
        let path = scratch("persistent-valid-tail");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{\"n\":1}").unwrap();

        let mut appender = Appender::open(&path).unwrap();
        appender.append(&json!({"n": 2})).unwrap();

        let report = read_checked(&path);
        assert!(report.error.is_none());
        let seen: Vec<i64> = report
            .records
            .iter()
            .map(|record| record.value["n"].as_i64().unwrap())
            .collect();
        assert_eq!(seen, vec![1, 2]);
    }

    #[test]
    fn a_persistent_appender_repairs_a_torn_tail_before_its_first_write() {
        let path = scratch("persistent-torn-tail");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{\"n\":1}\n{\"n\":2").unwrap();

        let mut appender = Appender::open(&path).unwrap();
        appender.append(&json!({"n": 3})).unwrap();

        let report = read_checked(&path);
        assert!(report.error.is_none());
        let seen: Vec<i64> = report
            .records
            .iter()
            .map(|record| record.value["n"].as_i64().unwrap())
            .collect();
        assert_eq!(seen, vec![1, 3]);
        assert!(!std::fs::read_to_string(path).unwrap().contains("{\"n\":2"));
    }

    #[test]
    fn a_torn_line_is_skipped_rather_than_fatal() {
        let path = scratch("torn");
        append(&path, &json!({"n": 1})).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"n\": 2\n").unwrap(); // cut off mid-write
        append(&path, &json!({"n": 3})).unwrap();
        let seen: Vec<i64> = read(&path)
            .iter()
            .map(|v| v["n"].as_i64().unwrap())
            .collect();
        assert_eq!(seen, vec![1, 3]);
    }

    #[test]
    fn an_unterminated_torn_line_cannot_swallow_the_next_event() {
        let path = scratch("unterminated");
        append(&path, &json!({"n": 1})).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"n\": 2").unwrap(); // process died before newline
        drop(file);

        append(&path, &json!({"n": 3})).unwrap();

        let report = read_checked(&path);
        assert!(report.error.is_none());
        let seen: Vec<i64> = report
            .records
            .iter()
            .map(|record| record.value["n"].as_i64().unwrap())
            .collect();
        assert_eq!(seen, vec![1, 3]);
        assert!(
            !std::fs::read_to_string(path).unwrap().contains("{\"n\": 2"),
            "the torn bytes were truncated instead of fossilized as corruption"
        );
    }

    #[test]
    fn valid_json_without_a_final_newline_is_preserved_before_append() {
        let path = scratch("valid-unterminated");
        append(&path, &json!({"n": 1})).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"n\":2}").unwrap();
        drop(file);

        append(&path, &json!({"n": 3})).unwrap();

        let report = read_checked(&path);
        assert!(report.error.is_none());
        let seen: Vec<i64> = report
            .records
            .iter()
            .map(|record| record.value["n"].as_i64().unwrap())
            .collect();
        assert_eq!(seen, vec![1, 2, 3]);
    }

    #[test]
    fn checked_read_reports_a_malformed_complete_line() {
        let path = scratch("malformed-complete");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{\"n\":1}\n{\"n\":\n{\"n\":3}\n").unwrap();

        let report = read_checked(&path);

        assert_eq!(report.records.len(), 1, "only the known-good prefix loads");
        assert!(matches!(
            report.error,
            Some(ReadError::Malformed { line: 2, .. })
        ));
    }

    #[test]
    fn checked_read_does_not_confuse_io_failure_with_an_empty_file() {
        let path = scratch("io-error");
        std::fs::create_dir_all(&path).unwrap();

        let report = read_checked(&path);

        assert!(!report.missing);
        assert!(matches!(report.error, Some(ReadError::Io { .. })));
    }

    #[test]
    fn appending_repairs_permissions_and_flushes_private_state() {
        use std::os::unix::fs::PermissionsExt;

        let path = scratch("permissions");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        append(&path, &json!({"secret": true})).unwrap();

        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn a_persistent_appender_takes_ownership_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let path = scratch("persistent-permissions");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut appender = Appender::open(&path).unwrap();
        appender.append(&json!({"secret": true})).unwrap();

        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

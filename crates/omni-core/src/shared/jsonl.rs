//! Append-only JSONL, the one storage primitive omni uses.
//!
//! Reads tolerate a torn final line: a reader must never crash on a file that
//! is being appended to right now.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Mutex;

use serde_json::Value;

/// One writer at a time, so a line is never interleaved with another's.
static WRITE: Mutex<()> = Mutex::new(());

pub fn append(path: &Path, value: &Value) -> std::io::Result<()> {
    extend(path, std::slice::from_ref(value))
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(body.as_bytes())
}

/// Every well-formed line in the file. Malformed ones are skipped, not fatal.
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
        assert!(read(Path::new("/nowhere/at/all.jsonl")).is_empty());
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
}

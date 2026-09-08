//! What the bridge remembers between runs, and who is allowed through.
//!
//! Everything lives in one 0600 file, `web.json`, beside the daemon socket.
//! It holds three things: the port the bridge last managed to bind, the
//! control token that proves a caller could read this file, and the grants
//! that websites are currently holding.
//!
//! The rule that shapes all of it: **a grant belongs to a port.** Come back up
//! on the same port and the websites you paired are still paired. Come back up
//! anywhere else — because something else took the port while you were gone —
//! and every grant is dropped, because from a browser's side that is a
//! different address and it has no way to know the bridge behind it is the
//! same one.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::keys;
use crate::sha256;

/// Where the bridge would rather live.
pub const BASE_PORT: u16 = 1998;
/// How far down it will count before giving up. Ten doors is enough that a
/// website can knock on all of them, and few enough that it is quick.
pub const FLOOR_PORT: u16 = 1989;
/// How long a pairing code stays good. It is on somebody's screen, being
/// copied into a browser; five minutes is the whole job.
pub const CODE_TTL: f64 = 300.0;
/// How long a grant survives with nobody using it.
pub const IDLE_TTL: f64 = 3600.0;
/// How long a grant survives at all, however busy it has been.
pub const ABSOLUTE_TTL: f64 = 86_400.0;
/// Codes waiting to be redeemed at once. A person is pairing one site.
pub const MAX_PENDING: usize = 8;
/// Wrong codes before the door stops answering for a while.
pub const MAX_FAILURES: u32 = 5;
/// And how long that is. Four letters is 331,776 codes, which is only out of
/// reach because of this number.
pub const LOCKOUT: f64 = 60.0;
/// `seen` moves on every authenticated request. Persist it at this interval
/// rather than rewriting the file under a stream of them.
const SEEN_FLUSH: f64 = 30.0;

/// One website's standing permission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grant {
    /// Short, public, and safe to print: what `omni web revoke` takes.
    pub id: String,
    /// SHA-256 of the omniauth. The token itself is never written down.
    pub fingerprint: String,
    /// Whatever the site called itself when it paired.
    #[serde(default)]
    pub name: String,
    /// The browser origin this was paired from, empty for a non-browser
    /// holder. A browser cannot lie about this, so a leaked token is still
    /// refused anywhere but the site it was issued to.
    #[serde(default)]
    pub origin: String,
    pub port: u16,
    pub issued: String,
    pub issued_at: f64,
    pub seen_at: f64,
}

impl Grant {
    fn expired(&self, now: f64) -> bool {
        now - self.seen_at > IDLE_TTL || now - self.issued_at > ABSOLUTE_TTL
    }

    /// Seconds left before this grant goes, whichever limit arrives first.
    pub fn expires_in(&self, now: f64) -> f64 {
        let idle = IDLE_TTL - (now - self.seen_at);
        let absolute = ABSOLUTE_TTL - (now - self.issued_at);
        idle.min(absolute).max(0.0)
    }
}

/// A code that has been shown to somebody and not yet used.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    pub fingerprint: String,
    pub port: u16,
    pub issued: String,
    pub issued_at: f64,
}

/// The file, as it sits on disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Memory {
    /// The last port a bridge actually bound. The next one tries here first.
    #[serde(default)]
    pub port: Option<u16>,
    /// Set while a bridge is running, cleared when it exits cleanly.
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub instance: String,
    #[serde(default)]
    pub started: String,
    /// Proof of local file access, and nothing more. Unlike a grant this one
    /// is stored as itself, because the caller has to send it back.
    #[serde(default)]
    pub control: String,
    #[serde(default)]
    pub grants: Vec<Grant>,
    #[serde(default)]
    pub pending: Vec<Pending>,
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub locked_until: f64,
}

/// Where `web.json` lives for this `OMNI_HOME`.
pub fn path() -> PathBuf {
    omni_core::shared::paths::home().join("web.json")
}

pub fn load() -> Memory {
    read(&path())
}

pub fn read(from: &Path) -> Memory {
    std::fs::read_to_string(from)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Write the file whole, or not at all. A half-written `web.json` would lock
/// every paired site out with no way to tell why.
pub fn write(to: &Path, memory: &Memory) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        omni_core::shared::paths::ensure(parent)?;
    }
    let temporary = to.with_extension(format!("json.{}.tmp", std::process::id()));
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?
    };
    file.write_all(serde_json::to_string_pretty(memory)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, to)
}

/// Why a request was turned away. Each of these becomes a different status
/// code, because "no" without a reason is what makes a bridge hard to adopt.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// Nothing was presented.
    Missing,
    /// Presented, but not a grant this bridge issued — or no longer one.
    Unknown,
    /// A real token, used from somewhere other than where it was paired.
    WrongOrigin { expected: String },
    /// The pairing code was wrong, and enough have been.
    LockedOut { seconds: f64 },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Missing => write!(
                f,
                "this needs an omniauth: send `Authorization: Bearer omniauth_…`, or pair first with a code from `omni web connect`"
            ),
            Refusal::Unknown => write!(
                f,
                "that omniauth is not one this bridge is holding; pair again with a code from `omni web connect`"
            ),
            Refusal::WrongOrigin { expected } => write!(
                f,
                "that omniauth was paired to {expected} and will only answer there"
            ),
            Refusal::LockedOut { seconds } => write!(
                f,
                "too many wrong codes; this door is closed for {seconds:.0}s"
            ),
        }
    }
}

/// The live door: `Memory`, a lock around it, and the port it is really on.
pub struct Doorman {
    file: PathBuf,
    port: u16,
    instance: String,
    memory: std::sync::Mutex<Memory>,
    flushed: std::sync::Mutex<f64>,
}

impl Doorman {
    /// Take over the file for a bridge that has just bound `port`.
    ///
    /// `port` is what was actually bound, which is the only thing that decides
    /// whether the grants already in the file survive.
    pub fn open(file: PathBuf, port: u16) -> Self {
        let mut memory = read(&file);
        let kept = memory.port == Some(port);
        if !kept {
            memory.grants.clear();
        }
        // Codes never survive a restart. One is only useful for the minutes
        // it is on somebody's screen, and the person who has it is standing
        // right there and can ask for another.
        memory.pending.clear();
        memory.failures = 0;
        memory.locked_until = 0.0;
        if memory.control.is_empty() || !kept {
            memory.control = keys::control();
        }
        let instance = uuid::Uuid::new_v4().to_string();
        memory.port = Some(port);
        memory.pid = Some(std::process::id());
        memory.instance = instance.clone();
        memory.started = omni_core::shared::clock::now();
        let doorman = Doorman {
            file,
            port,
            instance,
            memory: std::sync::Mutex::new(memory),
            flushed: std::sync::Mutex::new(0.0),
        };
        doorman.flush();
        doorman
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    pub fn control_token(&self) -> String {
        self.with(|memory| memory.control.clone())
    }

    /// How many grants came through the last restart, for the startup line.
    pub fn carried_over(&self) -> usize {
        self.with(|memory| memory.grants.len())
    }

    fn with<T>(&self, act: impl FnOnce(&mut Memory) -> T) -> T {
        let mut memory = self.memory.lock().unwrap_or_else(|p| p.into_inner());
        act(&mut memory)
    }

    fn flush(&self) {
        let snapshot = self.with(|memory| memory.clone());
        if let Err(error) = write(&self.file, &snapshot) {
            crate::say(&format!("cannot write {}: {error}", self.file.display()));
        }
        *self.flushed.lock().unwrap_or_else(|p| p.into_inner()) = omni_core::shared::clock::epoch();
    }

    /// Persist, but not more than once every [`SEEN_FLUSH`] seconds. Used for
    /// the `seen` bump that every authenticated request causes.
    fn flush_lazily(&self) {
        let now = omni_core::shared::clock::epoch();
        {
            let last = self.flushed.lock().unwrap_or_else(|p| p.into_inner());
            if now - *last < SEEN_FLUSH {
                return;
            }
        }
        self.flush();
    }

    /// Drop what has run out. Returns the ids of grants that just went, so
    /// their daemon attachments can be let go with them.
    pub fn sweep(&self) -> Vec<String> {
        let now = omni_core::shared::clock::epoch();
        let gone = self.with(|memory| {
            memory.pending.retain(|code| now - code.issued_at <= CODE_TTL);
            let mut gone = Vec::new();
            memory.grants.retain(|grant| {
                if grant.expired(now) {
                    gone.push(grant.id.clone());
                    return false;
                }
                true
            });
            gone
        });
        if !gone.is_empty() {
            self.flush();
        }
        gone
    }

    /// Mint a code for somebody to carry to a browser.
    pub fn mint_code(&self) -> String {
        let now = omni_core::shared::clock::epoch();
        let code = keys::pairing_code(self.port);
        self.with(|memory| {
            memory.pending.retain(|old| now - old.issued_at <= CODE_TTL);
            while memory.pending.len() >= MAX_PENDING {
                memory.pending.remove(0);
            }
            memory.pending.push(Pending {
                fingerprint: sha256::fingerprint(&code),
                port: self.port,
                issued: omni_core::shared::clock::now(),
                issued_at: now,
            });
        });
        self.flush();
        code
    }

    /// Spend a code and get back an omniauth. The code is gone either way it
    /// goes: used, or counted against the attempt limit.
    pub fn redeem(
        &self,
        code: &str,
        name: &str,
        origin: &str,
    ) -> Result<(String, Grant), Refusal> {
        let now = omni_core::shared::clock::epoch();
        let outcome = self.with(|memory| {
            if memory.locked_until > now {
                return Err(Refusal::LockedOut {
                    seconds: memory.locked_until - now,
                });
            }
            memory.pending.retain(|old| now - old.issued_at <= CODE_TTL);
            let offered = sha256::fingerprint(code);
            let found = memory
                .pending
                .iter()
                .position(|waiting| sha256::same(&waiting.fingerprint, &offered));
            let Some(found) = found else {
                memory.failures += 1;
                if memory.failures >= MAX_FAILURES {
                    /* Somebody is guessing. Everything on offer is withdrawn,
                       not just slowed down: the person who asked for a code is
                       at a terminal and can ask for another one, and a guesser
                       now has nothing live to guess at. */
                    memory.pending.clear();
                    memory.failures = 0;
                    memory.locked_until = now + LOCKOUT;
                    return Err(Refusal::LockedOut { seconds: LOCKOUT });
                }
                return Err(Refusal::Unknown);
            };
            memory.pending.remove(found);
            memory.failures = 0;
            let token = keys::omniauth();
            let grant = Grant {
                id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
                fingerprint: sha256::fingerprint(&token),
                name: name.trim().chars().take(120).collect(),
                origin: origin.to_string(),
                port: self.port,
                issued: omni_core::shared::clock::now(),
                issued_at: now,
                seen_at: now,
            };
            memory.grants.push(grant.clone());
            Ok((token, grant))
        });
        self.flush();
        outcome
    }

    /// Is this token good, from here, right now? A yes also counts as use,
    /// which is what keeps a busy grant from idling out.
    pub fn check(&self, token: Option<&str>, origin: &str) -> Result<Grant, Refusal> {
        let Some(token) = token.filter(|token| !token.is_empty()) else {
            return Err(Refusal::Missing);
        };
        let now = omni_core::shared::clock::epoch();
        let offered = sha256::fingerprint(token);
        let outcome = self.with(|memory| {
            let Some(grant) = memory
                .grants
                .iter_mut()
                .find(|grant| sha256::same(&grant.fingerprint, &offered))
            else {
                return Err(Refusal::Unknown);
            };
            if grant.expired(now) {
                let id = grant.id.clone();
                memory.grants.retain(|grant| grant.id != id);
                return Err(Refusal::Unknown);
            }
            // A grant paired from a browser is pinned to that browser's
            // origin. One paired from curl has no origin to pin it to, and is
            // left alone rather than being made to invent one.
            if !grant.origin.is_empty() && !origin.is_empty() && grant.origin != origin {
                return Err(Refusal::WrongOrigin {
                    expected: grant.origin.clone(),
                });
            }
            if !grant.origin.is_empty() && origin.is_empty() {
                return Err(Refusal::WrongOrigin {
                    expected: grant.origin.clone(),
                });
            }
            grant.seen_at = now;
            Ok(grant.clone())
        });
        if outcome.is_ok() {
            self.flush_lazily();
        }
        outcome
    }

    /// True when the caller could read `web.json`, which is the whole test.
    pub fn is_control(&self, token: Option<&str>) -> bool {
        let Some(token) = token.filter(|token| !token.is_empty()) else {
            return false;
        };
        self.with(|memory| !memory.control.is_empty() && sha256::same(&memory.control, token))
    }

    pub fn grants(&self) -> Vec<Grant> {
        self.with(|memory| memory.grants.clone())
    }

    pub fn pending_codes(&self) -> usize {
        let now = omni_core::shared::clock::epoch();
        self.with(|memory| {
            memory
                .pending
                .iter()
                .filter(|code| now - code.issued_at <= CODE_TTL)
                .count()
        })
    }

    /// Take a grant back. Matches on the short id, or on an origin, so
    /// `omni web revoke https://example.com` does what it looks like.
    pub fn revoke(&self, which: &str) -> Vec<Grant> {
        let taken = self.with(|memory| {
            let mut taken = Vec::new();
            memory.grants.retain(|grant| {
                let matched = grant.id == which
                    || grant.origin == which
                    || grant.name.eq_ignore_ascii_case(which);
                if matched {
                    taken.push(grant.clone());
                }
                !matched
            });
            taken
        });
        if !taken.is_empty() {
            self.flush();
        }
        taken
    }

    pub fn revoke_all(&self) -> Vec<Grant> {
        let taken = self.with(|memory| std::mem::take(&mut memory.grants));
        self.flush();
        taken
    }

    /// Stand down without forgetting anything. The port and the grants stay,
    /// so the next bridge on this port picks up where this one left off.
    pub fn release(&self) {
        self.with(|memory| {
            memory.pid = None;
            memory.instance = String::new();
            memory.pending.clear();
        });
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "omni-web-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path.join("web.json")
    }

    #[test]
    fn a_grant_survives_a_restart_on_the_same_port() {
        let file = scratch("same-port");
        let first = Doorman::open(file.clone(), 1998);
        let code = first.mint_code();
        let (token, _) = first.redeem(&code, "Example", "https://example.com").unwrap();
        first.release();

        let second = Doorman::open(file, 1998);
        assert!(
            second.check(Some(&token), "https://example.com").is_ok(),
            "the same door, so the same keys"
        );
    }

    #[test]
    fn a_grant_does_not_survive_a_move_to_another_port() {
        let file = scratch("moved");
        let first = Doorman::open(file.clone(), 1998);
        let code = first.mint_code();
        let (token, _) = first.redeem(&code, "Example", "https://example.com").unwrap();
        first.release();

        let second = Doorman::open(file, 1997);
        assert_eq!(
            second.check(Some(&token), "https://example.com"),
            Err(Refusal::Unknown),
            "a different address is a different bridge as far as a browser knows"
        );
        assert_eq!(second.grants().len(), 0);
    }

    #[test]
    fn a_code_is_spent_the_first_time_it_is_used() {
        let file = scratch("single-use");
        let doorman = Doorman::open(file, 1998);
        let code = doorman.mint_code();
        assert!(doorman.redeem(&code, "First", "https://first.example").is_ok());
        assert_eq!(
            doorman.redeem(&code, "Second", "https://second.example"),
            Err(Refusal::Unknown),
            "one code, one site"
        );
    }

    #[test]
    fn a_token_only_answers_where_it_was_paired() {
        let file = scratch("origin");
        let doorman = Doorman::open(file, 1998);
        let code = doorman.mint_code();
        let (token, _) = doorman.redeem(&code, "Example", "https://example.com").unwrap();

        assert!(doorman.check(Some(&token), "https://example.com").is_ok());
        assert_eq!(
            doorman.check(Some(&token), "https://evil.example"),
            Err(Refusal::WrongOrigin {
                expected: "https://example.com".into()
            })
        );
        assert_eq!(
            doorman.check(Some(&token), ""),
            Err(Refusal::WrongOrigin {
                expected: "https://example.com".into()
            }),
            "a browser grant does not become a curl grant by dropping the header"
        );
    }

    #[test]
    fn a_token_paired_without_a_browser_is_not_pinned_to_one() {
        let file = scratch("headless");
        let doorman = Doorman::open(file, 1998);
        let code = doorman.mint_code();
        let (token, _) = doorman.redeem(&code, "a script", "").unwrap();
        assert!(doorman.check(Some(&token), "").is_ok());
        assert!(doorman.check(Some(&token), "https://anywhere.example").is_ok());
    }

    #[test]
    fn guessing_closes_the_door_and_withdraws_what_was_on_offer() {
        let file = scratch("guessing");
        let doorman = Doorman::open(file, 1998);
        let real = doorman.mint_code();
        for _ in 0..MAX_FAILURES - 1 {
            assert_eq!(doorman.redeem("ZZZZ-1998", "guess", ""), Err(Refusal::Unknown));
        }
        assert!(matches!(
            doorman.redeem("ZZZZ-1998", "guess", ""),
            Err(Refusal::LockedOut { .. })
        ));
        assert!(
            matches!(doorman.redeem(&real, "late", ""), Err(Refusal::LockedOut { .. })),
            "the real code is withdrawn along with the door"
        );
    }

    #[test]
    fn the_file_never_holds_a_token_that_would_open_it() {
        let file = scratch("fingerprints");
        let doorman = Doorman::open(file.clone(), 1998);
        let code = doorman.mint_code();
        let (token, _) = doorman.redeem(&code, "Example", "https://example.com").unwrap();

        let written = std::fs::read_to_string(&file).unwrap();
        assert!(!written.contains(&token), "the omniauth is on disk in the clear");
        assert!(!written.contains(&code), "the pairing code is on disk in the clear");
        assert!(written.contains(&sha256::fingerprint(&token)));
    }

    #[test]
    fn the_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let file = scratch("permissions");
        let doorman = Doorman::open(file.clone(), 1998);
        doorman.mint_code();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "web.json is {mode:o}");
    }

    #[test]
    fn revoking_takes_a_short_id_or_the_origin_it_was_paired_to() {
        let file = scratch("revoke");
        let doorman = Doorman::open(file, 1998);
        let first = doorman.mint_code();
        let (token, grant) = doorman.redeem(&first, "Example", "https://example.com").unwrap();

        assert_eq!(doorman.revoke("https://example.com").len(), 1);
        assert_eq!(doorman.check(Some(&token), "https://example.com"), Err(Refusal::Unknown));
        assert!(doorman.revoke(&grant.id).is_empty(), "already gone");
    }

    #[test]
    fn nothing_presented_is_a_different_answer_from_something_wrong() {
        let file = scratch("missing");
        let doorman = Doorman::open(file, 1998);
        assert_eq!(doorman.check(None, ""), Err(Refusal::Missing));
        assert_eq!(doorman.check(Some(""), ""), Err(Refusal::Missing));
        assert_eq!(doorman.check(Some("omniauth_nope"), ""), Err(Refusal::Unknown));
    }

    #[test]
    fn the_control_token_is_the_one_secret_kept_as_itself() {
        let file = scratch("control");
        let doorman = Doorman::open(file.clone(), 1998);
        let control = doorman.control_token();
        assert!(doorman.is_control(Some(&control)));
        assert!(!doorman.is_control(Some("omnictl_nope")));
        assert!(!doorman.is_control(None));
        assert!(std::fs::read_to_string(&file).unwrap().contains(&control));
    }
}

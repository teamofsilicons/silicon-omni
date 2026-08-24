//! Driving a CLI's own login, so nobody has to open the CLI.
//!
//! All three do the same dance: print a URL, wait for you to open it, then
//! either finish over a local callback or ask you to paste a code back. omni
//! spawns the login, hands you the URL, and types the code for you when there
//! is one.
//!
//! If the CLI does something else entirely, whatever it printed is handed back
//! verbatim — a confusing message you can read beats a silent failure.
//!
//! One login per provider can be in flight, held here between the `start_auth`
//! that opened it and the `finish_auth` that closes it.

use std::collections::BTreeMap;
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};

use crate::shared::proc::{LineProcess, Spawn};

static IN_FLIGHT: RwLock<Option<BTreeMap<String, Arc<Login>>>> = RwLock::new(None);
static NEXT_LOGIN: AtomicU64 = AtomicU64::new(1);
static LOGIN_EPOCH: AtomicU64 = AtomicU64::new(0);
static REAPER: Once = Once::new();

/// A browser login should never become a forgotten permanent child process.
const LIFETIME: Duration = Duration::from_secs(10 * 60);
const SWEEP: Duration = Duration::from_secs(1);
const STOP_GRACE: Duration = Duration::from_secs(3);

pub struct Login {
    proc: LineProcess,
    token: u64,
    expires: Instant,
}

impl Login {
    /// Begin a login. Returns the URL to open, or whatever went wrong.
    pub fn begin(provider: &str, argv: &[&str], timeout: Duration) -> String {
        Self::begin_with_lifetime(provider, argv, timeout, LIFETIME)
    }

    fn begin_with_lifetime(
        provider: &str,
        argv: &[&str],
        timeout: Duration,
        lifetime: Duration,
    ) -> String {
        let (tx, rx) = mpsc::channel();
        let said: Arc<Mutex<Vec<String>>> = Arc::default();
        let heard = said.clone();
        let capture = move |line: &str| {
            heard
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(line.to_string());
            if let Some(url) = first_url(line) {
                let _ = tx.send(url);
            }
        };
        let watch = capture.clone();
        let epoch = LOGIN_EPOCH.load(Ordering::SeqCst);
        let token = NEXT_LOGIN.fetch_add(1, Ordering::SeqCst);
        let finished_provider = provider.to_string();
        let Ok(proc) = Spawn::new(argv.iter().copied())
            .on_line(capture)
            .on_stderr(watch)
            .on_exit(move |_| {
                // A CLI that finishes through a browser callback needs no
                // later `finish_auth`; do not retain a stale global entry.
                drop(take(&finished_provider, Some(token)));
            })
            .start()
        else {
            return format!("omni could not run {}", argv.join(" "));
        };
        let login = Arc::new(Login {
            proc,
            token,
            expires: Instant::now() + lifetime,
        });
        // Register before waiting for a URL. Shutdown must be able to find a
        // login even while its CLI is still booting or waiting to print one.
        if !track(provider, login.clone(), epoch) {
            return "omni stopped this login during shutdown".to_string();
        }
        let url = rx.recv_timeout(timeout).ok();
        let transcript = said.lock().unwrap_or_else(|p| p.into_inner()).join("\n");
        match url {
            Some(url) => url,
            None => {
                drop(take(provider, Some(token)));
                login.proc.stop(STOP_GRACE);
                format!(
                    "{transcript}\n\nomni could not drive this login. Run it yourself: {}",
                    argv.join(" ")
                )
                .trim()
                .to_string()
            }
        }
    }

    /// Hand the code (or redirect URL) back to the CLI and wait for it to settle.
    pub fn complete(provider: &str, code: &str, timeout: Duration) {
        let login = take(provider, None);
        let Some(login) = login else { return };
        if !code.trim().is_empty() {
            login.proc.send_line(code.trim());
        }
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline && login.proc.alive() {
            std::thread::sleep(Duration::from_millis(250));
        }
        login.proc.stop(STOP_GRACE);
    }
}

/// Replace one provider's login atomically, then retire the superseded child
/// outside the global lock. A process that raced its own registration is
/// removed immediately rather than surviving as a dead map entry.
fn track(provider: &str, login: Arc<Login>, epoch: u64) -> bool {
    let (installed, old) = {
        let mut slot = IN_FLIGHT.write().unwrap_or_else(|p| p.into_inner());
        if LOGIN_EPOCH.load(Ordering::SeqCst) != epoch {
            (false, None)
        } else {
            let old = slot
                .get_or_insert_with(Default::default)
                .insert(provider.to_string(), login.clone());
            (true, old)
        }
    };
    if !installed {
        login.proc.stop(STOP_GRACE);
        return false;
    }
    ensure_reaper();
    if !login.proc.alive() {
        drop(take(provider, Some(login.token)));
    }
    if let Some(old) = old {
        old.proc.stop(STOP_GRACE);
    }
    true
}

/// Remove a provider's current login, optionally only if it is the generation
/// the caller observed. Tokens keep an old process's exit callback or TTL from
/// tearing down its replacement.
fn take(provider: &str, token: Option<u64>) -> Option<Arc<Login>> {
    let mut slot = IN_FLIGHT.write().unwrap_or_else(|p| p.into_inner());
    let removed = {
        let flight = slot.as_mut()?;
        let matches = flight
            .get(provider)
            .is_some_and(|login| token.is_none_or(|token| login.token == token));
        matches.then(|| flight.remove(provider)).flatten()
    };
    if slot.as_ref().is_some_and(BTreeMap::is_empty) {
        *slot = None;
    }
    removed
}

fn ensure_reaper() {
    REAPER.call_once(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(SWEEP);
                reap_expired();
            }
        });
    });
}

fn reap_expired() {
    let now = Instant::now();
    let expired: Vec<Arc<Login>> = {
        let mut slot = IN_FLIGHT.write().unwrap_or_else(|p| p.into_inner());
        let Some(flight) = slot.as_mut() else { return };
        let names: Vec<String> = flight
            .iter()
            .filter(|(_, login)| login.expires <= now || !login.proc.alive())
            .map(|(provider, _)| provider.clone())
            .collect();
        let expired = names
            .iter()
            .filter_map(|provider| flight.remove(provider))
            .collect();
        if flight.is_empty() {
            *slot = None;
        }
        expired
    };
    for login in expired {
        login.proc.stop(STOP_GRACE);
    }
}

/// Stop every browser-login child during daemon shutdown.
pub fn shutdown() {
    // A begin which already captured the old epoch must not install a child
    // after this drain. Increment first; track checks it while holding the map
    // lock, so either this drain sees the child or track rejects it.
    LOGIN_EPOCH.fetch_add(1, Ordering::SeqCst);
    let flight = std::mem::take(
        &mut *IN_FLIGHT
            .write()
            .unwrap_or_else(|poison| poison.into_inner()),
    );
    for (_, login) in flight.into_iter().flatten() {
        login.proc.stop(STOP_GRACE);
    }
}

/// The first thing in a line that looks like somewhere to go.
pub fn first_url(line: &str) -> Option<String> {
    let at = line.find("http://").or_else(|| line.find("https://"))?;
    let url: String = line[at..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !"\"'<>()]".contains(*c))
        .collect();
    (url.len() > 8).then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    static LOGIN_TEST: Mutex<()> = Mutex::new(());

    fn current(provider: &str) -> Option<Arc<Login>> {
        IN_FLIGHT
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()?
            .get(provider)
            .cloned()
    }

    fn login(provider: &str, lifetime: Duration) -> (String, Arc<Login>) {
        let url = Login::begin_with_lifetime(
            provider,
            &[
                "sh",
                "-c",
                "printf '%s\\n' 'https://example.test/login'; cat >/dev/null",
            ],
            Duration::from_secs(2),
            lifetime,
        );
        let current = current(provider).expect("the login stays in flight");
        (url, current)
    }

    #[test]
    fn the_url_is_picked_out_of_whatever_was_printed() {
        assert_eq!(
            first_url("Open this: https://claude.ai/oauth?code=1 in a browser"),
            Some("https://claude.ai/oauth?code=1".into())
        );
        assert_eq!(
            first_url("(http://localhost:1455/cb)"),
            Some("http://localhost:1455/cb".into())
        );
    }

    #[test]
    fn a_line_with_nowhere_to_go_has_no_url() {
        assert_eq!(first_url("Logging you in..."), None);
        assert_eq!(first_url("see https://"), None);
    }

    #[test]
    fn replacing_and_shutting_down_logins_reaps_their_processes() {
        let _turn = LOGIN_TEST.lock().unwrap();
        shutdown();
        let (url, first) = login("replace-test", Duration::from_secs(60));
        assert_eq!(url, "https://example.test/login");
        assert!(first.proc.alive());

        let (_, second) = login("replace-test", Duration::from_secs(60));
        assert!(!first.proc.alive(), "replacement stopped the older child");
        assert_ne!(first.token, second.token);
        assert!(second.proc.alive());

        shutdown();
        assert!(
            !second.proc.alive(),
            "daemon shutdown reaped the replacement"
        );
        assert!(current("replace-test").is_none());
    }

    #[test]
    fn expired_and_naturally_finished_logins_leave_no_stale_entry() {
        let _turn = LOGIN_TEST.lock().unwrap();
        shutdown();
        let (_, expired) = login("expiry-test", Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        reap_expired();
        assert!(!expired.proc.alive());
        assert!(current("expiry-test").is_none());

        let answer = Login::begin_with_lifetime(
            "exit-test",
            &["sh", "-c", "printf '%s\\n' 'https://example.test/done'"],
            Duration::from_secs(2),
            Duration::from_secs(60),
        );
        assert_eq!(answer, "https://example.test/done");
        let deadline = Instant::now() + Duration::from_secs(2);
        while current("exit-test").is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            current("exit-test").is_none(),
            "the on-exit callback removed the naturally completed login"
        );
        shutdown();
    }

    #[test]
    fn shutdown_reaps_a_login_that_is_still_waiting_for_its_url() {
        let _turn = LOGIN_TEST.lock().unwrap();
        shutdown();
        let starting = std::thread::spawn(|| {
            Login::begin_with_lifetime(
                "starting-test",
                &["sh", "-c", "cat >/dev/null"],
                Duration::from_secs(5),
                Duration::from_secs(60),
            )
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let login = loop {
            if let Some(login) = current("starting-test") {
                break login;
            }
            assert!(
                Instant::now() < deadline,
                "the pre-URL process was never registered"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        shutdown();
        assert!(!login.proc.alive());
        assert!(current("starting-test").is_none());
        assert!(
            starting
                .join()
                .unwrap()
                .contains("omni could not drive this login")
        );
    }

    #[test]
    fn a_pre_shutdown_spawn_cannot_register_after_the_drain() {
        let _turn = LOGIN_TEST.lock().unwrap();
        shutdown();
        let epoch = LOGIN_EPOCH.load(Ordering::SeqCst);
        let login = Arc::new(Login {
            proc: Spawn::new(["sh", "-c", "cat >/dev/null"]).start().unwrap(),
            token: NEXT_LOGIN.fetch_add(1, Ordering::SeqCst),
            expires: Instant::now() + Duration::from_secs(60),
        });
        assert!(login.proc.alive());

        shutdown();
        assert!(!track("stale-epoch-test", login.clone(), epoch));
        assert!(!login.proc.alive());
        assert!(current("stale-epoch-test").is_none());
    }
}

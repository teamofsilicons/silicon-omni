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
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};

use crate::shared::proc::{LineProcess, Spawn};

static IN_FLIGHT: RwLock<Option<BTreeMap<String, Arc<Login>>>> = RwLock::new(None);

pub struct Login {
    proc: LineProcess,
}

impl Login {
    /// Begin a login. Returns the URL to open, or whatever went wrong.
    pub fn begin(provider: &str, argv: &[&str], timeout: Duration) -> String {
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
        let Ok(proc) = Spawn::new(argv.iter().copied())
            .on_line(capture)
            .on_stderr(watch)
            .start()
        else {
            return format!("omni could not run {}", argv.join(" "));
        };
        let url = rx.recv_timeout(timeout).ok();
        let transcript = said.lock().unwrap_or_else(|p| p.into_inner()).join("\n");
        let login = Arc::new(Login { proc });
        match url {
            Some(url) => {
                IN_FLIGHT
                    .write()
                    .unwrap_or_else(|p| p.into_inner())
                    .get_or_insert_with(Default::default)
                    .insert(provider.to_string(), login);
                url
            }
            None => {
                login.proc.stop(Duration::from_secs(3));
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
        let login = IN_FLIGHT
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
            .and_then(|flight| flight.remove(provider));
        let Some(login) = login else { return };
        if !code.trim().is_empty() {
            login.proc.send_line(code.trim());
        }
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline && login.proc.alive() {
            std::thread::sleep(Duration::from_millis(250));
        }
        login.proc.stop(Duration::from_secs(3));
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
}

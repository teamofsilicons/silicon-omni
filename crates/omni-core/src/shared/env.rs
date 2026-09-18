//! The environment a terminal would have, so a CLI you can run, omni can run.
//!
//! The daemon inherits its environment from whatever started it first. From a
//! terminal that is fine. From a GUI, a launchd job, an IDE's run button or a
//! cron entry it is `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, nothing has read
//! `.zshrc`, and `claude` in `~/.local/bin` does not exist as far as the daemon
//! is concerned — and because the daemon outlives the shell that started it,
//! it never will.
//!
//! So the daemon asks the user's login shell what it would have had. The shell
//! is run once, as a login shell and an interactive one both, so every profile
//! a terminal would read is read, and whatever it exports is kept here. `PATH`
//! is merged with the shell's entries first. Every other variable the daemon
//! was not started with is taken from the shell; nothing it *was* started with
//! is overridden. Every child the daemon starts is given the result.
//!
//! Some profiles hang when the shell is login and interactive at once but has
//! no terminal — a `pyenv rehash` under job control will. After one such
//! failure the shell is asked in two steps instead, for the rest of the
//! daemon's life: as a login shell first, then as an interactive one on top of
//! that answer, which is the same files in the same order.
//!
//! The answer is refreshed on a miss, so a CLI installed after the daemon came
//! up is found the next time anybody asks for it — and never more than once a
//! minute, so a CLI that is genuinely missing does not cost a shell per call.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use super::proc;

/// Set while the shell is being asked, so a profile that wants to can stay quiet.
pub const RESOLVING: &str = "OMNI_RESOLVING_ENVIRONMENT";
/// Never ask the shell more often than this.
const COOLDOWN: Duration = Duration::from_secs(60);
/// A profile that hangs is a profile that is skipped.
const PATIENCE: Duration = Duration::from_secs(10);
const BEGIN: &str = "-----omni-env-begin-----";
const END: &str = "-----omni-env-end-----";
/// Facts about the probing shell process rather than about the user's setup.
const NOT_THEIRS: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_", RESOLVING];
const FALLBACK_SHELLS: &[&str] = &["/bin/zsh", "/bin/bash", "/bin/sh"];

struct Answer {
    vars: BTreeMap<String, String>,
    at: Instant,
}

static ANSWER: RwLock<Option<Answer>> = RwLock::new(None);
/// One probe at a time; everyone else waits for it and then finds it fresh.
static ASKING: Mutex<()> = Mutex::new(());
/// Login-and-interactive at once did not answer; ask in two steps from now on.
static IN_STEPS: AtomicBool = AtomicBool::new(false);

/// The shell to ask: the one the daemon was started under, else the account's.
pub fn shell() -> PathBuf {
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .or_else(|| passwd_shell().filter(|path| path.is_file()))
        .or_else(|| {
            FALLBACK_SHELLS
                .iter()
                .map(PathBuf::from)
                .find(|path| path.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

/// What the account says its shell is, for a daemon started without `SHELL`.
fn passwd_shell() -> Option<PathBuf> {
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buffer = vec![0_u8; 4096];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer refers to storage owned by this frame, and the
    // buffer length passed is its real length.
    let rc = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if rc != 0 || found.is_null() || entry.pw_shell.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so pw_shell points into `buffer`.
    let shell = unsafe { std::ffi::CStr::from_ptr(entry.pw_shell) };
    let shell = shell.to_str().ok()?;
    (!shell.is_empty()).then(|| PathBuf::from(shell))
}

/// Ask the shell again, unless it was asked within the last minute.
///
/// Returns whether there is an answer to use — this time's or the last.
pub fn refresh() -> bool {
    let _turn = ASKING.lock().unwrap_or_else(|p| p.into_inner());
    if !fresh() {
        let vars = ask(&shell()).unwrap_or_default();
        *ANSWER.write().unwrap_or_else(|p| p.into_inner()) = Some(Answer {
            vars,
            at: Instant::now(),
        });
    }
    answered()
}

fn fresh() -> bool {
    ANSWER
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .is_some_and(|answer| answer.at.elapsed() < COOLDOWN)
}

fn answered() -> bool {
    ANSWER
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .is_some_and(|answer| !answer.vars.is_empty())
}

fn learned() -> BTreeMap<String, String> {
    ANSWER
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|answer| answer.vars.clone())
        .unwrap_or_default()
}

/// Was the shell asked in two steps rather than as one terminal would be?
pub fn stepped() -> bool {
    IN_STEPS.load(Ordering::SeqCst)
}

/// Run a shell the way a terminal does and read back what it exports.
fn ask(shell: &Path) -> Option<BTreeMap<String, String>> {
    if !stepped() {
        if let Some(vars) = run(shell, &["-l", "-i", "-c"], None) {
            return Some(vars);
        }
        IN_STEPS.store(true, Ordering::SeqCst);
    }
    // The login shell's answer becomes the interactive shell's starting point,
    // so a profile that puts a tool on PATH for the rc file to call still does.
    let login = run(shell, &["-l", "-c"], None);
    run(shell, &["-i", "-c"], login.as_ref()).or(login)
}

fn run(
    shell: &Path,
    flags: &[&str],
    base: Option<&BTreeMap<String, String>>,
) -> Option<BTreeMap<String, String>> {
    let script = format!("printf '%s' '{BEGIN}'; env -0; printf '%s' '{END}'");
    let mut command = proc::command(&shell.to_string_lossy());
    if let Some(base) = base {
        command.envs(base);
    }
    command.args(flags).arg(&script).env(RESOLVING, "1");
    let (_, out) = proc::run(command, PATIENCE)?;
    parse(&out)
}

/// The variables between the markers, whatever the profiles printed around them.
fn parse(out: &str) -> Option<BTreeMap<String, String>> {
    let start = out.find(BEGIN)? + BEGIN.len();
    let stop = out.rfind(END)?;
    let vars: BTreeMap<String, String> = out
        .get(start..stop)?
        .split('\0')
        .filter_map(|record| record.split_once('='))
        .filter(|(key, _)| {
            !key.is_empty() && !NOT_THEIRS.contains(key) && !key.starts_with("OMNI_")
        })
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    (!vars.is_empty()).then_some(vars)
}

/// `PATH` as a terminal would have it: the shell's entries first, then any the
/// daemon was started with that the shell did not mention.
pub fn path() -> String {
    let own = std::env::var("PATH").unwrap_or_default();
    match learned().get("PATH") {
        Some(theirs) => merged(theirs, &own),
        None => own,
    }
}

fn merged(first: &str, then: &str) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for dir in first.split(':').chain(then.split(':')) {
        if !dir.is_empty() && !seen.contains(&dir) {
            seen.push(dir);
        }
    }
    seen.join(":")
}

/// What a child is started with: the daemon's own environment, filled in
/// from the shell's wherever the daemon had nothing to say.
pub fn vars() -> Vec<(String, String)> {
    let mut all: BTreeMap<String, String> = std::env::vars().collect();
    for (key, value) in learned() {
        if key != "PATH" {
            all.entry(key).or_insert(value);
        }
    }
    all.insert("PATH".into(), path());
    all.into_iter().collect()
}

/// The first match on `PATH`, the way a shell would find it.
///
/// A miss asks the shell again before giving up: a CLI installed after the
/// daemon came up should be found the moment somebody asks for it.
pub fn which(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let path = PathBuf::from(program);
        return path.is_file().then_some(path);
    }
    find(program, &path()).or_else(|| {
        if fresh() {
            return None;
        }
        refresh();
        find(program, &path())
    })
}

fn find(program: &str, path: &str) -> Option<PathBuf> {
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_shell_exports_is_read_between_the_markers() {
        let chatter = format!(
            "Welcome!\n{BEGIN}PATH=/a:/b\0HOME=/h\0MULTI=one\ntwo\0OMNI_HOME=/x\0PWD=/here\0=odd\0{END}bye"
        );
        let vars = parse(&chatter).unwrap();
        assert_eq!(vars["PATH"], "/a:/b");
        assert_eq!(vars["MULTI"], "one\ntwo", "a value may span lines");
        assert!(!vars.contains_key("OMNI_HOME"), "omni's own state is not theirs");
        assert!(!vars.contains_key("PWD"), "the probe's directory is not theirs");
        assert!(!vars.contains_key(""));
        assert!(parse("nothing to see").is_none());
        assert!(parse(&format!("{END}{BEGIN}")).is_none(), "markers out of order");
    }

    #[test]
    fn the_shells_path_comes_first_and_nothing_is_lost() {
        assert_eq!(merged("/a:/b", "/b:/c"), "/a:/b:/c");
        assert_eq!(merged("", "/c::/d"), "/c:/d");
    }

    #[test]
    fn which_finds_what_a_shell_would() {
        assert!(which("sh").is_some());
        assert!(which("/bin/sh").is_some());
        assert!(which("definitely-not-a-program-99").is_none());
        assert!(find("sh", "").is_none());
    }

    #[test]
    fn a_real_shell_answers_with_a_path() {
        // Whatever shell this machine has, it can say what it exports.
        let vars = ask(&shell()).expect("the login shell described itself");
        assert!(vars.contains_key("PATH"));
        assert!(!vars.contains_key(RESOLVING));
    }

    #[test]
    fn asking_in_two_steps_answers_too_and_the_second_step_sees_the_first() {
        let shell = shell();
        let login = run(&shell, &["-l", "-c"], None).expect("login step");
        let mut seeded = login.clone();
        seeded.insert("OMNI_STEP_ONE".into(), "seen".into());
        let interactive = run(&shell, &["-i", "-c"], Some(&seeded)).expect("interactive step");
        assert!(interactive.contains_key("PATH"));
        assert!(
            !interactive.contains_key("OMNI_STEP_ONE"),
            "omni's own variables are never taken from the shell"
        );
        let mut seeded = login;
        seeded.insert("STEP_ONE".into(), "seen".into());
        let interactive = run(&shell, &["-i", "-c"], Some(&seeded)).unwrap();
        assert_eq!(interactive.get("STEP_ONE").map(String::as_str), Some("seen"));
    }

    #[test]
    fn a_child_is_given_the_daemons_variables_plus_the_shells() {
        let given: BTreeMap<String, String> = vars().into_iter().collect();
        for (key, value) in std::env::vars() {
            if key != "PATH" {
                assert_eq!(given.get(&key), Some(&value), "{key} was overridden");
            }
        }
        let own = std::env::var("PATH").unwrap_or_default();
        for dir in own.split(':').filter(|dir| !dir.is_empty()) {
            assert!(
                given["PATH"].split(':').any(|kept| kept == dir),
                "{dir} fell off PATH"
            );
        }
    }
}

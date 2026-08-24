//! Spawn a CLI, read its stdout a line at a time, write lines to its stdin.
//!
//! Every provider adapter talks to its CLI through this. What the lines *mean*
//! differs per provider; how they move does not.
//!
//! Each child is put in its own process group. Provider CLIs start helpers of
//! their own — agy boots a language server — and a group is the only way to be
//! sure that killing the CLI kills what it started, rather than leaving a
//! daemon's worth of orphans behind after a long-running session ends.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub type OnLine = Box<dyn Fn(&str) + Send + Sync>;
pub type OnExit = Box<dyn Fn(i32) + Send + Sync>;

/// How a child is to be started, and who hears what it says.
pub struct Spawn {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Variables to take away from the child, for CLIs that read the ambient one.
    pub unset: Vec<String>,
    pub cwd: Option<String>,
    pub on_line: Option<OnLine>,
    pub on_stderr: Option<OnLine>,
    pub on_exit: Option<OnExit>,
}

impl Spawn {
    pub fn new<S: Into<String>>(argv: impl IntoIterator<Item = S>) -> Self {
        Spawn {
            argv: argv.into_iter().map(Into::into).collect(),
            env: Vec::new(),
            unset: Vec::new(),
            cwd: None,
            on_line: None,
            on_stderr: None,
            on_exit: None,
        }
    }

    pub fn env(mut self, key: &str, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn on_line(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.on_line = Some(Box::new(f));
        self
    }

    pub fn on_stderr(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.on_stderr = Some(Box::new(f));
        self
    }

    pub fn on_exit(mut self, f: impl Fn(i32) + Send + Sync + 'static) -> Self {
        self.on_exit = Some(Box::new(f));
        self
    }

    pub fn start(self) -> std::io::Result<LineProcess> {
        LineProcess::start(self)
    }
}

pub struct LineProcess {
    stdin: Mutex<Option<ChildStdin>>,
    pgid: i32,
    running: Arc<AtomicBool>,
    pumps: Mutex<Vec<JoinHandle<()>>>,
}

impl LineProcess {
    fn start(spec: Spawn) -> std::io::Result<Self> {
        let (program, rest) = spec
            .argv
            .split_first()
            .ok_or_else(|| std::io::Error::other("nothing to run"))?;
        let mut command = Command::new(program);
        command
            .args(rest)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        for key in &spec.unset {
            command.env_remove(key);
        }
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        // Its own group, so stopping it stops whatever it started.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let pgid = child.id() as i32;
        let child = Arc::new(Mutex::new(child));
        let running = Arc::new(AtomicBool::new(true));

        let mut pumps = Vec::new();
        if let (Some(stdout), Some(on_line)) = (stdout, spec.on_line) {
            pumps.push(pump(stdout, on_line));
        }
        if let (Some(stderr), Some(on_stderr)) = (stderr, spec.on_stderr) {
            pumps.push(pump(stderr, on_stderr));
        }
        pumps.push(reaper(child, running.clone(), spec.on_exit));

        Ok(LineProcess {
            stdin: Mutex::new(stdin),
            pgid,
            running,
            pumps: Mutex::new(pumps),
        })
    }

    pub fn alive(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Write one line to stdin. `false` means the pipe is gone.
    pub fn send_line(&self, text: &str) -> bool {
        let mut slot = self.stdin.lock().unwrap_or_else(|p| p.into_inner());
        let Some(stdin) = slot.as_mut() else {
            return false;
        };
        if stdin.write_all(text.as_bytes()).is_err()
            || stdin.write_all(b"\n").is_err()
            || stdin.flush().is_err()
        {
            *slot = None; // a broken pipe never comes back
            return false;
        }
        true
    }

    pub fn close_stdin(&self) {
        *self.stdin.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// EOF first (most CLIs exit cleanly on it), then the group, then kill.
    ///
    /// The reader threads are joined before returning, so a caller can be sure
    /// nothing else will arrive through `on_line` once this comes back.
    pub fn stop(&self, grace: Duration) {
        self.close_stdin();
        if !self.settled(grace) {
            self.signal(libc::SIGTERM);
            if !self.settled(grace) {
                self.signal(libc::SIGKILL);
                self.settled(grace);
            }
        }
        for pump in std::mem::take(&mut *self.pumps.lock().unwrap_or_else(|p| p.into_inner())) {
            let _ = pump.join();
        }
    }

    fn settled(&self, grace: Duration) -> bool {
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if !self.alive() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        !self.alive()
    }

    fn signal(&self, signal: i32) {
        unsafe { libc::kill(-self.pgid, signal) };
    }
}

impl Drop for LineProcess {
    /// A runner that is dropped without a stop must not leave a CLI running.
    fn drop(&mut self) {
        if self.alive() {
            self.close_stdin();
            self.signal(libc::SIGKILL);
        }
    }
}

fn pump(source: impl std::io::Read + Send + 'static, on_line: OnLine) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(source).lines().map_while(Result::ok) {
            let line = line.trim_end_matches(['\r', '\n']);
            if !line.is_empty() {
                on_line(line);
            }
        }
    })
}

/// One thread owns the `wait`, so `alive` is a flag rather than a race.
fn reaper(
    child: Arc<Mutex<Child>>,
    running: Arc<AtomicBool>,
    on_exit: Option<OnExit>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let status = loop {
            let polled = child
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .try_wait()
                .unwrap_or(None);
            if let Some(status) = polled {
                break status;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        running.store(false, Ordering::SeqCst);
        if let Some(on_exit) = on_exit {
            on_exit(status.code().unwrap_or(-1));
        }
    })
}

/// Run something to completion and collect what it said. For probes, not turns.
pub fn output(argv: &[&str], timeout: Duration) -> Option<(i32, String)> {
    run(Command::new(argv[0]).args(&argv[1..]), timeout)
}

pub fn run(command: &mut Command, timeout: Duration) -> Option<(i32, String)> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let stderr = child.stderr.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let mut err = String::new();
        let _ = std::io::Read::read_to_string(&mut { stdout }, &mut out);
        let _ = std::io::Read::read_to_string(&mut { stderr }, &mut err);
        (out, err)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let (out, err) = reader.join().ok()?;
    let text = if out.trim().is_empty() { err } else { out };
    Some((status.code().unwrap_or(-1), text.trim().to_string()))
}

/// Is a process still there? A pid that is gone is a dead owner.
pub fn alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    match unsafe { libc::kill(pid, 0) } {
        0 => true,
        _ => std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn lines_out_and_lines_in() {
        let (tx, rx) = mpsc::channel();
        let proc = Spawn::new(["cat"])
            .on_line(move |line| tx.send(line.to_string()).unwrap())
            .start()
            .unwrap();
        proc.send_line("hello");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "hello");
        proc.stop(Duration::from_secs(2));
        assert!(!proc.alive());
    }

    #[test]
    fn a_closed_pipe_is_reported_not_raised() {
        let proc = Spawn::new(["true"]).start().unwrap();
        proc.stop(Duration::from_secs(2));
        assert!(!proc.send_line("nobody is listening"));
    }

    #[test]
    fn an_exit_reaches_the_listener() {
        let (tx, rx) = mpsc::channel();
        let proc = Spawn::new(["sh", "-c", "exit 3"])
            .on_exit(move |code| tx.send(code).unwrap())
            .start()
            .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 3);
        proc.stop(Duration::from_secs(2));
    }

    #[test]
    fn a_cli_that_ignores_eof_is_still_stopped() {
        let proc = Spawn::new(["sh", "-c", "trap '' TERM; while :; do sleep 0.1; done"])
            .start()
            .unwrap();
        proc.stop(Duration::from_millis(300));
        assert!(!proc.alive(), "SIGKILL is the backstop");
    }

    #[test]
    fn stopping_takes_the_children_with_it() {
        let (tx, rx) = mpsc::channel();
        let proc = Spawn::new(["sh", "-c", "sleep 30 & echo $!; wait"])
            .on_line(move |line| {
                let _ = tx.send(line.to_string());
            })
            .start()
            .unwrap();
        let grandchild: i32 = rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .parse()
            .unwrap();
        assert!(alive(grandchild));
        proc.stop(Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            !alive(grandchild),
            "the whole group goes, not just the shell"
        );
    }

    #[test]
    fn a_probe_gives_back_what_it_said() {
        let (code, text) = output(&["echo", "hi"], Duration::from_secs(5)).unwrap();
        assert_eq!((code, text.as_str()), (0, "hi"));
    }

    #[test]
    fn a_probe_that_hangs_gives_up_rather_than_wedging() {
        assert!(output(&["sleep", "30"], Duration::from_millis(200)).is_none());
    }
}

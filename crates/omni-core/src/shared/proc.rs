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
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub type OnLine = Box<dyn Fn(&str) + Send + Sync>;
pub type OnExit = Box<dyn Fn(i32) + Send + Sync>;

// macOS has no atomic `pipe2(O_CLOEXEC)`. Serializing our process creation
// closes its pipe/fcntl inheritance window before another provider can fork.
static SPAWN_LOCK: Mutex<()> = Mutex::new(());

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
    callbacks: Arc<CallbackGate>,
    guardian: Mutex<Option<OwnedFd>>,
    pumps: Mutex<Vec<JoinHandle<()>>>,
}

struct CallbackState {
    accepting: bool,
    active: usize,
}

/// Closing this gate is the hand-off between process teardown and user code.
/// Readers which remain blocked in an inherited pipe may outlive the process
/// object, but they cannot enter a callback after `close` returns.
struct CallbackGate {
    state: Mutex<CallbackState>,
    idle: Condvar,
}

impl CallbackGate {
    fn new() -> Self {
        Self {
            state: Mutex::new(CallbackState {
                accepting: true,
                active: 0,
            }),
            idle: Condvar::new(),
        }
    }

    fn call(&self, callback: impl FnOnce()) {
        {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if !state.accepting {
                return;
            }
            state.active += 1;
        }

        struct Finished<'a>(&'a CallbackGate);
        impl Drop for Finished<'_> {
            fn drop(&mut self) {
                let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
                state.active -= 1;
                if state.active == 0 {
                    self.0.idle.notify_all();
                }
            }
        }

        let _finished = Finished(self);
        callback();
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.accepting = false;
        while state.active > 0 {
            state = self.idle.wait(state).unwrap_or_else(|p| p.into_inner());
        }
    }

    fn disable(&self) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .accepting = false;
    }
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
        let (mut child, guardian) = guarded_spawn(command)?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let pgid = child.id() as i32;
        let child = Arc::new(Mutex::new(child));
        let running = Arc::new(AtomicBool::new(true));
        let callbacks = Arc::new(CallbackGate::new());

        let mut pumps = Vec::new();
        if let (Some(stdout), Some(on_line)) = (stdout, spec.on_line) {
            pumps.push(pump(stdout, on_line, callbacks.clone()));
        }
        if let (Some(stderr), Some(on_stderr)) = (stderr, spec.on_stderr) {
            pumps.push(pump(stderr, on_stderr, callbacks.clone()));
        }
        pumps.push(reaper(
            child,
            running.clone(),
            spec.on_exit,
            callbacks.clone(),
        ));

        Ok(LineProcess {
            stdin: Mutex::new(stdin),
            pgid,
            running,
            callbacks,
            guardian: Mutex::new(Some(guardian)),
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
    /// A descendant can deliberately leave the group while retaining one of
    /// its pipes. Such a pipe must not turn `stop` into an unbounded join, so
    /// unfinished readers are detached behind a closed callback gate.
    pub fn stop(&self, grace: Duration) {
        self.close_stdin();
        if !self.settled(grace) {
            self.signal(libc::SIGTERM);
            if !self.settled(grace) {
                self.signal(libc::SIGKILL);
                self.settled(grace);
            }
        }
        // Closing the owner end also asks the out-of-group guardian to kill
        // any group members which survived the ordinary signal sequence.
        self.guardian
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        self.callbacks.close();
        for pump in std::mem::take(&mut *self.pumps.lock().unwrap_or_else(|p| p.into_inner())) {
            if pump.is_finished() {
                let _ = pump.join();
            }
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
        self.callbacks.disable();
        self.close_stdin();
        if self.alive() {
            self.signal(libc::SIGKILL);
        }
        self.guardian
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .take();
    }
}

fn pump(
    source: impl std::io::Read + Send + 'static,
    on_line: OnLine,
    callbacks: Arc<CallbackGate>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(source).lines().map_while(Result::ok) {
            let line = line.trim_end_matches(['\r', '\n']);
            if !line.is_empty() {
                callbacks.call(|| on_line(line));
            }
        }
    })
}

/// One thread owns the `wait`, so `alive` is a flag rather than a race.
fn reaper(
    child: Arc<Mutex<Child>>,
    running: Arc<AtomicBool>,
    on_exit: Option<OnExit>,
    callbacks: Arc<CallbackGate>,
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
            callbacks.call(|| on_exit(status.code().unwrap_or(-1)));
        }
    })
}

/// Spawn a process group with a sibling guardian which outlives the daemon.
///
/// The daemon owns the write end of a close-on-exec pipe. The guardian is the
/// only reader after spawn and is moved into a separate session. If the daemon
/// exits for any reason, EOF wakes the guardian and it kills the provider's
/// original process group. This is deliberately pipe-based rather than using
/// Linux's `PR_SET_PDEATHSIG`, so the same mechanism works on macOS.
fn guarded_spawn(mut command: Command) -> std::io::Result<(Child, OwnedFd)> {
    let _spawn = SPAWN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (death_read, death_write) = death_pipe()?;
    let read_fd = death_read.as_raw_fd();
    let write_fd = death_write.as_raw_fd();
    let max_fd = open_fd_limit();

    // Only async-signal-safe libc operations run after Command's fork. In
    // particular, the guardian never allocates, locks, logs, or invokes Rust
    // destructors before `_exit`.
    unsafe {
        command.pre_exec(move || guardian_pre_exec(read_fd, write_fd, max_fd));
    }
    let child = command.spawn()?;
    drop(death_read);
    Ok((child, death_write))
}

fn death_pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    #[cfg(target_os = "linux")]
    let made = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    #[cfg(not(target_os = "linux"))]
    let made = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if made == -1 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: a successful pipe call returns two newly-owned descriptors.
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    #[cfg(not(target_os = "linux"))]
    for fd in [read.as_raw_fd(), write.as_raw_fd()] {
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok((read, write))
}

fn open_fd_limit() -> RawFd {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0
        && limit.rlim_cur != libc::RLIM_INFINITY
    {
        limit.rlim_cur.min(RawFd::MAX as _) as RawFd
    } else {
        65_536
    }
}

/// Runs in Command's post-fork child, before the provider is exec'd.
unsafe fn guardian_pre_exec(
    death_read: RawFd,
    death_write: RawFd,
    max_fd: RawFd,
) -> std::io::Result<()> {
    // SAFETY: every operation in this routine is async-signal-safe.
    if unsafe { libc::setsid() } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let target_pgid = unsafe { libc::getpid() };
    unsafe { libc::close(death_write) };
    let guardian = unsafe { libc::fork() };
    if guardian == -1 {
        unsafe { libc::close(death_read) };
        return Err(std::io::Error::last_os_error());
    }
    if guardian == 0 {
        unsafe { guardian_main(death_read, target_pgid, max_fd) };
    }
    unsafe { libc::close(death_read) };
    Ok(())
}

/// The guardian child. This function never returns and must remain strictly
/// within the POSIX async-signal-safe subset because it was created by `fork`
/// in a multi-threaded process.
unsafe fn guardian_main(death_read: RawFd, target_pgid: i32, max_fd: RawFd) -> ! {
    if unsafe { libc::setsid() } == -1 {
        unsafe {
            libc::kill(-target_pgid, libc::SIGKILL);
            libc::_exit(126);
        }
    }
    const DEATH_FD: RawFd = 3;
    if death_read != DEATH_FD && unsafe { libc::dup2(death_read, DEATH_FD) } == -1 {
        unsafe {
            libc::kill(-target_pgid, libc::SIGKILL);
            libc::_exit(126);
        }
    }
    unsafe {
        libc::close(libc::STDIN_FILENO);
        libc::close(libc::STDOUT_FILENO);
        libc::close(libc::STDERR_FILENO);
    }

    #[cfg(target_os = "linux")]
    let closed = unsafe { libc::syscall(libc::SYS_close_range, 4_u32, u32::MAX, 0_u32) } == 0;
    #[cfg(not(target_os = "linux"))]
    let closed = false;
    if !closed {
        for fd in 4..max_fd {
            unsafe { libc::close(fd) };
        }
    }

    let mut watched = libc::pollfd {
        fd: DEATH_FD,
        events: libc::POLLIN | libc::POLLHUP,
        revents: 0,
    };
    loop {
        let ready = unsafe { libc::poll(&mut watched, 1, 250) };
        if ready > 0 {
            let mut byte = 0_u8;
            // The daemon never writes data. EOF, an invalid descriptor, or
            // unexpected data all fail closed and terminate the group.
            let _ = unsafe { libc::read(DEATH_FD, (&mut byte as *mut u8).cast(), 1) };
            unsafe {
                libc::kill(-target_pgid, libc::SIGKILL);
                libc::_exit(0);
            }
        }
        // No target group means the guardian has finished its ordinary job.
        if unsafe { libc::kill(-target_pgid, 0) } == -1 {
            unsafe { libc::_exit(0) };
        }
    }
}

/// Run something to completion and collect what it said. For probes, not turns.
pub fn output(argv: &[&str], timeout: Duration) -> Option<(i32, String)> {
    let mut command = Command::new(argv[0]);
    command.args(&argv[1..]);
    run(command, timeout)
}

pub fn run(mut command: Command, timeout: Duration) -> Option<(i32, String)> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (mut child, _guardian) = guarded_spawn(command).ok()?;
    let pgid = child.id() as i32;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        stop_probe(&mut child, pgid);
        return None;
    };
    // Drain both pipes concurrently. Reading stdout to EOF before touching
    // stderr deadlocks as soon as a noisy CLI fills its stderr pipe while it
    // is still holding stdout open.
    let stdout_reader = drain(stdout);
    let stderr_reader = drain(stderr);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                stop_probe(&mut child, pgid);
                return None;
            }
        }
    };
    // Readers are channels rather than joined threads so even a descendant
    // which deliberately escaped the process group cannot wedge the caller by
    // retaining an inherited pipe forever. The ordinary case still drains all
    // output before the probe's deadline.
    let out = recv_before(&stdout_reader, deadline);
    let err = recv_before(&stderr_reader, deadline);
    let (Some(out), Some(err)) = (out, err) else {
        signal_group(pgid, libc::SIGKILL);
        return None;
    };
    let text = if out.trim().is_empty() { err } else { out };
    Some((status.code().unwrap_or(-1), text.trim().to_string()))
}

fn drain(source: impl std::io::Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut { source }, &mut out);
        let _ = tx.send(out);
    });
    rx
}

fn recv_before(reader: &mpsc::Receiver<String>, deadline: Instant) -> Option<String> {
    reader
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()
}

fn signal_group(pgid: i32, signal: i32) {
    if pgid > 0 {
        unsafe {
            libc::kill(-pgid, signal);
        }
    }
}

fn stop_probe(child: &mut Child, pgid: i32) {
    signal_group(pgid, libc::SIGKILL);
    // The direct kill is a harmless backstop if process-group signalling was
    // unavailable for an environmental reason.
    let _ = child.kill();
    let _ = child.wait();
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

    const ESCAPED_HELPER: &str = "shared::proc::tests::escaped_pipe_helper";
    const GUARDIAN_HELPER: &str = "shared::proc::tests::guardian_owner_helper";

    fn scratch(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "omni-{label}-{}-{}",
            std::process::id(),
            crate::shared::clock::now().replace([':', '.'], "-")
        ))
    }

    fn read_pid(path: &std::path::Path) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                return text.trim().parse().unwrap();
            }
            assert!(Instant::now() < deadline, "helper did not publish its pid");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn group_alive(pgid: i32) -> bool {
        match unsafe { libc::kill(-pgid, 0) } {
            0 => true,
            _ => std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM),
        }
    }

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
    fn a_noisy_stderr_cannot_block_a_probe_that_answers_on_stdout() {
        let script =
            "i=0; while [ \"$i\" -lt 50000 ]; do printf x >&2; i=$((i + 1)); done; printf ok";
        let answer = output(&["sh", "-c", script], Duration::from_secs(10));
        assert_eq!(answer, Some((0, "ok".into())));
    }

    #[test]
    fn a_cli_that_ignores_eof_is_still_stopped() {
        let proc = Spawn::new(["sh", "-c", "trap '' TERM; while :; do sleep 0.1; done"])
            .start()
            .unwrap();
        proc.stop(Duration::from_millis(300));
        assert!(!proc.alive(), "SIGKILL is the backstop");
    }

    /// Subprocess fixture for `stop_is_bounded_when_an_escaped_child_keeps_a_pipe`.
    /// It is inert when the ordinary test harness runs it directly.
    #[test]
    fn escaped_pipe_helper() {
        let Some(pid_file) = std::env::var_os("OMNI_ESCAPED_PIPE_PID") else {
            return;
        };
        let child = unsafe { libc::fork() };
        assert!(child >= 0, "fixture fork failed");
        if child == 0 {
            // Leave the provider's process group but retain its stdout. Only
            // async-signal-safe calls are made in this post-fork child.
            unsafe {
                if libc::setsid() == -1 {
                    libc::_exit(125);
                }
                let pause = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 100_000_000,
                };
                let line = b"late-after-stop\n";
                for _ in 0..30 {
                    libc::nanosleep(&pause, std::ptr::null_mut());
                    libc::write(libc::STDOUT_FILENO, line.as_ptr().cast(), line.len());
                }
                libc::_exit(0);
            }
        }
        std::fs::write(pid_file, child.to_string()).unwrap();
    }

    #[test]
    fn stop_is_bounded_when_an_escaped_child_keeps_a_pipe() {
        let pid_file = scratch("escaped-pipe");
        let executable = std::env::current_exe().unwrap();
        let (tx, rx) = mpsc::channel();
        let proc = Spawn::new(vec![
            executable.to_string_lossy().into_owned(),
            "--exact".into(),
            ESCAPED_HELPER.into(),
            "--nocapture".into(),
        ])
        .env("OMNI_ESCAPED_PIPE_PID", pid_file.to_string_lossy())
        .on_line(move |line| {
            let _ = tx.send(line.to_string());
        })
        .start()
        .unwrap();
        let escaped = read_pid(&pid_file);

        let started = Instant::now();
        proc.stop(Duration::from_millis(100));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "an inherited pipe made stop exceed its signal grace"
        );

        while rx.try_recv().is_ok() {}
        assert!(
            rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "a detached reader invoked on_line after stop returned"
        );
        if alive(escaped) {
            unsafe { libc::kill(escaped, libc::SIGKILL) };
        }
        let _ = std::fs::remove_file(pid_file);
    }

    /// Subprocess fixture which exits without running `LineProcess::drop`.
    #[test]
    fn guardian_owner_helper() {
        let Some(pid_file) = std::env::var_os("OMNI_GUARDIAN_TARGET_PID") else {
            return;
        };
        let proc = Spawn::new(["sh", "-c", "trap '' TERM; while :; do sleep 1; done"])
            .start()
            .unwrap();
        std::fs::write(pid_file, proc.pgid.to_string()).unwrap();
        // Simulate SIGKILL/abort: no Rust destructor or shutdown hook runs.
        unsafe { libc::_exit(0) };
    }

    #[test]
    fn guardian_kills_a_provider_group_when_its_owner_dies_abruptly() {
        let pid_file = scratch("guardian-target");
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", GUARDIAN_HELPER, "--nocapture"])
            .env("OMNI_GUARDIAN_TARGET_PID", &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let target = read_pid(&pid_file);
        let deadline = Instant::now() + Duration::from_secs(5);
        while group_alive(target) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let survived = group_alive(target);
        if survived {
            signal_group(target, libc::SIGKILL);
        }
        let _ = std::fs::remove_file(pid_file);
        assert!(!survived, "the death-pipe guardian left its group alive");
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

    #[test]
    fn a_timed_out_probe_takes_its_grandchild_with_it() {
        let pid_file = std::env::temp_dir().join(format!(
            "omni-probe-child-{}-{}",
            std::process::id(),
            crate::shared::clock::now().replace([':', '.'], "-")
        ));
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                // The shell exits while its child retains both probe pipes.
                // Joining reader threads here used to hang past the timeout.
                "sleep 30 & child=$!; printf '%s\\n' \"$child\" > \"$1\"; exit 0",
                "probe",
            ])
            .arg(&pid_file);
        assert!(run(command, Duration::from_millis(500)).is_none());

        let grandchild: i32 = std::fs::read_to_string(&pid_file)
            .expect("the probe published its grandchild before waiting")
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while alive(grandchild) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(pid_file);
        assert!(
            !alive(grandchild),
            "timing out the probe must terminate the whole process group"
        );
    }
}

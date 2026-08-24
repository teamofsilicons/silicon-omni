//! Synchronous client for the persistent silicon omni daemon.
//!
//! [`Client`] owns one Unix-socket connection and a background reader. Calls
//! may be made concurrently: replies are matched by request id while session
//! frames are delivered to [`Subscription`]s. [`Client::open`] combines a
//! subscription with the daemon's `open` operation so replay frames cannot be
//! missed even when they arrive before the reply.
//!
//! `Client::connect` follows the same convention as the Python package: use an
//! existing daemon when one is listening, otherwise find and start `omnid`.
//! Use [`Client::connect_existing`] or [`Client::connect_to`] when starting a
//! process would be surprising.

#![cfg_attr(not(unix), allow(unused))]

#[cfg(not(unix))]
compile_error!("omni-client currently requires Unix-domain sockets");

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs::{File, OpenOptions as FsOpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

pub use omni_core::events::kind;
pub use omni_core::intelligence::Rung;
pub use omni_core::wire::{Frame, PROTOCOL, Request};
use omni_core::wire::{Incoming, Reply, read_line};
pub use omni_core::{Event, Snapshot};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long a normal daemon request may take.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// How long to wait for a daemon we started to bind its socket.
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// A transport, protocol, or daemon failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Io(String),
    Json(String),
    /// The daemon answered the request with `ok: false`.
    Daemon(String),
    /// No reply arrived before the request deadline.
    Timeout {
        op: String,
        after: Duration,
    },
    /// The socket closed while work was outstanding.
    Disconnected,
    /// One connection cannot open the same session twice: the daemon treats
    /// the connection itself as the subscription identity.
    AlreadyOpen(String),
    /// `omnid` was needed but could not be found.
    DaemonUnavailable(String),
    Protocol(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(message) | Error::Json(message) | Error::Daemon(message) => {
                write!(f, "{message}")
            }
            Error::Timeout { op, after } => {
                write!(f, "{op} got no answer in {}s", after.as_secs_f64())
            }
            Error::Disconnected => write!(f, "the omni daemon went away"),
            Error::AlreadyOpen(session) => {
                write!(f, "session {session:?} is already open on this client")
            }
            Error::DaemonUnavailable(message) | Error::Protocol(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::Json(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Information returned by `ping`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub protocol: u32,
    pub version: String,
    pub pid: u32,
    pub started: String,
    pub home: String,
}

/// One setting carried atomically with an `open` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    pub what: String,
    pub value: Value,
}

impl Setting {
    pub fn new(what: impl Into<String>, value: impl Into<Value>) -> Self {
        Setting {
            what: what.into(),
            value: value.into(),
        }
    }
}

/// Parameters for [`Client::open`].
#[derive(Debug, Clone)]
pub struct OpenOptions {
    pub session: String,
    pub providers: Option<Vec<String>>,
    /// The process directory offered when this creates a new session. A
    /// session that already has a directory keeps it unless `cwd` is also
    /// supplied as an explicit setting.
    pub cwd: Option<String>,
    /// First sequence number to replay. Use `-1` for future frames only.
    pub from: i64,
    /// Applied in order before a new provider launches, and at the next turn
    /// boundary when attaching to a session that is already warm.
    pub settings: Vec<Setting>,
}

impl OpenOptions {
    pub fn new(session: impl Into<String>) -> Self {
        OpenOptions {
            session: session.into(),
            providers: None,
            cwd: std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
            from: 0,
            settings: Vec::new(),
        }
    }

    pub fn providers(mut self, providers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.providers = Some(providers.into_iter().map(Into::into).collect());
        self
    }

    pub fn from_seq(mut self, from: i64) -> Self {
        self.from = from;
        self
    }

    /// Override the process directory offered for a newly-created session.
    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn setting(mut self, what: impl Into<String>, value: impl Into<Value>) -> Self {
        self.settings.push(Setting::new(what, value));
        self
    }
}

/// The daemon's answer after attaching a connection to a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opened {
    pub session: String,
    pub replayed: usize,
    pub listeners: usize,
    pub snapshot: Snapshot,
}

/// A live session as listed by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSession {
    pub snapshot: Snapshot,
    pub listeners: usize,
}

#[derive(Debug, Deserialize)]
struct Accepted {
    accepted: bool,
}

#[derive(Debug, Deserialize)]
struct Stopped {
    stopped: bool,
}

#[derive(Debug, Deserialize)]
struct Status {
    snapshot: Snapshot,
    listeners: usize,
}

#[derive(Debug, Deserialize)]
struct History {
    events: Vec<Event>,
}

#[derive(Debug, Deserialize)]
struct Sessions {
    sessions: Vec<LiveSession>,
}

struct Inner {
    writer: Mutex<UnixStream>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, Sender<Result<Reply>>>>,
    subscribers: Mutex<HashMap<String, HashMap<u64, Sender<Frame>>>>,
    next_subscriber: AtomicU64,
    open_sessions: Mutex<HashSet<String>>,
    alive: AtomicBool,
}

impl Inner {
    fn write(&self, request: &Request) -> Result<()> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(Error::Disconnected);
        }
        let mut line = serde_json::to_vec(request)?;
        line.push(b'\n');
        let result = {
            let mut writer = self.writer.lock().unwrap_or_else(|p| p.into_inner());
            writer.write_all(&line).and_then(|_| writer.flush())
        };
        result.map_err(|error| {
            self.disconnect();
            Error::Io(format!("could not send {}: {error}", request.op))
        })
    }

    fn disconnect(&self) {
        if !self.alive.swap(false, Ordering::SeqCst) {
            return;
        }
        let pending = std::mem::take(&mut *self.pending.lock().unwrap_or_else(|p| p.into_inner()));
        for waiter in pending.into_values() {
            let _ = waiter.send(Err(Error::Disconnected));
        }
        self.subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        self.open_sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        let _ = self
            .writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .shutdown(std::net::Shutdown::Both);
    }

    fn dispatch_frame(&self, frame: Frame) {
        let mut subscribers = self.subscribers.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(for_session) = subscribers.get_mut(&frame.session) {
            for_session.retain(|_, listener| listener.send(frame.clone()).is_ok());
        }
    }

    fn remove_subscription(&self, session: &str, id: u64) {
        let mut subscribers = self.subscribers.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(for_session) = subscribers.get_mut(session) {
            for_session.remove(&id);
            if for_session.is_empty() {
                subscribers.remove(session);
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.writer.get_mut().map(|stream| {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        });
    }
}

/// One multiplexed connection to `omnid`.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("alive", &self.is_connected())
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Connect to the daemon for the current `OMNI_HOME`, starting it if needed.
    pub fn connect() -> Result<Self> {
        let path = ensure_daemon(STARTUP_TIMEOUT)?;
        Self::connect_to(path)
    }

    /// Connect only if the daemon is already listening.
    pub fn connect_existing() -> Result<Self> {
        Self::connect_to(socket_path())
    }

    pub fn connect_to(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let stream = UnixStream::connect(path)
            .map_err(|error| Error::Io(format!("cannot connect to {}: {error}", path.display())))?;
        let client = Self::from_stream(stream)?;
        let hello: DaemonInfo = decode(
            "ping",
            client.call_with_timeout(Request::new(0, "ping"), HANDSHAKE_TIMEOUT)?,
        )?;
        if hello.protocol != PROTOCOL {
            return Err(Error::Protocol(format!(
                "omni protocol mismatch: this client speaks {PROTOCOL}, but the running daemon speaks {}; stop the old daemon and retry",
                hello.protocol
            )));
        }
        Ok(client)
    }

    /// Build a client around an already-connected stream.
    ///
    /// This is useful for embedders with their own discovery policy and for
    /// protocol tests built with [`UnixStream::pair`].
    pub fn from_stream(stream: UnixStream) -> Result<Self> {
        let reading = stream
            .try_clone()
            .map_err(|error| Error::Io(format!("cannot clone daemon socket: {error}")))?;
        let inner = Arc::new(Inner {
            writer: Mutex::new(stream),
            next_id: AtomicI64::new(1),
            pending: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(HashMap::new()),
            next_subscriber: AtomicU64::new(1),
            open_sessions: Mutex::new(HashSet::new()),
            alive: AtomicBool::new(true),
        });
        let reader_inner = Arc::downgrade(&inner);
        std::thread::Builder::new()
            .name("omni-client:reader".into())
            .spawn(move || read_daemon(reading, reader_inner))
            .map_err(|error| Error::Io(format!("cannot start socket reader: {error}")))?;
        Ok(Client { inner })
    }

    pub fn is_connected(&self) -> bool {
        self.inner.alive.load(Ordering::SeqCst)
    }

    /// Send any protocol request and return its result.
    ///
    /// The id supplied by the caller is replaced with one unique to this
    /// connection. Typed helpers below cover the stable public operations.
    pub fn call(&self, request: Request) -> Result<Value> {
        self.call_with_timeout(request, DEFAULT_TIMEOUT)
    }

    pub fn call_with_timeout(&self, mut request: Request, timeout: Duration) -> Result<Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        request.id = id;
        let op = request.op.clone();
        let (settle, answer) = mpsc::channel();
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, settle);
        if let Err(error) = self.inner.write(&request) {
            self.inner
                .pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&id);
            return Err(error);
        }
        let reply = match answer.recv_timeout(timeout) {
            Ok(reply) => reply?,
            Err(RecvTimeoutError::Timeout) => {
                self.inner
                    .pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                return Err(Error::Timeout { op, after: timeout });
            }
            Err(RecvTimeoutError::Disconnected) => return Err(Error::Disconnected),
        };
        match reply.ok {
            true => Ok(reply.result),
            false => Err(Error::Daemon(
                reply
                    .error
                    .unwrap_or_else(|| format!("{} failed", request.op)),
            )),
        }
    }

    /// Subscribe locally before an `open`, so even pre-reply replay is queued.
    pub fn subscribe(&self, session: impl Into<String>) -> Subscription {
        let session = session.into();
        let id = self.inner.next_subscriber.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        self.inner
            .subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(session.clone())
            .or_default()
            .insert(id, sender);
        Subscription {
            session,
            id,
            receiver,
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub fn ping(&self) -> Result<DaemonInfo> {
        decode("ping", self.call(Request::new(0, "ping"))?)
    }

    pub fn open(&self, options: OpenOptions) -> Result<Session> {
        {
            let mut active = self
                .inner
                .open_sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if !active.insert(options.session.clone()) {
                return Err(Error::AlreadyOpen(options.session));
            }
        }
        let subscription = self.subscribe(options.session.clone());
        let mut request = Request::new(0, "open").on(&options.session);
        request.providers = options.providers;
        request.cwd = options.cwd;
        request.from = Some(options.from);
        request.value = serde_json::to_value(options.settings)?;
        match self
            .call(request)
            .and_then(|value| decode::<Opened>("open", value))
        {
            Ok(opened) => Ok(Session {
                client: self.clone(),
                subscription,
                opened,
                attached: true,
            }),
            Err(error) => {
                self.inner
                    .open_sessions
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&options.session);
                Err(error)
            }
        }
    }

    pub fn send(&self, session: &str, text: &str) -> Result<bool> {
        let result: Accepted = decode(
            "send",
            self.call(Request::new(0, "send").on(session).saying(text))?,
        )?;
        Ok(result.accepted)
    }

    pub fn set(&self, session: &str, what: &str, value: Value) -> Result<bool> {
        let result: Accepted = decode(
            "set",
            self.call(Request::new(0, "set").on(session).about(what, value))?,
        )?;
        Ok(result.accepted)
    }

    pub fn stop(&self, session: &str) -> Result<bool> {
        let result: Stopped = decode("stop", self.call(Request::new(0, "stop").on(session))?)?;
        Ok(result.stopped)
    }

    pub fn status(&self, session: &str) -> Result<(Snapshot, usize)> {
        let result: Status = decode("status", self.call(Request::new(0, "status").on(session))?)?;
        Ok((result.snapshot, result.listeners))
    }

    pub fn events(&self, session: &str, from: i64) -> Result<Vec<Event>> {
        let mut request = Request::new(0, "events").on(session);
        request.from = Some(from);
        let result: History = decode("events", self.call(request)?)?;
        Ok(result.events)
    }

    pub fn sessions(&self) -> Result<Vec<LiveSession>> {
        let result: Sessions = decode("sessions", self.call(Request::new(0, "sessions"))?)?;
        Ok(result.sessions)
    }

    pub fn providers(&self, only: Option<Vec<String>>) -> Result<Vec<String>> {
        let mut request = Request::new(0, "providers");
        request.providers = only;
        decode("providers", self.call(request)?)
    }

    pub fn dial(&self, providers: Option<Vec<String>>) -> Result<BTreeMap<i64, Rung>> {
        let mut request = Request::new(0, "dial");
        request.providers = providers;
        decode("dial", self.call(request)?)
    }

    /// Ask one provider account question (`auth_status`, `installed`,
    /// `limits`, `start_auth`, `finish_auth`, or `forget`).
    pub fn account(&self, provider: &str, what: &str, text: Option<&str>) -> Result<Value> {
        let mut request = Request::new(0, "account");
        request.provider = Some(provider.into());
        request.what = Some(what.into());
        request.text = text.map(str::to_owned);
        // Completing browser auth can legitimately wait several minutes for
        // the user and for a CLI callback. It must not inherit the ordinary
        // two-minute request deadline.
        let timeout = match what {
            "finish_auth" => Duration::from_secs(600),
            "start_auth" | "limits" => Duration::from_secs(180),
            _ => DEFAULT_TIMEOUT,
        };
        self.call_with_timeout(request, timeout)
    }

    pub fn shutdown(&self) -> Result<()> {
        self.call(Request::new(0, "shutdown"))?;
        Ok(())
    }

    /// Send a best-effort request whose reply is intentionally ignored.
    fn notify(&self, mut request: Request) {
        request.id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let _ = self.inner.write(&request);
    }
}

/// Frames for one session, filtered out of a multiplexed client connection.
pub struct Subscription {
    session: String,
    id: u64,
    receiver: Receiver<Frame>,
    inner: Weak<Inner>,
}

impl fmt::Debug for Subscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscription")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl Subscription {
    pub fn session(&self) -> &str {
        &self.session
    }

    pub fn recv(&self) -> Result<Frame> {
        self.receiver.recv().map_err(|_| Error::Disconnected)
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<Frame>> {
        match self.receiver.recv_timeout(timeout) {
            Ok(frame) => Ok(Some(frame)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(Error::Disconnected),
        }
    }

    pub fn try_recv(&self) -> Result<Option<Frame>> {
        match self.receiver.try_recv() {
            Ok(frame) => Ok(Some(frame)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(Error::Disconnected),
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.remove_subscription(&self.session, self.id);
        }
    }
}

/// An attached session and its ordered stream of replay/live frames.
pub struct Session {
    client: Client,
    subscription: Subscription,
    opened: Opened,
    attached: bool,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("session", &self.opened.session)
            .field("attached", &self.attached)
            .field("snapshot", &self.opened.snapshot)
            .finish()
    }
}

impl Session {
    pub fn id(&self) -> &str {
        &self.opened.session
    }

    pub fn opened(&self) -> &Opened {
        &self.opened
    }

    pub fn recv(&self) -> Result<Frame> {
        self.subscription.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<Frame>> {
        self.subscription.recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<Option<Frame>> {
        self.subscription.try_recv()
    }

    pub fn send(&self, text: &str) -> Result<bool> {
        self.client.send(self.id(), text)
    }

    pub fn set(&self, what: &str, value: Value) -> Result<bool> {
        self.client.set(self.id(), what, value)
    }

    pub fn status(&self) -> Result<(Snapshot, usize)> {
        self.client.status(self.id())
    }

    /// End the live conversation and detach this handle.
    pub fn stop(&mut self) -> Result<bool> {
        self.stop_with_timeout(DEFAULT_TIMEOUT)
    }

    /// Stop listening while the daemon keeps the provider warm.
    pub fn detach(&mut self) -> Result<()> {
        self.detach_with_timeout(DEFAULT_TIMEOUT)
    }

    fn stop_with_timeout(&mut self, timeout: Duration) -> Result<bool> {
        let session = self.id().to_string();
        let result = self
            .client
            .call_with_timeout(Request::new(0, "stop").on(&session), timeout)
            .and_then(|value| decode::<Stopped>("stop", value))
            .map(|answer| answer.stopped);
        self.finish_lifecycle(result)
    }

    fn detach_with_timeout(&mut self, timeout: Duration) -> Result<()> {
        let session = self.id().to_string();
        let result = self
            .client
            .call_with_timeout(Request::new(0, "detach").on(&session), timeout)
            .map(|_| ());
        self.finish_lifecycle(result)
    }

    /// A failed lifecycle call leaves it ambiguous whether the daemon acted
    /// before its reply was lost. Closing the whole connection is the only
    /// operation that unconditionally makes the daemon forget every listener
    /// owned by this client. Keep `attached` true until Drop so it does not
    /// mistake the failed request for an acknowledged detach.
    fn finish_lifecycle<T>(&mut self, result: Result<T>) -> Result<T> {
        match result {
            Ok(value) => {
                self.mark_detached();
                Ok(value)
            }
            Err(error) => {
                self.client.inner.disconnect();
                Err(error)
            }
        }
    }

    fn mark_detached(&mut self) {
        if self.attached {
            self.attached = false;
            self.client
                .inner
                .remove_subscription(&self.subscription.session, self.subscription.id);
            let session = self.opened.session.clone();
            self.client
                .inner
                .open_sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&session);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.attached {
            let id = self.id().to_string();
            self.client.notify(Request::new(0, "detach").on(&id));
            self.mark_detached();
        }
    }
}

fn decode<T: DeserializeOwned>(op: &str, value: Value) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|error| Error::Protocol(format!("bad {op} reply from daemon: {error}")))
}

fn read_daemon(stream: UnixStream, inner: Weak<Inner>) {
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else {
            break;
        };
        let Some(inner) = inner.upgrade() else {
            return;
        };
        match read_line(&line) {
            Some(Incoming::Reply(reply)) => {
                let waiter = inner
                    .pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&reply.id);
                if let Some(waiter) = waiter {
                    let _ = waiter.send(Ok(reply));
                }
            }
            Some(Incoming::Frame(frame)) => inner.dispatch_frame(*frame),
            None => continue,
        }
    }
    if let Some(inner) = inner.upgrade() {
        inner.disconnect();
    }
}

/// The socket selected by `OMNI_HOME` (or `~/.omni`).
pub fn socket_path() -> PathBuf {
    omni_core::shared::paths::socket()
}

/// Locate `omnid` without starting it.
pub fn daemon_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OMNI_DAEMON").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            let sibling = parent.join("omnid");
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            let candidate = path.join("omnid");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // A contributor running examples or tests from this checkout should find
    // the daemon cargo already built beside the workspace target directory.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for root in manifest.ancestors() {
        if !root.join("Cargo.toml").is_file() {
            continue;
        }
        for profile in ["release", "debug"] {
            let candidate = root.join("target").join(profile).join("omnid");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// True only when something accepts a connection at the current socket.
pub fn daemon_is_listening() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// Ensure `omnid` is listening and return its socket path.
pub fn ensure_daemon(timeout: Duration) -> Result<PathBuf> {
    let socket = socket_path();
    if UnixStream::connect(&socket).is_ok() {
        return Ok(socket);
    }
    let binary = daemon_binary().ok_or_else(|| {
        Error::DaemonUnavailable(
            "omnid is not installed; install omni-daemon, build `cargo build -p omni-daemon`, or set OMNI_DAEMON".into(),
        )
    })?;
    let home = omni_core::shared::paths::home();
    omni_core::shared::paths::ensure(&home).map_err(|error| {
        Error::Io(format!(
            "cannot create omni home {}: {error}",
            home.display()
        ))
    })?;
    let log_path = omni_core::shared::paths::log_file();
    let log = append_log(&log_path)?;
    let errors = log
        .try_clone()
        .map_err(|error| Error::Io(format!("cannot clone {}: {error}", log_path.display())))?;
    let mut command = Command::new(&binary);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(errors))
        .env("OMNI_HOME", &home)
        // stdout and stderr already point at omnid.log. Without this marker
        // the daemon would mirror its status lines into the same file again.
        .env("OMNI_STDIO_LOGGED", "1");
    use std::os::unix::process::CommandExt;
    // Give the daemon its own session so it survives the terminal or shell
    // which happened to launch the first client.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| Error::Io(format!("cannot start {}: {error}", binary.display())))?;
    // A racing client may have started the winning daemon milliseconds before
    // us. The process we spawned then exits on the daemon lock; reap it instead
    // of leaving a zombie behind in this long-lived client.
    let _ = std::thread::Builder::new()
        .name("omni-client:daemon-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        });

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if UnixStream::connect(&socket).is_ok() {
            return Ok(socket);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(Error::DaemonUnavailable(format!(
        "started {} but nothing listened on {} after {}s; see {}",
        binary.display(),
        socket.display(),
        timeout.as_secs_f64(),
        log_path.display()
    )))
}

fn append_log(path: &Path) -> Result<File> {
    let file = FsOpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| Error::Io(format!("cannot open {}: {error}", path.display())))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| Error::Io(format!("cannot secure {}: {error}", path.display())))?;
    Ok(file)
}

/// Wait for the current daemon socket to stop accepting connections.
pub fn wait_until_stopped(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !daemon_is_listening() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    !daemon_is_listening()
}

#[cfg(test)]
mod tests {
    use super::*;
    use omni_core::events::kind;
    use serde_json::json;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    fn snapshot(session: &str, seq: i64) -> Snapshot {
        Snapshot {
            session: session.into(),
            status: "waiting".into(),
            level: 5,
            providers: vec!["test".into()],
            provider: "test".into(),
            model: "double".into(),
            effort: "medium".into(),
            cwd: "/tmp".into(),
            seq,
            in_turn: false,
            queued: 0,
        }
    }

    fn send_json(stream: &mut UnixStream, value: &impl Serialize) {
        serde_json::to_writer(&mut *stream, value).unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
    }

    fn opened(request: &Request) -> Reply {
        Reply::ok(
            request.id,
            json!({
                "session": "demo",
                "replayed": 0,
                "listeners": 1,
                "snapshot": snapshot("demo", 0),
            }),
        )
    }

    fn lifecycle(session: &mut Session, op: &str, timeout: Duration) -> Result<()> {
        match op {
            "stop" => session.stop_with_timeout(timeout).map(|_| ()),
            "detach" => session.detach_with_timeout(timeout),
            other => panic!("unexpected lifecycle operation {other}"),
        }
    }

    fn assert_failed_lifecycle_cleanup(client: &Client, session: &Session) {
        // The Session must remain logically attached until Drop, even though
        // disconnect has already removed all local routing state and closed
        // the socket so the daemon cannot retain its listener.
        assert!(session.attached);
        assert!(!client.is_connected());
        assert!(
            client
                .inner
                .subscribers
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_empty()
        );
        assert!(
            client
                .inner
                .open_sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_empty()
        );
        assert_eq!(session.try_recv().unwrap_err(), Error::Disconnected);
    }

    #[test]
    fn connecting_rejects_a_daemon_that_speaks_another_protocol() {
        let path = std::env::temp_dir().join(format!(
            "omni-client-protocol-{}-{:?}.sock",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let reading = stream.try_clone().unwrap();
            let line = BufReader::new(reading).lines().next().unwrap().unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            assert_eq!(request.op, "ping");
            send_json(
                &mut stream,
                &Reply::ok(
                    request.id,
                    json!({
                        "protocol": PROTOCOL + 1,
                        "version": "other",
                        "pid": 1,
                        "started": "now",
                        "home": "/tmp",
                    }),
                ),
            );
        });

        assert!(matches!(Client::connect_to(&path), Err(Error::Protocol(_))));
        server.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn concurrent_calls_are_matched_while_frames_arrive_between_them() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let client = Client::from_stream(ours).unwrap();
        let events = client.subscribe("demo");
        let (first_seen, continue_second) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let reading = theirs.try_clone().unwrap();
            let mut lines = BufReader::new(reading).lines();
            let first: Request = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            first_seen.send(()).unwrap();
            let second: Request = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            let mut writing = theirs;
            send_json(
                &mut writing,
                &Reply::ok(second.id, json!({"which": second.op})),
            );
            let event = Event::new(kind::TEXT).saying("between");
            send_json(
                &mut writing,
                &Frame::event("demo", event, snapshot("demo", 1)),
            );
            send_json(
                &mut writing,
                &Reply::ok(first.id, json!({"which": first.op})),
            );
        });

        let first_client = client.clone();
        let first = std::thread::spawn(move || first_client.call(Request::new(0, "one")));
        continue_second.recv().unwrap();
        let second = client.call(Request::new(0, "two")).unwrap();
        assert_eq!(second["which"], "two");
        assert_eq!(first.join().unwrap().unwrap()["which"], "one");
        assert_eq!(events.recv().unwrap().event.unwrap().text, "between");
        server.join().unwrap();
    }

    #[test]
    fn open_captures_replay_that_precedes_its_reply() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let client = Client::from_stream(ours).unwrap();
        let server = std::thread::spawn(move || {
            let reading = theirs.try_clone().unwrap();
            let line = BufReader::new(reading).lines().next().unwrap().unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            assert_eq!(request.op, "open");
            assert_eq!(request.from, Some(3));
            assert_eq!(request.cwd.as_deref(), Some("/client/work"));
            let mut writing = theirs;
            let event = Event::new(kind::TEXT).saying("replayed");
            send_json(
                &mut writing,
                &Frame::event("demo", event, snapshot("demo", 3)),
            );
            send_json(
                &mut writing,
                &Reply::ok(
                    request.id,
                    json!({
                        "session": "demo",
                        "replayed": 1,
                        "listeners": 1,
                        "snapshot": snapshot("demo", 3),
                    }),
                ),
            );
        });

        let mut session = client
            .open(OpenOptions::new("demo").cwd("/client/work").from_seq(3))
            .unwrap();
        assert_eq!(session.opened().replayed, 1);
        assert_eq!(session.recv().unwrap().event.unwrap().text, "replayed");
        // Avoid making the closed mock serve a best-effort detach in Drop.
        session.mark_detached();
        server.join().unwrap();
    }

    #[test]
    fn open_options_default_to_the_calling_process_directory() {
        let expected = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            OpenOptions::new("demo").cwd.as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn daemon_refusals_are_typed_errors() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let client = Client::from_stream(ours).unwrap();
        let server = std::thread::spawn(move || {
            let reading = theirs.try_clone().unwrap();
            let line = BufReader::new(reading).lines().next().unwrap().unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            let mut writing = theirs;
            send_json(&mut writing, &Reply::failed(request.id, "not today"));
        });
        assert_eq!(
            client.call(Request::new(0, "nope")).unwrap_err(),
            Error::Daemon("not today".into())
        );
        server.join().unwrap();
    }

    #[test]
    fn a_timed_out_call_does_not_poison_the_next_one() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let client = Client::from_stream(ours).unwrap();
        let server = std::thread::spawn(move || {
            let reading = theirs.try_clone().unwrap();
            let mut lines = BufReader::new(reading).lines();
            let _first: Request = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            let second: Request = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            let mut writing = theirs;
            send_json(&mut writing, &Reply::ok(second.id, json!(true)));
        });
        assert!(matches!(
            client.call_with_timeout(Request::new(0, "slow"), Duration::from_millis(10)),
            Err(Error::Timeout { .. })
        ));
        assert_eq!(client.call(Request::new(0, "next")).unwrap(), json!(true));
        server.join().unwrap();
    }

    #[test]
    fn dropping_the_last_client_closes_its_socket_and_reader() {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let client = Client::from_stream(ours).unwrap();
        drop(client);
        theirs
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut byte = [0_u8; 1];
        assert_eq!(std::io::Read::read(&mut theirs, &mut byte).unwrap(), 0);
    }

    #[test]
    fn a_failed_open_does_not_reserve_the_session_locally() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let client = Client::from_stream(ours).unwrap();
        let server = std::thread::spawn(move || {
            let reading = theirs.try_clone().unwrap();
            let mut lines = BufReader::new(reading).lines();
            let first: Request = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            let mut writing = theirs.try_clone().unwrap();
            send_json(&mut writing, &Reply::failed(first.id, "try again"));
            let second: Request = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            let mut writing = theirs;
            send_json(
                &mut writing,
                &Reply::ok(
                    second.id,
                    json!({
                        "session": "demo",
                        "replayed": 0,
                        "listeners": 1,
                        "snapshot": snapshot("demo", 0),
                    }),
                ),
            );
        });

        assert!(matches!(
            client.open(OpenOptions::new("demo")),
            Err(Error::Daemon(_))
        ));
        let mut session = client.open(OpenOptions::new("demo")).unwrap();
        session.mark_detached();
        server.join().unwrap();
    }

    #[test]
    fn successful_stop_and_detach_acknowledgements_remove_only_their_listener() {
        for op in ["stop", "detach"] {
            let (ours, theirs) = UnixStream::pair().unwrap();
            let client = Client::from_stream(ours).unwrap();
            let (release, hold) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let reading = theirs.try_clone().unwrap();
                let mut lines = BufReader::new(reading).lines();
                let open: Request = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                let mut writing = theirs;
                send_json(&mut writing, &opened(&open));

                let request: Request =
                    serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                assert_eq!(request.op, op);
                let result = match op {
                    "stop" => json!({"stopped": true}),
                    "detach" => json!({"detached": "demo"}),
                    _ => unreachable!(),
                };
                send_json(&mut writing, &Reply::ok(request.id, result));
                hold.recv_timeout(Duration::from_secs(2)).unwrap();
            });

            let mut session = client.open(OpenOptions::new("demo")).unwrap();
            lifecycle(&mut session, op, Duration::from_secs(1)).unwrap();
            assert!(!session.attached);
            assert!(client.is_connected());
            assert!(
                client
                    .inner
                    .subscribers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_empty()
            );
            assert!(
                client
                    .inner
                    .open_sessions
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_empty()
            );
            assert_eq!(session.try_recv().unwrap_err(), Error::Disconnected);

            release.send(()).unwrap();
            server.join().unwrap();
        }
    }

    #[test]
    fn refused_stop_and_detach_close_the_transport_without_marking_the_session_detached() {
        for op in ["stop", "detach"] {
            let (ours, theirs) = UnixStream::pair().unwrap();
            let client = Client::from_stream(ours).unwrap();
            let server = std::thread::spawn(move || {
                let reading = theirs.try_clone().unwrap();
                reading
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut lines = BufReader::new(reading).lines();
                let open: Request = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                let mut writing = theirs;
                send_json(&mut writing, &opened(&open));

                let request: Request =
                    serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                assert_eq!(request.op, op);
                send_json(&mut writing, &Reply::failed(request.id, "refused"));

                let after_failure = lines.next();
                assert!(
                    after_failure.is_none(),
                    "failed {op} did not close the client socket: {after_failure:?}"
                );
            });

            let mut session = client.open(OpenOptions::new("demo")).unwrap();
            assert_eq!(
                lifecycle(&mut session, op, Duration::from_secs(1)).unwrap_err(),
                Error::Daemon("refused".into())
            );
            assert_failed_lifecycle_cleanup(&client, &session);
            server.join().unwrap();
        }
    }

    #[test]
    fn timed_out_stop_and_detach_close_the_transport_without_leaking_a_listener() {
        for op in ["stop", "detach"] {
            let (ours, theirs) = UnixStream::pair().unwrap();
            let client = Client::from_stream(ours).unwrap();
            let server = std::thread::spawn(move || {
                let reading = theirs.try_clone().unwrap();
                reading
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut lines = BufReader::new(reading).lines();
                let open: Request = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                let mut writing = theirs;
                send_json(&mut writing, &opened(&open));

                let request: Request =
                    serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                assert_eq!(request.op, op);
                // Deliberately withhold the reply. The lifecycle timeout must
                // close the socket, which is the daemon's unambiguous signal
                // to forget every listener owned by this connection.
                let after_timeout = lines.next();
                assert!(
                    after_timeout.is_none(),
                    "timed-out {op} did not close the client socket: {after_timeout:?}"
                );
            });

            let timeout = Duration::from_millis(20);
            let mut session = client.open(OpenOptions::new("demo")).unwrap();
            assert_eq!(
                lifecycle(&mut session, op, timeout).unwrap_err(),
                Error::Timeout {
                    op: op.into(),
                    after: timeout,
                }
            );
            assert_failed_lifecycle_cleanup(&client, &session);
            server.join().unwrap();
        }
    }
}

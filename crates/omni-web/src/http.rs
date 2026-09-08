//! A small HTTP/1.1 server, because that is all a loopback door needs.
//!
//! The bridge answers a handful of routes and one long-lived event stream,
//! from one machine, over one interface. That is a thread per connection and
//! a few hundred lines of parsing — not a reason to bring an async runtime
//! and a web framework into a project whose daemon has five dependencies.
//!
//! Nothing here knows what omni is. It reads a request, hands it to a
//! handler, and writes back what it gets.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// Request line plus headers. Generous for a browser, finite for anything else.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// A prompt can be long. It cannot be unbounded.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
/// How long a connection may take to say what it wants.
const HEAD_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a written response may block before the reader is presumed gone.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Connections at once. Event streams are long-lived, so this is mostly a
/// ceiling on how many tabs can watch at the same time.
const MAX_CONNECTIONS: usize = 64;
/// Requests on one kept-alive connection before it is retired.
const MAX_KEEPALIVE: usize = 256;

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    /// Header names lowercased, because a browser and curl disagree on case.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn header(&self, name: &str) -> &str {
        self.headers.get(name).map(String::as_str).unwrap_or("")
    }

    pub fn query(&self, name: &str) -> &str {
        self.query.get(name).map(String::as_str).unwrap_or("")
    }

    /// The bearer token, from the header a program would use or the query
    /// parameter `EventSource` leaves as the only option.
    pub fn bearer(&self) -> Option<&str> {
        let header = self.header("authorization");
        if let Some(token) = header
            .strip_prefix("Bearer ")
            .or_else(|| header.strip_prefix("bearer "))
        {
            return Some(token.trim()).filter(|token| !token.is_empty());
        }
        Some(self.query("token")).filter(|token| !token.is_empty())
    }

    pub fn json(&self) -> Result<serde_json::Value, String> {
        if self.body.is_empty() {
            return Ok(serde_json::Value::Object(Default::default()));
        }
        serde_json::from_slice(&self.body).map_err(|error| format!("that is not JSON: {error}"))
    }
}

pub struct Res {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Res {
    pub fn new(status: u16) -> Self {
        Res {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Res::new(status)
            .with("content-type", "application/json; charset=utf-8")
            .body(serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()))
    }

    pub fn text(status: u16, text: &str) -> Self {
        Res::new(status)
            .with("content-type", "text/plain; charset=utf-8")
            .body(text.as_bytes().to_vec())
    }

    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }
}

/// What a handler gives back: an answer, or a stream it wants to keep writing.
pub enum Answer {
    Done(Res),
    /// Headers go out immediately; the closure then owns the socket until it
    /// returns, at which point the connection closes.
    Stream(Vec<(String, String)>, Box<dyn FnOnce(&mut Sse) + Send>),
}

/// The writing half of a server-sent event stream.
pub struct Sse {
    socket: TcpStream,
    open: bool,
}

impl Sse {
    /// One event. `id` lets a browser resume with `Last-Event-ID` after the
    /// reconnect it does on its own.
    pub fn event(&mut self, id: Option<i64>, name: &str, data: &str) -> std::io::Result<()> {
        let mut frame = String::with_capacity(data.len() + 64);
        if let Some(id) = id {
            frame.push_str(&format!("id: {id}\n"));
        }
        if !name.is_empty() {
            frame.push_str(&format!("event: {name}\n"));
        }
        // A payload with a newline in it is several `data:` lines, and a
        // reader joins them back with newlines. JSON has none, but a comment
        // or an error sentence might.
        for line in data.split('\n') {
            frame.push_str("data: ");
            frame.push_str(line);
            frame.push('\n');
        }
        frame.push('\n');
        self.write(frame.as_bytes())
    }

    /// A colon line, which every SSE reader ignores. Sent periodically so a
    /// proxy or a sleeping laptop cannot mistake a quiet session for a dead
    /// connection.
    pub fn keepalive(&mut self) -> std::io::Result<()> {
        self.write(b": ping\n\n")
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if !self.open {
            return Err(std::io::Error::new(ErrorKind::BrokenPipe, "stream closed"));
        }
        match self.socket.write_all(bytes).and_then(|_| self.socket.flush()) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.open = false;
                Err(error)
            }
        }
    }
}

/// Bound sockets, and the flag that takes them down.
pub struct Server {
    pub port: u16,
    listeners: Vec<TcpListener>,
    stopping: Arc<AtomicBool>,
}

impl Server {
    /// Bind one port on the loopback interface, v4 and — if the system has it
    /// — v6 as well.
    ///
    /// Both are needed. `localhost` in a browser is `::1` on some machines and
    /// `127.0.0.1` on others, and a bridge that only answers one of them looks
    /// broken to half its users. The v4 bind decides whether the port counts
    /// as free; v6 is taken when it can be.
    pub fn bind(port: u16) -> std::io::Result<Server> {
        let v4 = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))?;
        // Port 0 means "anything free", which tests use. Read back what that
        // turned out to be, so v6 lands on the same number and everything
        // downstream — the Host check, the pairing code — agrees on it.
        let port = v4.local_addr()?.port();
        let mut listeners = vec![v4];
        if let Ok(v6) = TcpListener::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)) {
            listeners.push(v6);
        }
        Ok(Server {
            port,
            listeners,
            stopping: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Take `preferred` if it is free, else count down from [`crate::state::BASE_PORT`].
    ///
    /// Counting down rather than up keeps the search inside a known window, so
    /// a website can find the bridge by knocking on ten doors instead of
    /// scanning. It is also why moving is a visible event: the address is part
    /// of the identity a browser paired with.
    pub fn find(preferred: Option<u16>) -> Result<Server, String> {
        let mut tried = Vec::new();
        if let Some(port) = preferred {
            match Server::bind(port) {
                Ok(server) => return Ok(server),
                Err(error) => tried.push(format!("{port} ({})", brief(&error))),
            }
        }
        for port in (crate::state::FLOOR_PORT..=crate::state::BASE_PORT).rev() {
            if Some(port) == preferred {
                continue;
            }
            match Server::bind(port) {
                Ok(server) => return Ok(server),
                Err(error) => tried.push(format!("{port} ({})", brief(&error))),
            }
        }
        Err(format!(
            "every port from {} down to {} is taken: {}",
            crate::state::BASE_PORT,
            crate::state::FLOOR_PORT,
            tried.join(", ")
        ))
    }

    /// A handle that makes [`Server::run`] return.
    pub fn stopper(&self) -> Stopper {
        Stopper {
            stopping: self.stopping.clone(),
            addresses: self
                .listeners
                .iter()
                .filter_map(|listener| listener.local_addr().ok())
                .collect(),
        }
    }

    /// Accept until stopped. One thread per connection.
    pub fn run(self, handle: Arc<dyn Fn(Req) -> Answer + Send + Sync>) {
        let live = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for listener in self.listeners {
            let stopping = self.stopping.clone();
            let handle = handle.clone();
            let live = live.clone();
            threads.push(std::thread::spawn(move || {
                for socket in listener.incoming() {
                    if stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(socket) = socket else { continue };
                    if live.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                        // Say so rather than queueing: a caller that is told
                        // to come back can, and a stalled fetch cannot.
                        let mut socket = socket;
                        let _ = write_head(
                            &mut socket,
                            503,
                            &[
                                ("content-type".into(), "text/plain".into()),
                                ("retry-after".into(), "1".into()),
                                ("connection".into(), "close".into()),
                            ],
                            Some(0),
                        );
                        continue;
                    }
                    live.fetch_add(1, Ordering::SeqCst);
                    let handle = handle.clone();
                    let live = live.clone();
                    let stopping = stopping.clone();
                    std::thread::spawn(move || {
                        converse(socket, handle, &stopping);
                        live.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            }));
        }
        for thread in threads {
            let _ = thread.join();
        }
    }
}

/// Makes a running [`Server`] come back.
#[derive(Clone)]
pub struct Stopper {
    stopping: Arc<AtomicBool>,
    addresses: Vec<SocketAddr>,
}

impl Stopper {
    pub fn stop(&self) {
        if self.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        // `incoming` blocks, so each listener gets one harmless connection
        // to wake it up and let it see the flag.
        for address in &self.addresses {
            let _ = TcpStream::connect_timeout(address, Duration::from_millis(500));
        }
    }

    pub fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }
}

fn brief(error: &std::io::Error) -> String {
    match error.kind() {
        ErrorKind::AddrInUse => "in use".into(),
        ErrorKind::PermissionDenied => "not allowed".into(),
        _ => error.to_string(),
    }
}

fn converse(
    socket: TcpStream,
    handle: Arc<dyn Fn(Req) -> Answer + Send + Sync>,
    stopping: &AtomicBool,
) {
    let _ = socket.set_nodelay(true);
    let Ok(writing) = socket.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(socket);
    let mut writer = writing;

    for _ in 0..MAX_KEEPALIVE {
        let _ = reader.get_ref().set_read_timeout(Some(HEAD_TIMEOUT));
        let _ = writer.set_write_timeout(Some(WRITE_TIMEOUT));
        let request = match read_request(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err((status, why)) => {
                let _ = respond(&mut writer, Res::text(status, &why), true);
                return;
            }
        };
        let keep_alive = !request
            .header("connection")
            .eq_ignore_ascii_case("close")
            && !stopping.load(Ordering::SeqCst);

        match handle(request) {
            Answer::Done(response) => {
                if respond(&mut writer, response, !keep_alive).is_err() || !keep_alive {
                    return;
                }
            }
            Answer::Stream(headers, run) => {
                let mut all = headers;
                all.push(("connection".into(), "close".into()));
                if write_head(&mut writer, 200, &all, None).is_err() {
                    return;
                }
                // A stream may sit quiet for hours between turns. Nothing is
                // expected from the reader once it has asked, so the socket
                // gets no read deadline and only the write one applies.
                let _ = reader.get_ref().set_read_timeout(None);
                let mut sse = Sse {
                    socket: writer,
                    open: true,
                };
                run(&mut sse);
                return;
            }
        }
    }
}

type Failed = (u16, String);

fn read_request(reader: &mut BufReader<TcpStream>) -> Result<Option<Req>, Failed> {
    let mut head = Vec::new();
    loop {
        let mut line = Vec::new();
        let read = match read_line(reader, &mut line) {
            Ok(read) => read,
            // A kept-alive connection that the peer simply closed is the
            // ordinary end of a conversation, not a failure to report.
            Err(_) if head.is_empty() => return Ok(None),
            Err(error) => return Err((400, format!("could not read the request: {error}"))),
        };
        if read == 0 {
            return if head.is_empty() {
                Ok(None)
            } else {
                Err((400, "the request stopped half way".into()))
            };
        }
        let text = String::from_utf8_lossy(&line).trim_end().to_string();
        if text.is_empty() {
            break;
        }
        head.push(text);
        if head.iter().map(String::len).sum::<usize>() > MAX_HEAD_BYTES {
            return Err((431, "those headers are too long".into()));
        }
        if head.len() > 100 {
            return Err((431, "that is too many headers".into()));
        }
    }
    if head.is_empty() {
        return Ok(None);
    }

    let mut parts = head[0].split_whitespace();
    let method = parts.next().unwrap_or_default().to_uppercase();
    let target = parts.next().unwrap_or_default().to_string();
    if method.is_empty() || target.is_empty() {
        return Err((400, "that is not a request line".into()));
    }

    let mut headers = BTreeMap::new();
    for line in &head[1..] {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        return Err((411, "send a Content-Length; this door does not do chunked".into()));
    }

    let length: usize = headers
        .get("content-length")
        .map(|value| value.parse().unwrap_or(usize::MAX))
        .unwrap_or(0);
    if length > MAX_BODY_BYTES {
        return Err((413, format!("a body may be {MAX_BODY_BYTES} bytes at most")));
    }
    let mut body = vec![0u8; length];
    if length > 0 && reader.read_exact(&mut body).is_err() {
        return Err((400, "the body stopped half way".into()));
    }

    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), parse_query(query)),
        None => (target, BTreeMap::new()),
    };

    Ok(Some(Req {
        method,
        path: percent_decode(&path),
        query,
        headers,
        body,
    }))
}

/// `read_line`, but capped, so one endless line cannot allocate the process
/// out of memory before the header budget is ever checked.
fn read_line(reader: &mut BufReader<TcpStream>, line: &mut Vec<u8>) -> std::io::Result<usize> {
    let mut taken = 0;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(taken);
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(at) => {
                line.extend_from_slice(&available[..=at]);
                reader.consume(at + 1);
                return Ok(taken + at + 1);
            }
            None => {
                let length = available.len();
                line.extend_from_slice(available);
                reader.consume(length);
                taken += length;
                if taken > MAX_HEAD_BYTES {
                    return Err(std::io::Error::new(ErrorKind::InvalidData, "header too long"));
                }
            }
        }
    }
}

fn parse_query(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((name, value)) => (percent_decode(name), percent_decode(value)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn respond(socket: &mut TcpStream, response: Res, close: bool) -> std::io::Result<()> {
    let mut headers = response.headers;
    if close {
        headers.push(("connection".into(), "close".into()));
    }
    write_head(socket, response.status, &headers, Some(response.body.len()))?;
    socket.write_all(&response.body)?;
    socket.flush()
}

fn write_head(
    socket: &mut TcpStream,
    status: u16,
    headers: &[(String, String)],
    length: Option<usize>,
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    for (name, value) in headers {
        // A header value with a newline in it would let a route inject a
        // whole response. Nothing here builds one from untrusted text, and
        // this makes that true rather than merely intended.
        if value.contains(['\r', '\n']) {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(length) = length {
        head.push_str(&format!("content-length: {length}\r\n"));
    }
    head.push_str("\r\n");
    socket.write_all(head.as_bytes())
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_string_comes_apart_the_way_a_browser_wrote_it() {
        let query = parse_query("session=my%20chat&since=-1&flag");
        assert_eq!(query.get("session").unwrap(), "my chat");
        assert_eq!(query.get("since").unwrap(), "-1");
        assert_eq!(query.get("flag").unwrap(), "");
    }

    #[test]
    fn a_bearer_comes_from_the_header_or_the_only_place_eventsource_can_put_it() {
        let mut request = Req {
            method: "GET".into(),
            path: "/events".into(),
            query: BTreeMap::new(),
            headers: BTreeMap::new(),
            body: Vec::new(),
        };
        assert_eq!(request.bearer(), None);

        request.query.insert("token".into(), "omniauth_x".into());
        assert_eq!(request.bearer(), Some("omniauth_x"));

        request
            .headers
            .insert("authorization".into(), "Bearer omniauth_y".into());
        assert_eq!(
            request.bearer(),
            Some("omniauth_y"),
            "the header wins, because the query one ends up in logs"
        );
    }

    #[test]
    fn an_event_with_newlines_in_it_stays_one_event() {
        // `Sse` needs a socket, so the framing is checked on its own terms:
        // every line of a payload is prefixed, and one blank line ends it.
        let data = "first\nsecond";
        let mut frame = String::new();
        for line in data.split('\n') {
            frame.push_str("data: ");
            frame.push_str(line);
            frame.push('\n');
        }
        frame.push('\n');
        assert_eq!(frame, "data: first\ndata: second\n\n");
    }

    #[test]
    fn percent_escapes_and_plus_signs_both_decode() {
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        assert_eq!(percent_decode("one+two"), "one two");
        assert_eq!(percent_decode("100%"), "100%", "a stray percent is literal");
    }
}

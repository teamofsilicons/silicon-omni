//! The bridge over a real socket.
//!
//! The unit tests know what each piece answers. These check the part that
//! only exists once bytes are involved: that a browser's request parses, that
//! a kept-alive connection serves more than one, and that an event stream is
//! framed the way an `EventSource` expects to read it.
//!
//! Nothing here starts a daemon. Every route exercised is one the bridge can
//! answer on its own, so the test is about the door and not about what is
//! behind it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use omni_web::api::Web;
use omni_web::bridge::Bridge;
use omni_web::http::{Answer, Req, Res, Server};
use omni_web::state::Doorman;
use serde_json::{Value, json};

struct Running {
    port: u16,
    stopper: omni_web::http::Stopper,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stopper.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn scratch() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "omni-web-door-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&path).expect("a scratch directory");
    path.join("web.json")
}

/// A bridge on a port the operating system picked, so tests never fight over
/// 1998 with each other or with a real one.
fn bridge() -> (Running, Arc<Web>) {
    let server = Server::bind(0).expect("a free port");
    let port = server.port;
    let doorman = Arc::new(Doorman::open(scratch(), port));
    let web = Arc::new(Web::new(doorman, Arc::new(Bridge::default())));
    let stopper = server.stopper();
    *web.stopper.lock().unwrap() = Some(stopper.clone());

    let handler = {
        let web = web.clone();
        Arc::new(move |request| Web::answer(&web, request))
    };
    let thread = std::thread::spawn(move || server.run(handler));
    wait_for(port);
    (
        Running {
            port,
            stopper,
            thread: Some(thread),
        },
        web,
    )
}

fn wait_for(port: u16) {
    for _ in 0..200 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("nothing came up on {port}");
}

fn open(port: u16) -> TcpStream {
    let socket = TcpStream::connect(("127.0.0.1", port)).expect("the bridge is listening");
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    socket
}

/// Write one request and read one response off a connection that stays open.
fn exchange(socket: &mut TcpStream, raw: &str) -> (u16, Vec<(String, String)>, String) {
    socket.write_all(raw.as_bytes()).unwrap();
    socket.flush().unwrap();
    read_response(socket)
}

fn read_response(socket: &mut TcpStream) -> (u16, Vec<(String, String)>, String) {
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let status: u16 = line.split_whitespace().nth(1).unwrap().parse().unwrap();

    let mut headers = Vec::new();
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap() == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let (name, value) = (name.trim().to_lowercase(), value.trim().to_string());
            if name == "content-length" {
                length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body).unwrap();
    }
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

fn get(port: u16, path: &str, extra: &str) -> (u16, Value) {
    let mut socket = open(port);
    let (status, _, body) = exchange(
        &mut socket,
        &format!(
            "GET {path} HTTP/1.1\r\nhost: localhost:{port}\r\nconnection: close\r\n{extra}\r\n"
        ),
    );
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

fn post(port: u16, path: &str, extra: &str, body: &Value) -> (u16, Value) {
    let payload = serde_json::to_string(body).unwrap();
    let mut socket = open(port);
    let (status, _, answer) = exchange(
        &mut socket,
        &format!(
            "POST {path} HTTP/1.1\r\nhost: localhost:{port}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n{extra}\r\n{payload}",
            payload.len()
        ),
    );
    (status, serde_json::from_str(&answer).unwrap_or(Value::Null))
}

#[test]
fn a_website_can_find_the_bridge_by_knocking() {
    let (running, _web) = bridge();
    let (status, hello) = get(running.port, "/omni", "");
    assert_eq!(status, 200);
    assert_eq!(hello["omni"], "web");
    assert_eq!(hello["port"], running.port);
    assert_eq!(hello["protocol"], silicon_omni::PROTOCOL);
}

#[test]
fn the_whole_pairing_dance_works_over_a_socket() {
    let (running, web) = bridge();
    let control = web.doorman.control_token();

    // The terminal asks for a code.
    let (status, minted) = post(
        running.port,
        "/control/connect",
        &format!("authorization: Bearer {control}\r\n"),
        &json!({}),
    );
    assert_eq!(status, 200);
    let code = minted["code"].as_str().unwrap().to_string();
    assert!(code.ends_with(&format!("-{}", running.port)), "{code}");

    // The website spends it.
    let (status, paired) = post(
        running.port,
        "/connect",
        "origin: https://example.com\r\n",
        &json!({"code": code, "name": "Example"}),
    );
    assert_eq!(status, 200);
    let token = paired["omniauth"].as_str().unwrap().to_string();

    // And is somebody afterwards.
    let (status, me) = get(
        running.port,
        "/whoami",
        &format!("origin: https://example.com\r\nauthorization: Bearer {token}\r\n"),
    );
    assert_eq!(status, 200);
    assert_eq!(me["grant"]["name"], "Example");
    assert_eq!(me["grant"]["origin"], "https://example.com");

    // The same code a second time is worth nothing.
    let (status, _) = post(
        running.port,
        "/connect",
        "origin: https://elsewhere.example\r\n",
        &json!({"code": code}),
    );
    assert_eq!(status, 401);
}

#[test]
fn a_token_does_not_travel_to_another_site() {
    let (running, web) = bridge();
    let code = web.doorman.mint_code();
    let (token, _) = web
        .doorman
        .redeem(&code, "Example", "https://example.com")
        .unwrap();

    let (status, refused) = get(
        running.port,
        "/sessions",
        &format!("origin: https://evil.example\r\nauthorization: Bearer {token}\r\n"),
    );
    assert_eq!(status, 403);
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("https://example.com")
    );
}

#[test]
fn a_page_that_reached_us_under_another_name_gets_nothing() {
    let (running, _web) = bridge();
    let mut socket = open(running.port);
    let (status, _, body) = exchange(
        &mut socket,
        &format!(
            "GET /omni HTTP/1.1\r\nhost: totally.evil.example:{}\r\nconnection: close\r\n\r\n",
            running.port
        ),
    );
    assert_eq!(status, 403);
    assert!(body.contains("localhost"));
}

#[test]
fn one_connection_serves_more_than_one_request() {
    let (running, _web) = bridge();
    let mut socket = open(running.port);
    for _ in 0..3 {
        let (status, _, body) = exchange(
            &mut socket,
            &format!("GET /omni HTTP/1.1\r\nhost: localhost:{}\r\n\r\n", running.port),
        );
        assert_eq!(status, 200);
        assert!(body.contains("\"omni\":\"web\""));
    }
}

#[test]
fn a_preflight_comes_back_with_what_the_fetch_after_it_needs() {
    let (running, _web) = bridge();
    let mut socket = open(running.port);
    let (status, headers, _) = exchange(
        &mut socket,
        &format!(
            "OPTIONS /send HTTP/1.1\r\nhost: localhost:{}\r\norigin: https://example.com\r\naccess-control-request-method: POST\r\nconnection: close\r\n\r\n",
            running.port
        ),
    );
    assert_eq!(status, 204);
    let headers: std::collections::HashMap<_, _> = headers.into_iter().collect();
    assert_eq!(
        headers.get("access-control-allow-origin").map(String::as_str),
        Some("https://example.com")
    );
    assert!(!headers.contains_key("access-control-allow-credentials"));
}

#[test]
fn a_body_bigger_than_the_ceiling_is_refused_before_it_is_read() {
    let (running, _web) = bridge();
    let mut socket = open(running.port);
    let (status, _, body) = exchange(
        &mut socket,
        &format!(
            "POST /connect HTTP/1.1\r\nhost: localhost:{}\r\ncontent-length: 999999999\r\nconnection: close\r\n\r\n",
            running.port
        ),
    );
    assert_eq!(status, 413);
    assert!(body.contains("at most"));
}

#[test]
fn a_route_that_does_not_exist_says_which_ones_do() {
    let (running, web) = bridge();
    let code = web.doorman.mint_code();
    let (token, _) = web.doorman.redeem(&code, "script", "").unwrap();
    let (status, answer) = get(
        running.port,
        "/nowhere",
        &format!("authorization: Bearer {token}\r\n"),
    );
    assert_eq!(status, 404);
    assert!(answer["error"].as_str().unwrap().contains("/events"));
}

#[test]
fn an_event_stream_is_framed_the_way_a_browser_reads_it() {
    // The framing belongs to the HTTP layer, so it is checked there — with a
    // handler that streams three things and stops, and no omni underneath.
    let server = Server::bind(0).expect("a free port");
    let port = server.port;
    let stopper = server.stopper();
    let thread = std::thread::spawn(move || {
        server.run(Arc::new(|_request: Req| {
            Answer::Stream(
                vec![("content-type".into(), "text/event-stream".into())],
                Box::new(|sse| {
                    let _ = sse.keepalive();
                    let _ = sse.event(Some(7), "frame", r#"{"a":1}"#);
                    let _ = sse.event(None, "end", "over\nand out");
                }),
            )
        }))
    });
    wait_for(port);

    let mut socket = open(port);
    socket
        .write_all(format!("GET /events HTTP/1.1\r\nhost: localhost:{port}\r\n\r\n").as_bytes())
        .unwrap();
    let mut whole = String::new();
    socket.read_to_string(&mut whole).unwrap();

    let (head, body) = whole.split_once("\r\n\r\n").expect("headers then a stream");
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(head.to_lowercase().contains("text/event-stream"));
    assert_eq!(
        body,
        ": ping\n\nid: 7\nevent: frame\ndata: {\"a\":1}\n\nevent: end\ndata: over\ndata: and out\n\n",
        "a multi-line payload is several data lines and still one event"
    );

    stopper.stop();
    let _ = thread.join();
}

#[test]
fn a_handler_can_still_answer_plainly_on_the_same_server() {
    let server = Server::bind(0).expect("a free port");
    let port = server.port;
    let stopper = server.stopper();
    let thread = std::thread::spawn(move || {
        server.run(Arc::new(|request: Req| {
            Answer::Done(Res::json(200, &json!({"saw": request.query("q")})))
        }))
    });
    wait_for(port);

    let mut socket = open(port);
    let (status, _, body) = exchange(
        &mut socket,
        &format!("GET /echo?q=hello%20there HTTP/1.1\r\nhost: localhost:{port}\r\nconnection: close\r\n\r\n"),
    );
    assert_eq!(status, 200);
    assert_eq!(body, r#"{"saw":"hello there"}"#);

    stopper.stop();
    let _ = thread.join();
}

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use serde_json::{Value, json};

#[test]
fn silicon_setup_login_status_and_logout_keep_credentials_private() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let backend = format!("http://{}", listener.local_addr().unwrap());
    let home = std::env::temp_dir().join(format!("omni-cli-auth-{}", std::process::id()));
    let signed_in = json!({"authenticated":true,"account":{"id":"test-user","display_name":"Test"},"is_admin":false,"expires_at":"2099-01-01T00:00:00Z"});
    let mut exchange = signed_in.clone();
    exchange["session_token"] = json!("private-session-token");
    let server = thread::spawn(move || {
        for (method, path, body, authenticated) in [
            ("POST", "/api/auth/slt", exchange, false),
            ("GET", "/api/auth/session", signed_in, true),
            (
                "POST",
                "/api/auth/logout",
                json!({"authenticated":false}),
                true,
            ),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut input = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            input.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), format!("{method} {path} HTTP/1.1"));
            let mut headers = String::new();
            let mut length = 0;
            loop {
                let mut header = String::new();
                input.read_line(&mut header).unwrap();
                if header == "\r\n" {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
                headers.push_str(&header.to_ascii_lowercase());
            }
            assert_eq!(
                headers.contains("authorization: bearer private-session-token"),
                authenticated
            );
            if length > 0 {
                let mut bytes = vec![0; length];
                input.read_exact(&mut bytes).unwrap();
                if path == "/api/auth/slt" {
                    assert_eq!(
                        serde_json::from_slice::<Value>(&bytes).unwrap(),
                        json!({"slt":"private-slt"})
                    );
                }
            }
            let text = body.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{text}", text.len()).unwrap();
        }
    });

    let command = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_omni"));
        command
            .args(args)
            .env("OMNI_HOME", &home)
            .env("OMNI_BACKEND", &backend)
            .env_remove("OMNI_REGISTRY")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    };
    let checked = |output: std::process::Output| {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for bytes in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains("private-session-token"));
            assert!(!text.contains("private-slt"));
        }
        output.stdout
    };
    let discovery = checked(
        command(&["accounts", "--json"])
            .env("OMNI_BACKEND", "http://localhost:1")
            .output()
            .unwrap(),
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&discovery).unwrap()["app_id"],
        "omni"
    );

    let mut child = command(&["login", "--slt-stdin", "--json"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"private-slt\n")
        .unwrap();
    let login = checked(child.wait_with_output().unwrap());
    assert_eq!(
        serde_json::from_slice::<Value>(&login).unwrap()["authenticated"],
        true
    );
    assert_eq!(
        fs::metadata(home.join("accounts.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    // A different backend origin must not receive the saved session token.
    let status = checked(
        command(&["login", "status", "--json"])
            .env("OMNI_BACKEND", "http://localhost:1")
            .output()
            .unwrap(),
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&status).unwrap()["authenticated"],
        false
    );
    let status = checked(command(&["login", "status", "--json"]).output().unwrap());
    assert_eq!(
        serde_json::from_slice::<Value>(&status).unwrap()["account"]["id"],
        "test-user"
    );

    checked(command(&["logout"]).output().unwrap());
    let status = checked(command(&["login", "status", "--json"]).output().unwrap());
    assert_eq!(
        serde_json::from_slice::<Value>(&status).unwrap()["authenticated"],
        false
    );
    assert!(!home.join("omnid.sock").exists());
    server.join().unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn an_old_status_response_cannot_erase_a_concurrent_successful_login() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let backend = format!("http://{}", listener.local_addr().unwrap());
    let home = std::env::temp_dir().join(format!("omni-cli-auth-race-{}", std::process::id()));
    fs::create_dir_all(&home).unwrap();
    let path = home.join("accounts.json");
    fs::write(
        &path,
        json!({&backend: {"session_token":"expired-token","expires_at":"2000-01-01T00:00:00Z"}})
            .to_string(),
    )
    .unwrap();
    let (status_started, wait_for_status) = mpsc::channel();
    let (finish_status, wait_for_login) = mpsc::channel();
    let server = thread::spawn(move || {
        let read = |stream: &std::net::TcpStream| {
            let mut input = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            input.read_line(&mut line).unwrap();
            let mut length = 0;
            loop {
                let mut header = String::new();
                input.read_line(&mut header).unwrap();
                if header == "\r\n" {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            input.read_exact(&mut vec![0; length]).unwrap();
            line
        };
        let reply = |mut stream: std::net::TcpStream, value: Value| {
            let body = value.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        };
        let (status, _) = listener.accept().unwrap();
        assert!(read(&status).starts_with("GET /api/auth/session "));
        status_started.send(()).unwrap();
        let (login, _) = listener.accept().unwrap();
        assert!(read(&login).starts_with("POST /api/auth/slt "));
        reply(
            login,
            json!({"authenticated":true,"session_token":"new-session","expires_at":"2099-01-01T00:00:00Z","account":{"id":"test-user"}}),
        );
        wait_for_login
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        reply(status, json!({"authenticated":false}));
    });
    let command = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_omni"));
        command
            .args(args)
            .env("OMNI_HOME", &home)
            .env("OMNI_BACKEND", &backend)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    };
    let status = command(&["login", "status", "--json"]).spawn().unwrap();
    wait_for_status
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let login = command(&["login", "--slt", "test-slt", "--json"])
        .output()
        .unwrap();
    assert!(login.status.success());
    finish_status.send(()).unwrap();
    assert!(status.wait_with_output().unwrap().status.success());
    let saved: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved[&backend]["session_token"], "new-session");
    server.join().unwrap();
    fs::remove_dir_all(home).unwrap();
}

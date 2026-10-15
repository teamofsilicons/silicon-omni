use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::thread;

use serde_json::{Value, json};

#[test]
fn best_discovers_conditional_models_without_requiring_a_login() {
    let root = std::env::temp_dir().join(format!("omni-cli-catalog-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let executable = root.join("agy");
    let marker = root.join("probed");
    fs::write(&executable, "#!/bin/sh\nprintf called > \"$OMNI_TEST_PROBE_MARKER\"\nprintf 'gemini-4-argon-high\\tGemini 4 Argon\\n'\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let backend = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for expected in ["argon", "unknown", "excluded", "categories", "models"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let body = match expected {
                "categories" => {
                    assert!(line.starts_with("GET /categories "), "{line}");
                    json!({"categories":[]})
                }
                "models" => {
                    assert!(line.starts_with("GET /models "), "{line}");
                    json!({"models":[]})
                }
                _ => {
                    assert!(line.contains("available_models="), "{line}");
                    match expected {
                        "argon" => assert!(line.contains("gemini-4-argon-high"), "{line}"),
                        "unknown" => {
                            assert!(line.contains("%22antigravity-cli%22%3A%5B%5D"), "{line}")
                        }
                        _ => assert!(line.contains("available_models=%7B%7D"), "{line}"),
                    }
                    json!({"pick":{"provider":"antigravity-cli","model":if expected == "argon" {"gemini-4-argon-high"} else {"gemini-3.8-flash-high"}},"shortlist":[]})
                }
            };
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" {
                    break;
                }
                assert!(!header.to_ascii_lowercase().starts_with("authorization:"));
            }
            let body = body.to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_omni"))
            .args(args)
            .env("OMNI_HOME", root.join("home"))
            .env("OMNI_BACKEND", &backend)
            .env_remove("OMNI_REGISTRY")
            .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
            .env("OMNI_TEST_PROBE_MARKER", &marker)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    assert_eq!(
        run(&["best", "design", "--json"])["pick"]["model"],
        "gemini-4-argon-high"
    );
    assert!(marker.exists());
    fs::write(&executable, "#!/bin/sh\nexit 1\n").unwrap();
    assert_eq!(
        run(&["best", "design", "--providers", "antigravity-cli", "--json"])["pick"]["model"],
        "gemini-3.8-flash-high"
    );
    fs::remove_file(&marker).unwrap();
    fs::write(
        &executable,
        "#!/bin/sh\nprintf called > \"$OMNI_TEST_PROBE_MARKER\"\nexit 1\n",
    )
    .unwrap();
    run(&[
        "best",
        "writing",
        "--providers",
        "claude-code-cli",
        "--json",
    ]);
    run(&["categories", "--json"]);
    run(&["models", "--json"]);
    assert!(!marker.exists(), "unrelated public queries invoked Agy");
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::thread;

use omni_core::choose::{self, Ask, Pick};
use omni_core::providers;
use serde_json::json;

const AGY: &str = "antigravity-cli";
const ARGON: &str = "gemini-4-argon-high";
const FLASH: &str = "gemini-3.8-flash-high";

fn inventory(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

// Runs in a child so PATH and registry overrides cannot affect parallel tests.
#[test]
fn inventory_routing_child() {
    let Some(root) = std::env::var_os("OMNI_TEST_INVENTORY_ROOT") else {
        return;
    };
    let executable = Path::new(&root).join("agy");
    let providers = vec![AGY.to_string()];
    let ask = Ask::key("design");
    inventory(
        &executable,
        "printf 'gemini-4-argon-high\\tGemini 4 Argon\\n'",
    );
    assert_eq!(providers::account(AGY).unwrap().probe(), "authenticated");
    assert_eq!(choose::resolve(&ask, &providers).unwrap().model, ARGON);

    // A second resolve uses both caches; it must not invoke this failed probe.
    inventory(&executable, "exit 1");
    assert_eq!(choose::resolve(&ask, &providers).unwrap().model, ARGON);

    // Explicit model requests neither inspect availability nor contact the registry.
    assert_eq!(
        choose::resolve(&Ask::model(ARGON).from(AGY), &providers)
            .unwrap()
            .model,
        ARGON
    );

    providers::forget(AGY);
    assert_eq!(choose::resolve(&ask, &providers).unwrap().model, FLASH);

    // Successful discovery of a different inventory gets a distinct answer too.
    inventory(
        &executable,
        "printf 'gemini-3.8-flash-high\\tGemini 3.8 Flash\\n'",
    );
    providers::forget(AGY);
    assert_eq!(choose::resolve(&ask, &providers).unwrap().model, FLASH);

    // Losing access must never fall back to a cached Argon answer, even offline.
    inventory(&executable, "exit 1");
    providers::forget(AGY);
    assert_eq!(choose::resolve(&ask, &providers).unwrap().model, FLASH);

    let pinned = Ask::key("writing");
    choose::pin(&pinned, &providers, &Pick::new(AGY, FLASH, ""), 60.0);
    assert_eq!(choose::resolve(&pinned, &providers).unwrap().model, FLASH);
}

#[test]
fn changing_or_losing_inventory_uses_a_different_recommendation_cache() {
    let root = std::env::temp_dir().join(format!("omni-availability-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let registry = format!("http://{}/choose.json", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for expected in [ARGON, "", FLASH] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(
                line.contains("available_models=%7B%22antigravity-cli%22%3A%5B"),
                "{line}"
            );
            if expected.is_empty() {
                assert!(line.contains("%5B%5D%7D"), "{line}");
                assert!(!line.contains(ARGON), "{line}");
            } else {
                assert!(line.contains(expected), "{line}");
            }
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" {
                    break;
                }
            }
            let model = if line.contains(ARGON) { ARGON } else { FLASH };
            let body = json!({"pick":{"provider":AGY,"model":model}}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "inventory_routing_child", "--nocapture"])
        .env("OMNI_TEST_INVENTORY_ROOT", &root)
        .env("OMNI_HOME", root.join("home"))
        .env("OMNI_REGISTRY", registry)
        .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

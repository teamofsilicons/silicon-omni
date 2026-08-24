use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn term_and_interrupt_take_the_graceful_cleanup_path() {
    for signal in [libc::SIGTERM, libc::SIGINT] {
        exercise(signal);
    }
}

fn exercise(signal: i32) {
    let home = std::env::temp_dir().join(format!(
        "omni-daemon-signal-{}-{signal}",
        std::process::id()
    ));
    let empty_path = home.join("empty-path");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&empty_path).unwrap();

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_omnid"))
        .env("OMNI_HOME", &home)
        .env("PATH", &empty_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let log = home.join("omnid.log");
    let socket = home.join("omnid.sock");
    let ready_by = Instant::now() + Duration::from_secs(10);
    let ready = loop {
        if std::fs::read_to_string(&log).is_ok_and(|text| text.contains(" listening on ")) {
            break true;
        }
        if daemon.try_wait().unwrap().is_some() || Instant::now() >= ready_by {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !ready {
        let _ = daemon.kill();
        let _ = daemon.wait();
        let _ = std::fs::remove_dir_all(&home);
        panic!("daemon did not reach its signal-ready accept loop");
    }

    assert_eq!(unsafe { libc::kill(daemon.id() as i32, signal) }, 0);
    let stopped_by = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= stopped_by {
            let _ = daemon.kill();
            let _ = daemon.wait();
            let _ = std::fs::remove_dir_all(&home);
            panic!("daemon did not stop after signal {signal}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    assert!(status.success(), "signal {signal} bypassed normal return");
    assert!(!socket.exists(), "graceful cleanup left the socket behind");
    let _ = std::fs::remove_dir_all(home);
}

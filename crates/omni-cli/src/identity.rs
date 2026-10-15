//! Silicon account sessions are independent of inference-provider logins.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use omni_core::{choose, shared::paths};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{CliError, Output, Result, no_args, output, print_value, take_flag, take_value};

#[derive(Serialize, Deserialize)]
struct Session {
    session_token: String,
    expires_at: String,
}

type Sessions = BTreeMap<String, Session>;

pub fn accounts(mut args: Vec<String>) -> Result<()> {
    output(&mut args, false)?;
    no_args("accounts", args)?;
    print_value(&json!({
        "app_id": "omni",
        "accounts_url": "https://accounts.teamofsilicons.com",
        "base_url": origin()?,
        "client": "silicon-omni",
        "version": env!("CARGO_PKG_VERSION"),
    }));
    Ok(())
}

pub fn admin(args: Vec<String>) -> Result<()> {
    no_args("admin", args)?;
    let (_, response) = request(&origin()?, "/accounts", None, None)?;
    checked(&response)?;
    let url = response
        .get("admin_url")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError("backend did not provide an admin URL".into()))?;
    println!("{url}");
    Ok(())
}

pub fn login(mut args: Vec<String>) -> Result<()> {
    let format = output(&mut args, false)?;
    if args.first().is_some_and(|a| a == "status") {
        args.remove(0);
        no_args("login status", args)?;
        return status(format);
    }
    let slt = take_value(&mut args, "--slt")?;
    let stdin = take_flag(&mut args, "--slt-stdin");
    no_args("login", args)?;
    if slt.is_some() && stdin {
        return Err(CliError("use either --slt or --slt-stdin".into()));
    }
    let slt = if stdin {
        let mut text = String::new();
        io::stdin().take(65_537).read_to_string(&mut text)?;
        if text.len() > 65_536 {
            return Err(CliError("SLT input is too large".into()));
        }
        Some(text.trim().to_string())
    } else {
        slt
    };
    let backend = origin()?;
    let response = match slt {
        Some(slt) => {
            if slt.trim().is_empty() {
                return Err(CliError("SLT must not be empty".into()));
            }
            let (_, response) =
                request(&backend, "/api/auth/slt", Some(&json!({"slt": slt})), None)?;
            checked(&response)?;
            response
        }
        None => device_login(&backend)?,
    };
    let session = session_from(&response)?;
    let path = paths::home().join("accounts.json");
    update_sessions(&path, |sessions| {
        sessions.insert(backend, session);
        true
    })?;
    show_status(&response, format);
    Ok(())
}

fn status(format: Output) -> Result<()> {
    let backend = origin()?;
    let path = paths::home().join("accounts.json");
    let sessions = load(&path)?;
    let Some(session) = sessions.get(&backend) else {
        show_status(&json!({"authenticated": false}), format);
        return Ok(());
    };
    let (_, response) = request(
        &backend,
        "/api/auth/session",
        None,
        Some(&session.session_token),
    )?;
    checked(&response)?;
    if response
        .get("authenticated")
        .and_then(Value::as_bool)
        .is_none()
    {
        return Err(CliError("backend returned an invalid login status".into()));
    }
    if response["authenticated"] == false {
        remove_session(&path, &backend, &session.session_token)?;
    }
    show_status(&response, format);
    Ok(())
}

pub fn logout(args: Vec<String>) -> Result<()> {
    no_args("logout", args)?;
    let backend = origin()?;
    let path = paths::home().join("accounts.json");
    let sessions = load(&path)?;
    if let Some(session) = sessions.get(&backend) {
        let (_, response) = request(
            &backend,
            "/api/auth/logout",
            Some(&json!({})),
            Some(&session.session_token),
        )?;
        checked(&response)?;
        remove_session(&path, &backend, &session.session_token)?;
    }
    println!("Signed out.");
    Ok(())
}

fn device_login(backend: &str) -> Result<Value> {
    let (_, device) = request(backend, "/api/auth/device", Some(&json!({})), None)?;
    checked(&device)?;
    let required = |key: &str| {
        device
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| CliError(format!("backend did not provide {key}")))
    };
    let code = required("device_code")?;
    let user_code = required("user_code")?;
    let url = device
        .get("verification_uri_complete")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(Ok)
        .unwrap_or_else(|| required("verification_uri"))?;
    let expires = device
        .get("expires_in")
        .and_then(Value::as_u64)
        .filter(|n| (1..=3600).contains(n))
        .ok_or_else(|| CliError("backend returned an invalid device-code lifetime".into()))?;
    let mut interval = device
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .max(1);
    eprintln!("Open {url}\nEnter code: {user_code}\nWaiting for Silicon Accounts sign-in…");
    let deadline = Instant::now() + Duration::from_secs(expires);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CliError(
                "device login expired; run `omni login` again".into(),
            ));
        }
        std::thread::sleep(Duration::from_secs(interval).min(remaining));
        if Instant::now() >= deadline {
            return Err(CliError(
                "device login expired; run `omni login` again".into(),
            ));
        }
        let (_, response) = request(
            backend,
            "/api/auth/token",
            Some(&json!({"device_code": code})),
            None,
        )?;
        match error_code(&response) {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = interval.saturating_add(5);
                continue;
            }
            _ => checked(&response)?,
        }
        return Ok(response);
    }
}

/// Keep credential-bearing requests on one exact origin, with no redirects.
fn origin() -> Result<String> {
    origin_for(&choose::backend())
}

fn origin_for(base: &str) -> Result<String> {
    let uri = base
        .parse::<ureq::http::Uri>()
        .map_err(|_| CliError("OMNI_BACKEND must be an HTTP(S) origin".into()))?;
    let authority = uri
        .authority()
        .ok_or_else(|| CliError("OMNI_BACKEND needs a host".into()))?;
    let scheme = uri.scheme_str().unwrap_or("");
    let host = uri.host().unwrap_or("");
    let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    if authority.as_str().contains('@')
        || uri.query().is_some()
        || !matches!(uri.path(), "" | "/")
        || !(scheme == "https" || (scheme == "http" && local))
    {
        return Err(CliError(
            "OMNI_BACKEND must be an HTTPS origin (HTTP is allowed for localhost)".into(),
        ));
    }
    Ok(format!(
        "{scheme}://{}",
        authority.as_str().to_ascii_lowercase()
    ))
}

fn error_code(value: &Value) -> Option<&str> {
    value.get("error").and_then(|error| {
        error
            .as_str()
            .or_else(|| error.get("code").and_then(Value::as_str))
    })
}

fn checked(value: &Value) -> Result<()> {
    match error_code(value) {
        Some(code) => Err(CliError(format!("Silicon Accounts: {code}"))),
        None if value.get("error").is_some() => {
            Err(CliError("Silicon Accounts request failed".into()))
        }
        None => Ok(()),
    }
}

fn request(
    backend: &str,
    path: &str,
    body: Option<&Value>,
    token: Option<&str>,
) -> Result<(u16, Value)> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let url = format!("{backend}{path}");
    let response = if let Some(body) = body {
        let mut request = agent.post(&url).header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        request.send(body.to_string())
    } else {
        let mut request = agent.get(&url).header("accept", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        request.call()
    }
    .map_err(|_| CliError("cannot reach Silicon Accounts backend".into()))?;
    let mut response = response;
    let status = response.status();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|_| CliError("cannot read Silicon Accounts response".into()))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|_| CliError("Silicon Accounts backend returned invalid JSON".into()))?;
    if !status.is_success() && error_code(&value).is_none() {
        return Err(CliError(format!(
            "Silicon Accounts backend returned HTTP {status}"
        )));
    }
    Ok((status.as_u16(), value))
}

fn session_from(value: &Value) -> Result<Session> {
    if value["authenticated"] != true {
        return Err(CliError(
            "Silicon Accounts did not authenticate the login".into(),
        ));
    }
    let session: Session = serde_json::from_value(value.clone())
        .map_err(|_| CliError("backend returned an invalid login session".into()))?;
    if session.session_token.is_empty() || session.expires_at.is_empty() {
        return Err(CliError("backend returned an empty login session".into()));
    }
    Ok(session)
}

fn public_status(value: &Value) -> Value {
    let mut status = json!({"authenticated": value["authenticated"] == true});
    for field in ["account", "is_admin", "expires_at"] {
        if let Some(value) = value.get(field) {
            status[field] = value.clone();
        }
    }
    status
}

fn show_status(value: &Value, format: Output) {
    let status = public_status(value);
    if format == Output::Json {
        print_value(&status);
    } else if status["authenticated"] == true {
        let name = status["account"]["display_name"]
            .as_str()
            .or_else(|| status["account"]["id"].as_str())
            .unwrap_or("Silicon account");
        println!(
            "Signed in as {name}{}.",
            if status["is_admin"] == true {
                " (admin)"
            } else {
                ""
            }
        );
    } else {
        println!("Signed out.");
    }
}

fn load(path: &Path) -> Result<Sessions> {
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => return Err(err.into()),
    };
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::from_reader(file)
        .map_err(|_| CliError("cannot read saved Silicon Accounts sessions".into()))
}

/// Network requests happen before locking. Reload here so another process's
/// completed login cannot be overwritten by an older status/logout response.
fn update_sessions(path: &Path, update: impl FnOnce(&mut Sessions) -> bool) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError("invalid account-session path".into()))?;
    paths::ensure(parent)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path.with_extension("lock"))?;
    lock.set_permissions(fs::Permissions::from_mode(0o600))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut sessions = load(path)?;
    if update(&mut sessions) {
        save(path, &sessions)?;
    }
    Ok(())
}

fn remove_session(path: &Path, backend: &str, checked_token: &str) -> Result<()> {
    update_sessions(path, |sessions| {
        if sessions
            .get(backend)
            .is_some_and(|session| session.session_token == checked_token)
        {
            sessions.remove(backend);
            true
        } else {
            false
        }
    })
}

fn save(path: &Path, sessions: &Sessions) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError("invalid account-session path".into()))?;
    paths::ensure(parent)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(sessions)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_scoped_to_a_safe_exact_origin() {
        assert_eq!(
            origin_for("https://backend.omni.teamofsilicons.com/").unwrap(),
            "https://backend.omni.teamofsilicons.com"
        );
        assert_eq!(
            origin_for("http://localhost:3100").unwrap(),
            "http://localhost:3100"
        );
        for unsafe_origin in [
            "http://example.com",
            "https://user:password@example.com",
            "https://example.com/path",
            "https://example.com?q=x",
        ] {
            assert!(origin_for(unsafe_origin).is_err(), "{unsafe_origin}");
        }
    }

    #[test]
    fn status_never_contains_session_credentials() {
        let response = json!({"authenticated": true, "account": {"id":"a"}, "is_admin": false, "expires_at": "later", "session_token": "secret", "access_token": "also secret"});
        assert_eq!(
            public_status(&response),
            json!({"authenticated":true,"account":{"id":"a"},"is_admin":false,"expires_at":"later"})
        );
        assert_eq!(session_from(&response).unwrap().session_token, "secret");
        assert!(session_from(&json!({"authenticated": false})).is_err());
    }

    #[test]
    fn saved_sessions_are_private_and_do_not_cross_origins() {
        let directory =
            std::env::temp_dir().join(format!("omni-accounts-test-{}", std::process::id()));
        let path = directory.join("accounts.json");
        let sessions = BTreeMap::from([(
            "https://first.example".into(),
            Session {
                session_token: "private".into(),
                expires_at: "later".into(),
            },
        )]);
        save(&path, &sessions).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let loaded = load(&path).unwrap();
        assert_eq!(loaded["https://first.example"].session_token, "private");
        assert!(!loaded.contains_key("https://second.example"));
        fs::remove_dir_all(directory).unwrap();
    }
}

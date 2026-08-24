//! Is Claude Code here, are we signed in, and how much is left.
//!
//! `claude auth status` answers the first two as JSON. Usage comes from the
//! `get_usage` control request, which is free and needs no credentials of our
//! own — Claude Code asks on its own behalf and hands the answer back. That is
//! much better than reading somebody's keychain, and it keeps working when the
//! storage moves.

use std::time::Duration;

use serde_json::{Value, json};

use crate::providers::base::{AUTHENTICATED, Account, UNAUTHENTICATED};
use crate::providers::login::Login;
use crate::shared::clock;
use crate::shared::proc::output;

use super::control;

const WINDOWS: &[(&str, &str)] = &[("five_hour", "5h"), ("seven_day", "7d")];

pub struct Claude;

/// `{"utilization": 0-100, "resets_at": ...}` becomes omni's `{"used", "reset"}`.
fn window(entry: Option<&Value>) -> Value {
    let Some(entry) = entry.filter(|e| e.is_object()) else {
        return crate::providers::base::blank();
    };
    let used = entry
        .get("utilization")
        .and_then(Value::as_f64)
        .map(|used| (used / 100.0 * 10_000.0).round() / 10_000.0);
    json!({"used": used, "reset": clock::iso(entry.get("resets_at").unwrap_or(&Value::Null))})
}

impl Account for Claude {
    fn name(&self) -> &str {
        super::NAME
    }

    fn cli(&self) -> &str {
        super::CLI
    }

    fn probe(&self) -> String {
        let Some((code, out)) = output(&["claude", "auth", "status"], Duration::from_secs(20))
        else {
            return UNAUTHENTICATED.into();
        };
        if code != 0 {
            return UNAUTHENTICATED.into();
        }
        match serde_json::from_str::<Value>(&out) {
            Ok(data) if data.get("loggedIn").and_then(Value::as_bool) == Some(true) => {
                AUTHENTICATED.into()
            }
            _ => UNAUTHENTICATED.into(),
        }
    }

    /// Start `claude auth login` and hand back the URL it wants opened.
    fn start_auth(&self) -> String {
        Login::begin(
            super::NAME,
            &["claude", "auth", "login"],
            Duration::from_secs(45),
        )
    }

    /// Give the code (or redirect URL) back to the login, then re-check.
    fn finish_auth(&self, code: &str) -> String {
        Login::complete(super::NAME, code, Duration::from_secs(180));
        crate::providers::forget(super::NAME);
        self.probe()
    }

    fn limits(&self) -> Value {
        let usage =
            control::ask("get_usage", json!({}), Duration::from_secs(30)).unwrap_or(Value::Null);
        let buckets = &usage["rate_limits"];
        let mut out = serde_json::Map::new();
        for (long, short) in WINDOWS {
            out.insert((*short).into(), window(buckets.get(long)));
        }
        Value::Object(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percentage_becomes_a_fraction() {
        let seen = window(Some(
            &json!({"utilization": 24, "resets_at": "2026-01-02T03:04:05Z"}),
        ));
        assert_eq!(seen["used"], 0.24);
        assert_eq!(seen["reset"], "2026-01-02T03:04:05.000Z");
    }

    #[test]
    fn a_window_nobody_reported_is_null_not_zero() {
        let seen = window(None);
        assert!(seen["used"].is_null() && seen["reset"].is_null());
        let partial = window(Some(&json!({"resets_at": null})));
        assert!(partial["used"].is_null(), "no utilisation is not 0% used");
    }
}

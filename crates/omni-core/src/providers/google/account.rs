//! Is agy here, are we signed in, and how much is left.
//!
//! `agy models` is the safe auth probe: exit 0 means signed in. Never probe
//! with `-p` — unauthenticated print mode starts an interactive login and
//! blocks for a minute, spraying non-JSON over stdout.
//!
//! Quota comes from `agy -p "/usage"`, which the CLI answers itself: no model
//! is called and no quota is spent.

use std::time::Duration;

use serde_json::{Value, json};

use crate::providers::base::{AUTHENTICATED, Account, UNAUTHENTICATED, blank};
use crate::providers::login::Login;
use crate::shared::clock;
use crate::shared::proc::output;

const USAGE: &[&str] = &["agy", "-p", "/usage", "--output-format", "stream-json"];
const WINDOWS: &[(&str, &str)] = &[("5h", "5h"), ("weekly", "7d")];

pub struct Antigravity;

fn buckets(lines: &str) -> Vec<Value> {
    for line in lines.lines() {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if data.get("event").and_then(Value::as_str) != Some("command_result") {
            continue;
        }
        return data["command"]["data"]["groups"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .flat_map(|group| group["buckets"].as_array().cloned().unwrap_or_default())
            .collect();
    }
    Vec::new()
}

/// agy reports remaining, per model group. omni reports used, worst group first.
pub fn windows(lines: &str) -> Value {
    let mut out = json!({"5h": blank(), "7d": blank()});
    for bucket in buckets(lines) {
        let Some(name) = bucket["window"]
            .as_str()
            .and_then(|window| WINDOWS.iter().find(|(theirs, _)| *theirs == window))
            .map(|(_, ours)| *ours)
        else {
            continue;
        };
        let Some(remaining) = bucket["remaining_fraction"]
            .as_f64()
            .filter(|remaining| remaining.is_finite())
        else {
            // Missing quota is unknown quota, not a completely unused window.
            continue;
        };
        let remaining = remaining.clamp(0.0, 1.0);
        let used = ((1.0 - remaining) * 10_000.0).round() / 10_000.0;
        let worse = out[name]["used"].as_f64().is_none_or(|seen| used > seen);
        if worse {
            out[name] = json!({"used": used, "reset": clock::iso(&bucket["reset_time"])});
        }
    }
    out
}

impl Account for Antigravity {
    fn name(&self) -> &str {
        super::NAME
    }

    fn cli(&self) -> &str {
        super::CLI
    }

    fn probe(&self) -> String {
        match output(&["agy", "models"], Duration::from_secs(60)) {
            Some((0, _)) => AUTHENTICATED.into(),
            _ => UNAUTHENTICATED.into(),
        }
    }

    /// agy only offers a login as a side effect of running something.
    ///
    /// `/usage` is the cheapest thing to run: the CLI answers it itself, so the
    /// login is the only thing that actually happens.
    fn start_auth(&self) -> String {
        Login::begin(super::NAME, USAGE, Duration::from_secs(60))
    }

    fn finish_auth(&self, code: &str) -> String {
        Login::complete(super::NAME, code, Duration::from_secs(90));
        crate::providers::forget(super::NAME);
        self.probe()
    }

    fn limits(&self) -> Value {
        match output(USAGE, Duration::from_secs(90)) {
            Some((_, out)) => windows(&out),
            None => json!({"5h": blank(), "7d": blank()}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(groups: Value) -> String {
        json!({"event": "command_result", "command": {"data": {"groups": groups}}}).to_string()
    }

    #[test]
    fn remaining_becomes_used() {
        let seen = windows(&usage(json!([{
            "buckets": [{"window": "5h", "remaining_fraction": 0.76, "reset_time": 1767322800}]
        }])));
        assert_eq!(seen["5h"]["used"], 0.24);
        assert_eq!(seen["5h"]["reset"], "2026-01-02T03:00:00.000Z");
    }

    #[test]
    fn the_worst_group_is_the_one_reported() {
        let seen = windows(&usage(json!([
            {"buckets": [{"window": "5h", "remaining_fraction": 0.9}]},
            {"buckets": [{"window": "5h", "remaining_fraction": 0.1}]}
        ])));
        assert_eq!(
            seen["5h"]["used"], 0.9,
            "the group closest to the wall wins"
        );
    }

    #[test]
    fn agys_weekly_is_omnis_seven_day() {
        let seen = windows(&usage(json!([{
            "buckets": [{"window": "weekly", "remaining_fraction": 0.12}]
        }])));
        assert_eq!(seen["7d"]["used"], 0.88);
        assert!(seen["5h"]["used"].is_null());
    }

    #[test]
    fn nothing_parseable_is_nothing_known_rather_than_nothing_used() {
        let seen = windows("agy: command not found");
        assert!(seen["5h"]["used"].is_null() && seen["7d"]["used"].is_null());
    }

    #[test]
    fn missing_or_non_numeric_remaining_is_unknown_not_zero_used() {
        let seen = windows(&usage(json!([{
            "buckets": [
                {"window": "5h"},
                {"window": "weekly", "remaining_fraction": "0.5"}
            ]
        }])));
        assert!(seen["5h"]["used"].is_null());
        assert!(seen["7d"]["used"].is_null());
    }

    #[test]
    fn reported_fractions_are_clamped_to_a_real_window() {
        let seen = windows(&usage(json!([{
            "buckets": [
                {"window": "5h", "remaining_fraction": 1.4},
                {"window": "weekly", "remaining_fraction": -0.2}
            ]
        }])));
        assert_eq!(seen["5h"]["used"], 0.0);
        assert_eq!(seen["7d"]["used"], 1.0);
    }
}

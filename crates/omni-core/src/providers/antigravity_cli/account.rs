//! Is agy here, are we signed in, and how much is left.
//!
//! `agy models` is the safe auth probe: exit 0 means signed in. Never probe
//! with `-p` — unauthenticated print mode starts an interactive login and
//! blocks for a minute, spraying non-JSON over stdout.
//!
//! Quota comes from `agy -p "/usage"`, which the CLI answers itself: no model
//! is called and no quota is spent.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::providers::base::{AUTHENTICATED, Account, UNAUTHENTICATED, blank};
use crate::providers::login::Login;
use crate::shared::clock;
use crate::shared::proc::output;

const USAGE: &[&str] = &["agy", "-p", "/usage", "--output-format", "stream-json"];
const WINDOWS: &[(&str, &str)] = &[("5h", "5h"), ("weekly", "7d")];

pub struct Antigravity;

static MODELS: Mutex<Option<(Instant, BTreeSet<String>)>> = Mutex::new(None);

fn valid_model_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    id.len() <= 120
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn model_ids(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(id, _)| id.trim())
        .filter(|id| valid_model_id(id))
        .map(str::to_string)
        .collect()
}

/// A failed or unrecognized inventory grants access to no conditional models.
/// A zero max age forces a fresh probe for a public catalog query.
pub fn models(max_age: Duration) -> BTreeSet<String> {
    let mut memo = MODELS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((_, ids)) = memo.as_ref().filter(|(at, _)| at.elapsed() < max_age) {
        return ids.clone();
    }
    let ids = match output(&["agy", "models"], Duration::from_secs(15)) {
        Some((0, text)) => model_ids(&text),
        _ => BTreeSet::new(),
    };
    *memo = Some((Instant::now(), ids.clone()));
    ids
}

pub fn forget_models() {
    *MODELS.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

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
        let mut memo = MODELS.lock().unwrap_or_else(|p| p.into_inner());
        match output(&["agy", "models"], Duration::from_secs(60)) {
            Some((0, text)) => {
                *memo = Some((Instant::now(), model_ids(&text)));
                AUTHENTICATED.into()
            }
            _ => {
                *memo = None;
                UNAUTHENTICATED.into()
            }
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

    #[test]
    fn inventory_reads_only_tab_separated_ids_and_deduplicates_them() {
        let ids = model_ids(
            "Fetching available models...\ngemini-4-argon-high\tGemini 4 Argon\ngemini-3.8-flash-high\tGemini 3.8 Flash\ngemini-4-argon-high\tDuplicate\nnot an id\tWarning\n\tBlank\n",
        );
        assert_eq!(
            ids,
            BTreeSet::from(["gemini-3.8-flash-high".into(), "gemini-4-argon-high".into(),])
        );
        assert!(model_ids("Please sign in").is_empty());
        let invalid = format!(
            "-header\tHeader\ngemini!bad\tInvalid\nmodèle\tInvalid\n{}\tToo long\n",
            "a".repeat(121)
        );
        assert!(model_ids(&invalid).is_empty());
        assert!(valid_model_id(&"a".repeat(120)));
        assert!(valid_model_id("models/gemini_4:argon-high.v1"));
    }

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

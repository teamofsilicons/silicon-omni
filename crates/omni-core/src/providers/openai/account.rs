//! Is Codex here, are we signed in, and how much is left.
//!
//! Every answer lives behind the app-server protocol — and omni keeps one of
//! those warm, so all three questions are a single call against a process that
//! is already up. Login is the one that takes its time: the server runs the
//! browser callback itself, so omni hands back the URL and then waits for the
//! server to say it landed.

use std::time::Duration;

use serde_json::{Value, json};

use crate::providers::base::{AUTHENTICATED, Account, UNAUTHENTICATED, blank};
use crate::shared::clock;

use super::server::{self, Shared};

const MINUTES: &[(u64, &str)] = &[(300, "5h"), (10080, "7d")];
const COMPLETED: &str = "account/login/completed";

pub struct Codex;

/// Codex reports buckets by duration; omni reports 5h and 7d.
///
/// Never trust `primary` to be the short window — on some plans it is the
/// weekly one. Branch on `windowDurationMins`, always.
pub fn windows(payload: &Value) -> Value {
    let mut out = json!({"5h": blank(), "7d": blank()});
    let mut buckets = vec![payload["rateLimits"].clone()];
    if let Some(by_id) = payload["rateLimitsByLimitId"].as_object() {
        buckets.extend(by_id.values().cloned());
    }
    for bucket in buckets {
        for slot in ["primary", "secondary"] {
            let window = &bucket[slot];
            if !window.is_object() {
                continue;
            }
            let Some(name) = window["windowDurationMins"]
                .as_u64()
                .and_then(|mins| MINUTES.iter().find(|(m, _)| *m == mins))
                .map(|(_, name)| *name)
            else {
                continue;
            };
            // First report of a window wins: the same bucket can appear twice,
            // once globally and once per limit id, and they agree.
            if !out[name]["used"].is_null() {
                continue;
            }
            let used = window["usedPercent"]
                .as_f64()
                .map(|percent| (percent / 100.0 * 10_000.0).round() / 10_000.0);
            out[name] = json!({"used": used, "reset": clock::iso(&window["resetsAt"])});
        }
    }
    out
}

/// A warm server if there is one, otherwise one brought up and left warm — the
/// next question is then free.
fn connected() -> Option<std::sync::Arc<Shared>> {
    server::any_live().or_else(|| Shared::get(&super::runner::flags(&Default::default())).ok())
}

impl Account for Codex {
    fn name(&self) -> &str {
        super::NAME
    }

    fn cli(&self) -> &str {
        super::CLI
    }

    fn probe(&self) -> String {
        let Some(shared) = connected() else {
            return UNAUTHENTICATED.into();
        };
        match shared.try_call("account/read", json!({}), Duration::from_secs(30)) {
            Some(read) if !read["account"].is_null() => AUTHENTICATED.into(),
            _ => UNAUTHENTICATED.into(),
        }
    }

    /// Ask Codex to begin a ChatGPT login and hand back the URL to open.
    fn start_auth(&self) -> String {
        let Some(shared) = connected() else {
            return "codex is not installed, or would not start".into();
        };
        shared.forget(COMPLETED); // an old login must not answer for this one
        let Some(begun) = shared.try_call(
            "account/login/start",
            json!({"type": "chatgpt"}),
            Duration::from_secs(60),
        ) else {
            return "codex could not start a login".into();
        };
        for field in ["authUrl", "verificationUrl"] {
            if let Some(url) = begun[field].as_str() {
                return url.to_string();
            }
        }
        begun.to_string()
    }

    /// Codex runs the callback itself, so this waits rather than types.
    ///
    /// `code` is accepted for symmetry with the other providers and ignored.
    fn finish_auth(&self, _code: &str) -> String {
        if let Some(shared) = server::any_live() {
            for _ in 0..600 {
                if shared.last(COMPLETED).is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        crate::providers::forget(super::NAME);
        self.probe()
    }

    fn limits(&self) -> Value {
        let Some(shared) = connected() else {
            return json!(UNAUTHENTICATED);
        };
        // The server volunteers this as it changes, so a warm one has already
        // said it; only ask when it has not.
        let payload = shared.last("account/rateLimits/updated").or_else(|| {
            shared.try_call(
                "account/rateLimits/read",
                Value::Null,
                Duration::from_secs(30),
            )
        });
        match payload {
            Some(payload) => windows(&payload),
            None => json!({"5h": blank(), "7d": blank()}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_short_window_is_found_by_its_duration_not_its_slot() {
        // The weekly bucket in the "primary" slot: the trap this guards.
        let seen = windows(&json!({
            "rateLimits": {
                "primary": {"windowDurationMins": 10080, "usedPercent": 88},
                "secondary": {"windowDurationMins": 300, "usedPercent": 24}
            }
        }));
        assert_eq!(seen["7d"]["used"], 0.88);
        assert_eq!(seen["5h"]["used"], 0.24);
    }

    #[test]
    fn a_window_nobody_reported_stays_null() {
        let seen = windows(&json!({}));
        assert!(seen["5h"]["used"].is_null() && seen["7d"]["reset"].is_null());
    }

    #[test]
    fn a_bucket_with_an_unknown_duration_is_left_alone() {
        let seen = windows(&json!({
            "rateLimits": {"primary": {"windowDurationMins": 42, "usedPercent": 50}}
        }));
        assert!(seen["5h"]["used"].is_null() && seen["7d"]["used"].is_null());
    }

    #[test]
    fn the_same_window_reported_twice_is_not_counted_twice_over() {
        let seen = windows(&json!({
            "rateLimits": {"primary": {"windowDurationMins": 300, "usedPercent": 24}},
            "rateLimitsByLimitId": {
                "x": {"primary": {"windowDurationMins": 300, "usedPercent": 99}}
            }
        }));
        assert_eq!(seen["5h"]["used"], 0.24, "the first report stands");
    }

    #[test]
    fn a_reset_time_comes_back_in_omnis_one_shape() {
        let seen = windows(&json!({
            "rateLimits": {"primary": {"windowDurationMins": 300, "usedPercent": 1,
                                        "resetsAt": 1767322800}}
        }));
        assert_eq!(seen["5h"]["reset"], "2026-01-02T03:00:00.000Z");
    }
}

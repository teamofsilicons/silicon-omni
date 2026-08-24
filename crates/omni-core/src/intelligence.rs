//! One 0-10 dial that spans every provider you are logged into.
//!
//! A level maps to `{"provider", "model", "effort"}`, and omni hands those two
//! strings to the CLI verbatim. It does not interpret them, does not rank
//! anything, and does not know the name of a single model. Working out which
//! models belong on the dial happens at the registry, over a list kept in a
//! public repo, so a model released tomorrow needs no release of this package.
//!
//! The answer is kept under `~/.omni/cache` for an hour. Nothing is shipped in
//! the binary as a fallback: a model list baked into a release is a model list
//! that goes quietly stale, and a wrong recommendation is worse than an honest
//! refusal. A dial that was fetched once is reused even after it expires, so a
//! machine that has run before keeps working offline.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::shared::{clock, paths};

/// Where the dial comes from unless you say otherwise.
pub const REGISTRY: &str = "https://omni.teamofsilicons.com/intelligence.json";
/// Burst the cache after an hour.
pub const CACHE_TTL: f64 = 60.0 * 60.0;
/// After a failed fetch, sit still rather than retry every call.
pub const QUIET_TTL: f64 = 5.0 * 60.0;
/// Bump when a rung's shape changes; older caches are then ignored.
pub const VERSION: u32 = 2;
pub const LEVELS: i64 = 11;

/// One step of the dial: who runs it, with what, at what effort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rung {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub effort: String,
    #[serde(default)]
    pub level: i64,
}

impl Rung {
    pub fn new(provider: &str, model: &str, effort: &str) -> Self {
        Rung {
            provider: provider.into(),
            model: model.into(),
            effort: effort.into(),
            level: 0,
        }
    }
}

/// The registry has never been reached, so there is nothing to route to.
#[derive(Debug)]
pub struct NoDial(pub String);

impl std::fmt::Display for NoDial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for NoDial {}

/// The registry to ask. Read per call, so `OMNI_REGISTRY` can be set late.
pub fn remote() -> String {
    std::env::var("OMNI_REGISTRY")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| REGISTRY.to_string())
}

/// One dial per set of providers, named the same way on both sides.
pub fn key(providers: &[String]) -> String {
    let mut sorted: Vec<&str> = providers.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.join("+")
}

/// A dial is levels to rungs. Anything else is an envelope we are inside.
fn is_dial(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    !map.is_empty()
        && map.iter().all(|(level, rung)| {
            level.chars().all(|c| c.is_ascii_digit())
                && rung.get("model").is_some_and(Value::is_string)
        })
}

/// Take the dial out of whatever the registry wrapped it in.
///
/// Checked rather than assumed: an envelope that happens not to contain our key
/// must not be mistaken for a dial and cached as one.
fn unwrap(payload: &Value, name: &str) -> Option<Value> {
    let wrapped = payload.get("ladders").and_then(|l| l.get(name));
    [wrapped, payload.get("ladder"), Some(payload)]
        .into_iter()
        .flatten()
        .find(|candidate| is_dial(candidate))
        .cloned()
}

/// Ask the registry for the dial for exactly these providers.
pub fn fetch(name: &str, timeout: Duration) -> Option<Value> {
    let url = format!("{}?providers={}", remote(), urlencode(name));
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .new_agent();
    let body = agent
        .get(&url)
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    unwrap(&serde_json::from_str::<Value>(&body).ok()?, name)
}

fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn cache_file() -> PathBuf {
    paths::cache().join("intelligence.json")
}

/// The cache is read and written by one process (the daemon), but by several of
/// its threads: a lock here is cheaper and safer than losing an entry.
static CACHE: Mutex<()> = Mutex::new(());

fn cached() -> Map<String, Value> {
    let Ok(text) = std::fs::read_to_string(cache_file()) else {
        return Map::new();
    };
    let Ok(blob) = serde_json::from_str::<Map<String, Value>>(&text) else {
        return Map::new();
    };
    match blob.get("version").and_then(Value::as_u64) {
        Some(version) if version == VERSION as u64 => blob,
        _ => Map::new(),
    }
}

fn read_cache(name: &str, fresh_only: bool) -> Option<Value> {
    let blob = cached();
    let entry = blob.get(name)?;
    let age = clock::epoch() - entry.get("at").and_then(Value::as_f64).unwrap_or(0.0);
    let ttl = entry
        .get("ttl")
        .and_then(Value::as_f64)
        .unwrap_or(CACHE_TTL);
    if fresh_only && age > ttl {
        return None;
    }
    entry.get("levels").cloned()
}

pub fn write_cache(name: &str, levels: &Value, ttl: f64) {
    let _guard = CACHE.lock().unwrap_or_else(|poison| poison.into_inner());
    let _ = paths::ensure(&paths::cache());
    let mut blob = cached();
    blob.insert("version".into(), json!(VERSION));
    blob.insert(
        name.into(),
        json!({"at": clock::epoch(), "ttl": ttl, "levels": levels}),
    );
    let path = cache_file();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
    {
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        let _ = file.write_all(Value::Object(blob).to_string().as_bytes());
        let _ = file.sync_data();
    }
}

/// The dial for these providers, from the registry or from what it said last.
pub fn levels(providers: &[String]) -> Value {
    let name = key(providers);
    if let Some(fresh) = read_cache(&name, true) {
        return fresh;
    }
    if let Some(found) = fetch(&name, Duration::from_secs(5)) {
        write_cache(&name, &found, CACHE_TTL);
        return found;
    }
    // Unreachable. An old answer beats no answer, but stop asking for a while
    // rather than stalling on every call.
    match read_cache(&name, false) {
        Some(stale) => {
            write_cache(&name, &stale, QUIET_TTL);
            stale
        }
        None => json!({}),
    }
}

/// Levels 0-10 for the providers you have, 10 being the best you can reach.
pub fn table(providers: &[String]) -> BTreeMap<i64, Rung> {
    let mut out = BTreeMap::new();
    let allowed: BTreeSet<&str> = providers.iter().map(String::as_str).collect();
    if let Some(map) = levels(providers).as_object() {
        for (level, body) in map {
            let Ok(level) = level.parse::<i64>() else {
                continue;
            };
            if let Ok(mut rung) = serde_json::from_value::<Rung>(body.clone()) {
                // The registry chooses models and effort, never authority. A
                // malformed or compromised response cannot route a session to
                // a provider the caller did not make available.
                if !(0..LEVELS).contains(&level)
                    || !allowed.contains(rung.provider.as_str())
                    || rung.model.trim().is_empty()
                {
                    continue;
                }
                rung.level = level;
                out.insert(level, rung);
            }
        }
    }
    out
}

/// The single rung for `level`. Out-of-range levels clamp rather than fail.
pub fn resolve(level: i64, providers: &[String]) -> Result<Rung, NoDial> {
    let rungs = table(providers);
    if rungs.is_empty() {
        let mut named: Vec<&str> = providers.iter().map(String::as_str).collect();
        named.sort_unstable();
        return Err(NoDial(format!(
            "no dial for {named:?}: could not reach {} and nothing is cached",
            remote()
        )));
    }
    let wanted = level.clamp(0, LEVELS - 1);
    // Clamped to a level the dial may still not carry — take the nearest below,
    // then the nearest at all, rather than refusing a dial we actually have.
    let picked = rungs
        .range(..=wanted)
        .next_back()
        .or_else(|| rungs.iter().next())
        .map(|(_, rung)| rung.clone());
    picked.ok_or_else(|| NoDial("dial is empty".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_home;

    fn dial(rungs: &[(&str, &str, &str)]) -> Value {
        let mut out = Map::new();
        for (level, rung) in rungs.iter().enumerate() {
            out.insert(
                level.to_string(),
                json!({"provider": rung.0, "model": rung.1, "effort": rung.2}),
            );
        }
        Value::Object(out)
    }

    #[test]
    fn a_set_of_providers_has_one_name_whatever_order_you_say_it_in() {
        let a = key(&["google".into(), "claude".into()]);
        let b = key(&["claude".into(), "google".into(), "claude".into()]);
        assert_eq!(a, b);
        assert_eq!(a, "claude+google");
    }

    #[test]
    fn an_envelope_is_opened_and_a_stranger_is_not() {
        let inner = dial(&[("claude", "m", "")]);
        assert_eq!(
            unwrap(&json!({"ladders": {"claude": inner}}), "claude"),
            Some(dial(&[("claude", "m", "")]))
        );
        assert_eq!(unwrap(&json!({"ladders": {"other": {}}}), "claude"), None);
        assert_eq!(unwrap(&json!({"nothing": "useful"}), "claude"), None);
    }

    #[test]
    fn no_dial_is_an_honest_refusal_not_a_guess() {
        let _home = scratch_home("dial-none");
        let err = resolve(5, &["nobody".into()]).unwrap_err();
        assert!(err.to_string().contains("nothing is cached"), "{err}");
    }

    #[test]
    fn out_of_range_clamps_to_the_ends() {
        let _home = scratch_home("dial-clamp");
        let providers = vec!["a".to_string()];
        write_cache(
            &key(&providers),
            &dial(&[("a", "low", ""), ("a", "high", "")]),
            CACHE_TTL,
        );
        assert_eq!(resolve(-4, &providers).unwrap().model, "low");
        assert_eq!(resolve(99, &providers).unwrap().model, "high");
    }

    #[test]
    fn a_sparse_dial_takes_the_nearest_rung_below() {
        let _home = scratch_home("dial-sparse");
        let providers = vec!["a".to_string()];
        let mut sparse = Map::new();
        sparse.insert("0".into(), json!({"provider": "a", "model": "low"}));
        sparse.insert("10".into(), json!({"provider": "a", "model": "high"}));
        write_cache(&key(&providers), &Value::Object(sparse), CACHE_TTL);
        assert_eq!(resolve(7, &providers).unwrap().model, "low");
        assert_eq!(resolve(10, &providers).unwrap().model, "high");
    }

    #[test]
    fn a_registry_cannot_route_outside_the_requested_provider_set() {
        let _home = scratch_home("dial-provider-boundary");
        let providers = vec!["claude".to_string()];
        write_cache(
            &key(&providers),
            &json!({
                "0": {"provider": "openai", "model": "not-authorized"},
                "1": {"provider": "claude", "model": "allowed"},
                "12": {"provider": "claude", "model": "outside-the-dial"}
            }),
            CACHE_TTL,
        );

        let table = table(&providers);
        assert_eq!(table.len(), 1);
        assert_eq!(table[&1].model, "allowed");
    }

    #[test]
    fn a_stale_answer_beats_no_answer() {
        let _home = scratch_home("dial-stale");
        let providers = vec!["a".to_string()];
        write_cache(&key(&providers), &dial(&[("a", "remembered", "")]), -1.0);
        assert!(read_cache(&key(&providers), true).is_none(), "expired");
        // No registry is reachable under OMNI_REGISTRY, so this is the stale path.
        unsafe { std::env::set_var("OMNI_REGISTRY", "http://127.0.0.1:1/none") };
        assert_eq!(resolve(5, &providers).unwrap().model, "remembered");
        unsafe { std::env::remove_var("OMNI_REGISTRY") };
    }

    #[test]
    fn a_cache_from_an_older_shape_is_ignored() {
        let _home = scratch_home("dial-version");
        let _ = paths::ensure(&paths::cache());
        std::fs::write(
            cache_file(),
            json!({"version": 1, "a": {"levels": {}}}).to_string(),
        )
        .unwrap();
        assert!(read_cache("a", false).is_none());
    }
}

//! What should answer this turn.
//!
//! There are three ways to say it, and they exist because no one of them is
//! enough on its own:
//!
//! ```text
//! Ask::Key("code")                          a shortlist somebody chose
//! Ask::Intelligence { value: 7, .. }        the 0-10 dial
//! Ask::Model { model, effort, fast, .. }    you already know
//! ```
//!
//! A key and a number are answered by the registry, over a list kept in a
//! public repo, so a model released tomorrow needs no release of this package.
//! omni does not rank anything and knows the name of no model.
//!
//! A named model is answered here and never leaves the machine. You said what
//! you wanted; there is nothing to look up, and it works with no network.
//!
//! Registry answers are kept under `~/.omni/cache` for an hour. Nothing is
//! shipped in the binary as a fallback: a model list baked into a release is a
//! model list that goes quietly stale, and a wrong recommendation is worse than
//! an honest refusal. An answer fetched once is reused even after it expires,
//! so a machine that has run before keeps working offline.

use std::collections::BTreeSet;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::shared::{clock, paths};

/// Where answers come from unless you say otherwise.
pub const REGISTRY: &str = "https://omni.teamofsilicons.com/choose.json";
/// Burst the cache after an hour.
pub const CACHE_TTL: f64 = 60.0 * 60.0;
/// After a failed fetch, sit still rather than retry every call.
pub const QUIET_TTL: f64 = 5.0 * 60.0;
/// Bump when an answer's shape changes; older caches are then ignored.
pub const VERSION: u32 = 3;
/// The dial is 0-10 inclusive.
pub const INTELLIGENCE_VALUES: i64 = 11;

/// What omni was told to run: who runs it, with what, at what effort, hot or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Pick {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub effort: String,
    /// Ask the CLI for its faster tier. Ignored where there is no such thing.
    #[serde(default)]
    pub fast: bool,
}

impl Pick {
    pub fn new(provider: &str, model: &str, effort: &str) -> Self {
        Pick {
            provider: provider.into(),
            model: model.into(),
            effort: effort.into(),
            fast: false,
        }
    }
}

/// The three ways to say what should answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "how", rename_all = "lowercase")]
pub enum Ask {
    /// A shortlist somebody chose: `fast`, `code`, `design`, `research`, `cost`, `general`.
    Key { key: String },
    /// The left edge of a board, nought to ten.
    Intelligence {
        value: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bench: Option<String>,
    },
    /// You already know. Passed to the CLI verbatim.
    Model {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        model: String,
        #[serde(default)]
        effort: String,
        #[serde(default)]
        fast: bool,
    },
}

impl Default for Ask {
    /// No opinion expressed is the middle of the dial, as it always was.
    fn default() -> Self {
        Ask::Intelligence {
            value: 5,
            bench: None,
        }
    }
}

impl Ask {
    pub fn key(word: &str) -> Self {
        Ask::Key {
            key: word.trim().to_ascii_lowercase(),
        }
    }

    pub fn intelligence(value: i64) -> Self {
        Ask::Intelligence { value, bench: None }
    }

    /// A number, judged on a particular board.
    pub fn on(self, board: &str) -> Self {
        match self {
            Ask::Intelligence { value, .. } => Ask::Intelligence {
                value,
                bench: Some(board.trim().to_string()),
            },
            other => other,
        }
    }

    /// A model by name. Passed to the CLI verbatim.
    pub fn model(name: &str) -> Self {
        Ask::Model {
            provider: None,
            model: name.trim().to_string(),
            effort: String::new(),
            fast: false,
        }
    }

    /// Which CLI runs it. Only needed when more than one is available.
    pub fn from(self, name: &str) -> Self {
        match self {
            Ask::Model {
                model, effort, fast, ..
            } => Ask::Model {
                provider: Some(name.trim().to_string()),
                model,
                effort,
                fast,
            },
            other => other,
        }
    }

    pub fn effort(self, level: &str) -> Self {
        match self {
            Ask::Model {
                provider,
                model,
                fast,
                ..
            } => Ask::Model {
                provider,
                model,
                effort: level.trim().to_string(),
                fast,
            },
            other => other,
        }
    }

    /// Run it hot, where the CLI has such a thing.
    pub fn fast(self, hot: bool) -> Self {
        match self {
            Ask::Model {
                provider,
                model,
                effort,
                ..
            } => Ask::Model {
                provider,
                model,
                effort,
                fast: hot,
            },
            other => other,
        }
    }

    /// The query this ask becomes, and the name its answer is cached under.
    fn query(&self) -> Vec<(String, String)> {
        match self {
            Ask::Key { key } => vec![("key".into(), key.clone())],
            Ask::Intelligence { value, bench } => {
                let mut out = vec![(
                    "intelligence".into(),
                    (*value).clamp(0, INTELLIGENCE_VALUES - 1).to_string(),
                )];
                if let Some(bench) = bench {
                    out.push(("bench".into(), bench.clone()));
                }
                out
            }
            // never asked over the wire
            Ask::Model { .. } => Vec::new(),
        }
    }

}

impl std::fmt::Display for Ask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ask::Key { key } => write!(f, "{key}"),
            Ask::Intelligence { value, bench } => match bench {
                Some(bench) => write!(f, "{value} on {bench}"),
                None => write!(f, "{value}"),
            },
            Ask::Model {
                model, effort, fast, ..
            } => {
                write!(f, "{model}")?;
                if !effort.is_empty() {
                    write!(f, " {effort}")?;
                }
                if *fast {
                    write!(f, " fast")?;
                }
                Ok(())
            }
        }
    }
}

/// Nothing to route to: the registry has never been reached, or it refused.
#[derive(Debug)]
pub struct NoAnswer(pub String);

impl std::fmt::Display for NoAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for NoAnswer {}

/// The registry to ask. Read per call, so `OMNI_REGISTRY` can be set late.
pub fn remote() -> String {
    std::env::var("OMNI_REGISTRY")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| REGISTRY.to_string())
}

/// One answer per set of providers, named the same way on both sides.
pub fn providers_key(providers: &[String]) -> String {
    let mut sorted: Vec<&str> = providers.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.join("+")
}

/// What this exact question is cached under: the providers, and the ask.
pub fn cache_name(ask: &Ask, providers: &[String]) -> String {
    let mut parts = vec![providers_key(providers)];
    for (name, value) in ask.query() {
        parts.push(format!("{name}={value}"));
    }
    parts.join("|")
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

/// Take the pick out of whatever the registry wrapped it in.
///
/// Checked rather than assumed: an envelope that happens not to carry a pick
/// must not be mistaken for one and cached.
fn unwrap(payload: &Value) -> Option<Pick> {
    let pick = payload.get("pick")?;
    let parsed = serde_json::from_value::<Pick>(pick.clone()).ok()?;
    (!parsed.provider.trim().is_empty() && !parsed.model.trim().is_empty()).then_some(parsed)
}

/// Ask the registry this exact question.
pub fn fetch(ask: &Ask, providers: &[String], timeout: Duration) -> Option<Pick> {
    let mut query = vec![(
        "providers".to_string(),
        providers_key(providers),
    )];
    query.extend(ask.query());
    let tail: Vec<String> = query
        .iter()
        .map(|(name, value)| format!("{}={}", urlencode(name), urlencode(value)))
        .collect();
    let url = format!("{}?{}", remote(), tail.join("&"));

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
    unwrap(&serde_json::from_str::<Value>(&body).ok()?)
}

fn cache_file() -> PathBuf {
    paths::cache().join("choose.json")
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

fn read_cache(name: &str, fresh_only: bool) -> Option<Pick> {
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
    serde_json::from_value::<Pick>(entry.get("pick")?.clone()).ok()
}

/// Seed an answer so a question is never asked over the network.
///
/// This is how the test provider works, and it is deliberately the same path a
/// real answer takes: pinning writes a normal cache entry, so nothing about
/// resolution behaves differently under test than it does in the world.
pub fn pin(ask: &Ask, providers: &[String], pick: &Pick, ttl: f64) {
    write_cache(&cache_name(ask, providers), pick, ttl);
}

pub fn write_cache(name: &str, pick: &Pick, ttl: f64) {
    let _guard = CACHE.lock().unwrap_or_else(|poison| poison.into_inner());
    let _ = paths::ensure(&paths::cache());
    let mut blob = cached();
    blob.insert("version".into(), json!(VERSION));
    blob.insert(
        name.into(),
        json!({"at": clock::epoch(), "ttl": ttl, "pick": pick}),
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(cache_file())
    {
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        let _ = file.write_all(Value::Object(blob).to_string().as_bytes());
        let _ = file.sync_data();
    }
}

/// What should answer, for this ask and these providers.
///
/// The registry chooses models and effort, never authority: an answer naming a
/// provider the caller did not make available is refused, so a malformed or
/// compromised response cannot route a session somewhere it was not allowed.
pub fn resolve(ask: &Ask, providers: &[String]) -> Result<Pick, NoAnswer> {
    let allowed: BTreeSet<&str> = providers.iter().map(String::as_str).collect();
    if allowed.is_empty() {
        return Err(NoAnswer("no providers are available".into()));
    }

    // You already said what you wanted. Nothing to look up.
    if let Ask::Model {
        provider,
        model,
        effort,
        fast,
    } = ask
    {
        if model.trim().is_empty() {
            return Err(NoAnswer("no model named".into()));
        }
        let chosen = match provider {
            Some(named) => {
                if !allowed.contains(named.as_str()) {
                    return Err(NoAnswer(format!("{named} is not available")));
                }
                named.clone()
            }
            // unstated, and only unambiguous when one provider is available
            None if allowed.len() == 1 => (*allowed.iter().next().unwrap()).to_string(),
            None => {
                return Err(NoAnswer(
                    "say which provider runs it: more than one is available".into(),
                ));
            }
        };
        return Ok(Pick {
            provider: chosen,
            model: model.trim().to_string(),
            effort: effort.clone(),
            fast: *fast,
        });
    }

    let name = cache_name(ask, providers);
    let keep = |pick: Pick, ttl: f64| -> Result<Pick, NoAnswer> {
        if !allowed.contains(pick.provider.as_str()) {
            return Err(NoAnswer(format!(
                "the registry answered with {}, which is not available here",
                pick.provider
            )));
        }
        write_cache(&name, &pick, ttl);
        Ok(pick)
    };

    if let Some(fresh) = read_cache(&name, true) {
        if allowed.contains(fresh.provider.as_str()) {
            return Ok(fresh);
        }
    }
    if let Some(found) = fetch(ask, providers, Duration::from_secs(5)) {
        return keep(found, CACHE_TTL);
    }
    // Unreachable. An old answer beats no answer, but stop asking for a while
    // rather than stalling on every call.
    if let Some(stale) = read_cache(&name, false) {
        return keep(stale, QUIET_TTL);
    }
    let mut named: Vec<&str> = allowed.into_iter().collect();
    named.sort_unstable();
    Err(NoAnswer(format!(
        "no answer for {named:?}: could not reach {} and nothing is cached",
        remote()
    )))
}

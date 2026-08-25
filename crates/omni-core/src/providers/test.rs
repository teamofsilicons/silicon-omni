//! A provider that is not one.
//!
//! Every other adapter needs a CLI, a login and quota. This one needs nothing,
//! answers the same way every time, and can be made to fail on cue — so omni's
//! own behaviour, and yours on top of it, can be driven without spending
//! anything or waiting on a model.
//!
//! Nothing is registered until [`install`] is called, so it can never turn up
//! in the available providers by accident.
//!
//! What it does with what you send:
//!
//! | you send | it does |
//! |---|---|
//! | `[tool:NAME]` | runs `NAME`: a `TOOL_CALL` and a matching result |
//! | `[recall]` | replies with everything it was told before this message |
//! | anything else | replies `echo: <what you sent>` |
//!
//! `[recall]` is the interesting one: it answers out of history it was *seeded*
//! with as readily as history it lived through, which is exactly what a
//! provider switch has to preserve.
//!
//! # Testing the unhappy paths
//!
//! [`running`] hands you the live runner, which records what it was given and
//! what it was sent, and can be driven by hand. Each knob mimics something a
//! real CLI does: [`Knobs::defer`] is agy, which only sees history when the
//! next message goes out; [`Knobs::tunable`] off is agy again, which cannot
//! change model without a restart; a native id starting with [`FORGET`] is any
//! provider that has forgotten a session omni thinks it still has.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use serde_json::{Value, json};

use crate::events::{CRASH, Event, event_type};
use crate::intelligence::{Rung, key, write_cache};
use crate::providers::base::{AUTHENTICATED, Account, Config, Delivery, Emit, Runner};
use crate::translate::transcript;

pub const NAME: &str = "test";
pub const RECALL: &str = "[recall]";
pub const LEVELS: i64 = 11;
/// The dial is generated, not fetched, so it never goes stale.
const PINNED: f64 = 1e9;

/// A native id a provider no longer recognises. Prefix one to test a reseed.
pub const FORGET: &str = "gone-";

/// name -> the runner currently up for it. See [`running`].
static LIVE: RwLock<BTreeMap<String, Arc<Live>>> = RwLock::new(BTreeMap::new());
/// The names [`install`] has registered, so the rest of omni can find them.
static REGISTERED: RwLock<BTreeMap<String, ()>> = RwLock::new(BTreeMap::new());

/// What this double pretends to be able to do.
#[derive(Debug, Clone)]
pub struct Knobs {
    /// Answer a message as soon as it arrives. Off, and the turn stays open
    /// until you call [`Live::reply`] or [`Live::fail`] yourself.
    pub autoreply: bool,
    /// Can change model and effort in place. agy cannot, and says so by not.
    pub tunable: bool,
    /// Like agy: seeded history only reaches the model with the next message.
    pub defer: bool,
}

impl Default for Knobs {
    fn default() -> Self {
        Knobs {
            autoreply: true,
            tunable: true,
            defer: false,
        }
    }
}

#[derive(Default)]
struct State {
    up: bool,
    native_id: String,
    model: String,
    effort: String,
    given: Vec<Event>,
    sent: Vec<String>,
    /// Seed accepted for delivery with the next message, as agy does.
    pending_seed: bool,
    resumed: bool,
    retuned: usize,
    starts: usize,
}

/// The live double, shared between the session that runs it and whoever is
/// driving the test. Every field is behind a lock because those are two
/// different threads, always.
pub struct Live {
    pub name: String,
    knobs: Mutex<Knobs>,
    state: Mutex<State>,
    emit: Mutex<Option<Emit>>,
}

impl Live {
    fn new(name: &str) -> Self {
        Live {
            name: name.to_string(),
            knobs: Mutex::new(Knobs::default()),
            state: Mutex::new(State::default()),
            emit: Mutex::new(None),
        }
    }

    pub fn knobs(&self) -> Knobs {
        self.knobs.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_knobs(&self, knobs: Knobs) {
        *self.knobs.lock().unwrap_or_else(|p| p.into_inner()) = knobs;
    }

    /// Answer and close the turn. For when `autoreply` is off.
    pub fn reply(&self, text: &str) {
        self.say(Event::new(event_type::TEXT).saying(text));
        self.say(Event::new(event_type::END).with("stop", "complete"));
    }

    /// Break the way a real CLI breaks.
    ///
    /// `ends` adds the `END` that all three shipped adapters put out in the
    /// same breath as the error — the pair, not just the half of it.
    pub fn fail(&self, fault: &str, error: &str, ends: bool) {
        let fault = if fault.is_empty() { CRASH } else { fault };
        let said = if error.is_empty() {
            format!("{fault} from {}", self.name)
        } else {
            error.to_string()
        };
        self.say(Event::failure(fault, said));
        if ends {
            self.say(Event::new(event_type::END));
        }
    }

    /// Everything told to this conversation, seeded history included.
    pub fn heard(&self) -> Vec<String> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        transcript(&state.given)
            .into_iter()
            .map(|turn| turn.text)
            .chain(state.sent.iter().cloned())
            .collect()
    }

    pub fn sent(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .sent
            .clone()
    }

    pub fn given(&self) -> Vec<Event> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .given
            .clone()
    }

    pub fn resumed(&self) -> bool {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).resumed
    }

    pub fn retuned(&self) -> usize {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).retuned
    }

    /// How many times a runner for this provider was brought up.
    pub fn starts(&self) -> usize {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).starts
    }

    pub fn up(&self) -> bool {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).up
    }

    fn say(&self, mut event: Event) {
        let model = self
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .model
            .clone();
        event.provider = self.name.clone();
        event.model = model;
        let emit = self.emit.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(emit) = emit {
            emit(event);
        }
    }

    fn answer(&self, text: &str) -> String {
        if !text.contains(RECALL) {
            return format!("echo: {text}");
        }
        match self.heard().join(" | ") {
            heard if heard.is_empty() => "nothing yet".to_string(),
            heard => heard,
        }
    }

    /// What a `[tool:NAME]` in a message asks for.
    fn tools(text: &str) -> Vec<String> {
        text.match_indices("[tool:")
            .filter_map(|(at, _)| {
                let rest = &text[at + 6..];
                let end = rest.find(']')?;
                let name = &rest[..end];
                (!name.is_empty() && !name.contains(char::is_whitespace)).then(|| name.to_string())
            })
            .collect()
    }
}

/// One deterministic conversation. In-process: no subprocess, no threads.
pub struct Double {
    live: Arc<Live>,
    session_id: String,
}

impl Double {
    pub fn new(name: &str, session_id: &str, config: &Config, emit: Emit) -> Self {
        let live = LIVE
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Live::new(name)))
            .clone();
        *live.emit.lock().unwrap_or_else(|p| p.into_inner()) = Some(emit);
        {
            let mut state = live.state.lock().unwrap_or_else(|p| p.into_inner());
            state.model = config.model.clone();
            state.effort = config.effort.clone();
        }
        Double {
            live,
            session_id: session_id.to_string(),
        }
    }
}

impl Runner for Double {
    fn name(&self) -> &str {
        &self.live.name
    }

    fn native_id(&self) -> String {
        self.live
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .native_id
            .clone()
    }

    fn start(&mut self, native_id: &str, history: &[Event]) -> Result<(), String> {
        // A provider that no longer knows the session omni is asking for.
        let native_id = if native_id.starts_with(FORGET) {
            ""
        } else {
            native_id
        };
        let deferred = self.live.knobs().defer && !history.is_empty();
        let mut state = self.live.state.lock().unwrap_or_else(|p| p.into_inner());
        state.resumed = !native_id.is_empty();
        state.native_id = match native_id {
            "" => format!("{}-{}", self.live.name, self.session_id),
            given => given.to_string(),
        };
        state.given = history.to_vec();
        state.sent.clear();
        state.pending_seed = deferred;
        state.retuned = 0;
        state.starts += 1;
        state.up = true;
        Ok(())
    }

    fn seeded(&self) -> bool {
        let state = self.live.state.lock().unwrap_or_else(|p| p.into_inner());
        !state.pending_seed
    }

    fn send(&mut self, text: &str) -> Result<Delivery, String> {
        if !self.live.up() {
            return Err(format!("{} is not running; start it first", self.live.name));
        }
        let answer = self.live.answer(text);
        {
            let mut state = self.live.state.lock().unwrap_or_else(|p| p.into_inner());
            state.sent.push(text.to_string());
            state.pending_seed = false;
        }
        if !self.live.knobs().autoreply {
            return Ok(Delivery::Immediate);
        }
        self.live.say(Event::new(event_type::THINKING));
        let turn = self
            .live
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .sent
            .len();
        for tool in Live::tools(text) {
            let id = format!("{}-{turn}-{tool}", self.native_id());
            let mut call = Event::new(event_type::TOOL_CALL);
            call.tool = tool.clone();
            call.id = id.clone();
            call.args.insert("input".into(), json!(tool));
            self.live.say(call);
            let mut result = Event::new(event_type::TOOL_RESULT);
            result.tool = tool.clone();
            result.id = id;
            result.result = json!(format!("ran {tool}"));
            self.live.say(result);
        }
        self.live.say(Event::new(event_type::TEXT).saying(answer));
        self.live
            .say(Event::new(event_type::END).with("stop", "complete"));
        Ok(Delivery::Immediate)
    }

    fn catch_up(&mut self, history: &[Event]) -> bool {
        let mut state = self.live.state.lock().unwrap_or_else(|p| p.into_inner());
        state.given.extend(history.to_vec());
        if self.live.knobs().defer && !history.is_empty() {
            state.pending_seed = true;
        }
        drop(state);
        self.alive()
    }

    fn retune(&mut self, model: &str, effort: &str) -> bool {
        if !self.live.knobs().tunable {
            return false;
        }
        let mut state = self.live.state.lock().unwrap_or_else(|p| p.into_inner());
        state.model = model.to_string();
        state.effort = effort.to_string();
        state.retuned += 1;
        state.up
    }

    fn stop(&mut self) {
        self.live.state.lock().unwrap_or_else(|p| p.into_inner()).up = false;
    }

    fn alive(&self) -> bool {
        self.live.up()
    }
}

/// Always here, always signed in, never rate limited.
pub struct Signed(pub String);

impl Account for Signed {
    fn name(&self) -> &str {
        &self.0
    }

    fn cli(&self) -> &str {
        "python3"
    }

    fn installed(&self) -> bool {
        true
    }

    fn probe(&self) -> String {
        AUTHENTICATED.into()
    }

    fn start_auth(&self) -> String {
        "the test provider needs no login".into()
    }

    /// No quota to report, and `null` is how omni says so.
    fn limits(&self) -> Value {
        json!({"5h": {"used": null, "reset": null}, "7d": {"used": null, "reset": null}})
    }
}

// ------------------------------------------------------------------ setting up

pub fn rung(provider: &str, model: &str, effort: &str) -> Rung {
    Rung::new(provider, model, effort)
}

/// Spread rungs, best first, over levels 0-10 — the shape the registry serves.
///
/// A level that falls exactly between two rungs takes the weaker one, so the
/// dial is monotonic and every level maps to something.
pub fn dial(rungs: &[Rung]) -> Value {
    if rungs.is_empty() {
        return json!({});
    }
    let steps = (rungs.len() - 1) as f64;
    let mut out = serde_json::Map::new();
    for level in 0..LEVELS {
        let index = (((10 - level) as f64) * steps / 10.0).round() as usize;
        out.insert(
            level.to_string(),
            serde_json::to_value(&rungs[index.min(rungs.len() - 1)]).unwrap_or(Value::Null),
        );
    }
    Value::Object(out)
}

/// A whole dial on one provider, a different model at every level.
pub fn spread(name: &str) -> Value {
    let mut out = serde_json::Map::new();
    for level in 0..LEVELS {
        out.insert(
            level.to_string(),
            json!({"provider": name, "model": format!("{name}-model-{level}"), "effort": ""}),
        );
    }
    Value::Object(out)
}

pub fn pin(names: &[String], levels: &Value) {
    write_cache(&key(names), levels, PINNED);
}

/// Register test providers and pin dials for them, reaching no network.
///
/// With no names you get one provider called `test`, running a different model
/// at every level. Name several and the dial is spread over them, strongest
/// first — pass `rungs` to say exactly which sits where.
pub fn install(names: &[String], rungs: &[Rung]) -> Vec<String> {
    let picked: Vec<String> = if names.is_empty() {
        vec![NAME.to_string()]
    } else {
        names.to_vec()
    };
    {
        let mut registered = REGISTERED.write().unwrap_or_else(|p| p.into_inner());
        for name in &picked {
            registered.insert(name.clone(), ());
        }
    }
    if !rungs.is_empty() {
        pin(&picked, &dial(rungs));
    } else if picked.len() == 1 {
        pin(&picked, &spread(&picked[0]));
    } else {
        let spread: Vec<Rung> = picked
            .iter()
            .map(|name| rung(name, &format!("{name}-model"), ""))
            .collect();
        pin(&picked, &dial(&spread));
    }
    // One dial per set of providers, the way a real registry serves them — so a
    // chat that loses one still resolves. Without these, testing a failover
    // dead-ends at NoDial the moment the first provider is dropped.
    for name in &picked {
        let mine: Vec<Rung> = rungs
            .iter()
            .filter(|r| &r.provider == name)
            .cloned()
            .collect();
        let levels = if mine.is_empty() {
            spread(name)
        } else {
            dial(&mine)
        };
        pin(std::slice::from_ref(name), &levels);
    }
    picked
}

/// The runner currently up for a provider, to inspect or drive by hand.
pub fn running(name: &str) -> Option<Arc<Live>> {
    LIVE.read()
        .unwrap_or_else(|p| p.into_inner())
        .get(name)
        .cloned()
}

pub fn installed() -> Vec<String> {
    REGISTERED
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .keys()
        .cloned()
        .collect()
}

pub fn is_installed(name: &str) -> bool {
    REGISTERED
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .contains_key(name)
}

/// Forget every double. Between tests, and when a daemon is asked to reset.
pub fn forget_all() {
    LIVE.write().unwrap_or_else(|p| p.into_inner()).clear();
    REGISTERED
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_with_no_tools_asks_for_none() {
        assert!(Live::tools("just talking").is_empty());
        assert!(
            Live::tools("[tool: spaced]").is_empty(),
            "a name cannot hold a space"
        );
        assert!(Live::tools("[tool:unclosed").is_empty());
    }

    #[test]
    fn every_tool_named_in_a_message_is_run() {
        assert_eq!(Live::tools("[tool:ls] then [tool:cat]"), vec!["ls", "cat"]);
    }

    #[test]
    fn the_strongest_rung_is_at_the_top_of_the_dial() {
        let levels = dial(&[rung("beta", "big", "high"), rung("alpha", "small", "low")]);
        assert_eq!(levels["10"]["provider"], "beta");
        assert_eq!(levels["0"]["provider"], "alpha");
    }

    #[test]
    fn one_rung_fills_the_whole_dial() {
        let levels = dial(&[rung("solo", "only", "")]);
        for level in 0..LEVELS {
            assert_eq!(levels[level.to_string()]["model"], "only");
        }
    }

    #[test]
    fn a_spread_gives_every_level_its_own_model() {
        let levels = spread("x");
        assert_eq!(levels["0"]["model"], "x-model-0");
        assert_eq!(levels["10"]["model"], "x-model-10");
    }
}

//! Timestamps. One format everywhere: RFC3339 UTC with a trailing Z.
//!
//! Providers do not agree on how to say "at". Claude answers in ISO, Codex in
//! ISO with an offset, agy in seconds since the epoch — and any of them could
//! change their mind in a release. [`iso`] is the one funnel everything goes
//! through, so callers only ever read one shape.

use serde_json::Value;
use time::format_description::BorrowedFormatItem;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// `2026-08-24T09:41:07.512Z` — what every timestamp omni writes looks like.
const STAMP: &[BorrowedFormatItem] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");

/// Offsets written without the colon, which RFC3339 does not allow but two of
/// the three CLIs have emitted at least once.
const OFFSET: &[&[BorrowedFormatItem]] = &[
    format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond][offset_hour][offset_minute]"
    ),
    format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour][offset_minute]"
    ),
];

/// No offset at all. Read as UTC: a provider that does not say means "here",
/// and every one of them runs on UTC internally.
const NAIVE: &[&[BorrowedFormatItem]] = &[
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond]"),
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]"),
    format_description!("[year]-[month]-[day]T[hour]:[minute]"),
];

const DATE: &[BorrowedFormatItem] = format_description!("[year]-[month]-[day]");

pub fn stamp(when: OffsetDateTime) -> String {
    when.to_offset(UtcOffset::UTC)
        .format(STAMP)
        .unwrap_or_default()
}

pub fn now() -> String {
    stamp(OffsetDateTime::now_utc())
}

/// Seconds since the epoch, for the short-lived caches that measure age.
pub fn epoch() -> f64 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() as f64 / 1e9
}

/// Whatever a provider called a moment, as one RFC3339 UTC string.
///
/// Seconds, milliseconds and ISO strings with any offset all land in the same
/// shape. `None` stays `None` — unknown is not the epoch — and something
/// unparseable is handed back untouched rather than turned into a wrong time.
pub fn iso(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::Number(n) => return from_seconds(n.as_f64()?).or_else(|| Some(n.to_string())),
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    };
    if text.is_empty() {
        return None;
    }
    if let Ok(seconds) = text.parse::<f64>() {
        return from_seconds(seconds).or(Some(text));
    }
    Some(parse(&text).map(stamp).unwrap_or(text))
}

/// Milliseconds, if the number is far past any plausible second count.
fn from_seconds(seconds: f64) -> Option<String> {
    let nanos = (if seconds > 1e11 {
        seconds / 1000.0
    } else {
        seconds
    }) * 1e9;
    OffsetDateTime::from_unix_timestamp_nanos(nanos as i128)
        .ok()
        .map(stamp)
}

/// An ISO string with any offset, or none at all — a bare one is read as UTC.
pub fn parse(text: &str) -> Option<OffsetDateTime> {
    let text = text.trim();
    // A space where the T should be is the one deviation worth normalising:
    // it is what `date`, Postgres and Python's own str() all produce.
    let dated = text.replacen(' ', "T", 1);
    if let Ok(when) = OffsetDateTime::parse(&dated, &Rfc3339) {
        return Some(when);
    }
    for shape in OFFSET {
        if let Ok(when) = OffsetDateTime::parse(&dated, shape) {
            return Some(when);
        }
    }
    for shape in NAIVE {
        if let Ok(when) = PrimitiveDateTime::parse(&dated, shape) {
            return Some(when.assume_utc());
        }
    }
    Date::parse(text, DATE)
        .ok()
        .map(|date| date.midnight().assume_utc())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn as_iso(v: Value) -> Option<String> {
        iso(&v)
    }

    #[test]
    fn nothing_said_stays_nothing_said() {
        assert_eq!(as_iso(Value::Null), None);
        assert_eq!(as_iso(json!("")), None);
        assert_eq!(as_iso(json!("   ")), None);
    }

    #[test]
    fn every_shape_lands_on_one() {
        let want = Some("2026-01-02T03:04:05.000Z".to_string());
        for value in [
            json!("2026-01-02T03:04:05Z"),
            json!("2026-01-02T03:04:05+00:00"),
            json!("2026-01-02 03:04:05"),
            json!("2026-01-02T03:04:05"),
            json!("  2026-01-02T03:04:05Z  "),
        ] {
            assert_eq!(as_iso(value.clone()), want, "{value}");
        }
    }

    #[test]
    fn an_offset_is_moved_to_utc() {
        assert_eq!(
            as_iso(json!("2026-01-02T05:04:05+02:00")),
            Some("2026-01-02T03:04:05.000Z".into())
        );
    }

    #[test]
    fn seconds_and_milliseconds_are_told_apart() {
        let seconds = as_iso(json!(1_767_322_800)).unwrap();
        let millis = as_iso(json!(1_767_322_800_000i64)).unwrap();
        assert_eq!(seconds, millis, "the same moment, said two ways");
        assert_eq!(seconds, "2026-01-02T03:00:00.000Z");
    }

    #[test]
    fn a_number_in_a_string_counts_as_a_number() {
        assert_eq!(
            as_iso(json!("1767322800")),
            Some("2026-01-02T03:00:00.000Z".into())
        );
    }

    #[test]
    fn nonsense_is_handed_back_rather_than_guessed() {
        assert_eq!(as_iso(json!("next tuesday")), Some("next tuesday".into()));
    }

    #[test]
    fn now_is_the_shape_we_promised() {
        let text = now();
        assert!(text.ends_with('Z') && text.len() == 24, "{text}");
        assert!(parse(&text).is_some());
    }
}

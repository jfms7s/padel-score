//! WASM bindings for the scoring engine, so the browser can compute the score offline.

use padel_scoring::{Event, Format, Score, Start};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

/// Replays `events` (JSON array) under `format` and `start` (JSON) and returns JSON:
/// `{"ok": Score}` or `{"err": {"kind": .., "index": ..}}`; bad input gives
/// `{"err": {"kind": "bad_json", "message": ..}}`. Never panics.
pub fn replay_json(format: &str, start: &str, events: &str) -> String {
    let value = match run(format, start, events) {
        Ok(score) => json!({ "ok": score }),
        Err(err) => json!({ "err": err }),
    };
    value.to_string()
}

fn run(format: &str, start: &str, events: &str) -> Result<Score, Value> {
    let format: Format = parse("format", format)?;
    let start: Start = parse("start", start)?;
    let events: Vec<Event> = parse("events", events)?;
    padel_scoring::replay(&format, &start, &events)
        .map_err(|e| json!({ "kind": e.kind, "index": e.index }))
}

fn parse<T: DeserializeOwned>(what: &str, input: &str) -> Result<T, Value> {
    serde_json::from_str(input)
        .map_err(|e| json!({ "kind": "bad_json", "message": format!("{what}: {e}") }))
}

#[wasm_bindgen]
pub fn replay(format: &str, start: &str, events: &str) -> String {
    replay_json(format, start, events)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const SETS: &str = r#"{"sets":{"deuce":"advantage","sets":"one","deciding":"full_set"}}"#;
    const TIMED: &str = r#"{"timed":{"deuce":"golden_point","draws_allowed":false}}"#;
    const START: &str = r#"{"first_server":{"team":"a","index":0},"other_first_server":0}"#;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("output must be valid JSON")
    }

    #[test]
    fn replay_json_ok_for_three_points() {
        let out = parse(&replay_json(
            SETS,
            START,
            r#"[{"point":"a"},{"point":"a"},{"point":"b"}]"#,
        ));
        assert!(out.get("ok").is_some_and(Value::is_object), "{out}");
        assert!(out.get("err").is_none());
    }

    #[test]
    fn replay_json_reports_engine_error() {
        let out = parse(&replay_json(SETS, START, r#"[{"point":"a"},"time_up"]"#));
        assert_eq!(
            out,
            serde_json::json!({"err": {"kind": "not_timed", "index": 1}})
        );
        let out = parse(&replay_json(TIMED, START, r#"["time_up","time_up"]"#));
        assert_eq!(out["err"]["kind"], "already_time_up");
    }

    #[test]
    fn replay_json_bad_json_is_error_not_panic() {
        for (f, s, e) in [
            ("nope", START, "[]"),
            (SETS, "{", "[]"),
            (SETS, START, "[{\"point\":\"c\"}]"),
            ("", "", ""),
        ] {
            let out = parse(&replay_json(f, s, e));
            assert_eq!(out["err"]["kind"], "bad_json", "{out}");
            assert!(out["err"]["message"].is_string());
        }
    }
}

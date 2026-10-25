//! JSON log lines: level, message and the first few other fields.

use serde_json::{Map, Value};

use crate::classify::level_from_word;
use crate::{JsonSummary, Level};

const LEVEL_KEYS: [&str; 4] = ["level", "severity", "lvl", "loglevel"];
const MESSAGE_KEYS: [&str; 2] = ["msg", "message"];
const MAX_FIELDS: usize = 6;
const MAX_VALUE_CHARS: usize = 200;

fn level_of(value: &Value) -> Option<Level> {
    match value {
        Value::String(word) => level_from_word(word),
        // pino and bunyan: 10 trace, 20 debug, 30 info, 40 warn, 50 error, 60 fatal.
        Value::Number(n) => n.as_i64().map(|n| match n {
            ..=10 => Level::Trace,
            11..=20 => Level::Debug,
            21..=30 => Level::Info,
            31..=40 => Level::Warn,
            _ => Level::Error,
        }),
        _ => None,
    }
}

fn render(value: &Value) -> String {
    let text = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.chars().count() > MAX_VALUE_CHARS {
        let cut: String = text.chars().take(MAX_VALUE_CHARS).collect();
        format!("{cut}…")
    } else {
        text
    }
}

fn first_present(map: &Map<String, Value>, keys: &[&'static str]) -> Option<&'static str> {
    keys.iter().copied().find(|key| map.contains_key(*key))
}

/// A one-line JSON object as `level · message · k=v …`. Anything else (broken JSON, arrays,
/// scalars) is `None`. The first of `level`/`severity`/`lvl`/`loglevel` and of `msg`/`message`
/// are taken out of the field list; the next six fields are kept in source order.
#[must_use]
pub fn summarize_json(text: &str) -> Option<JsonSummary> {
    let trimmed = text.trim();
    if !(trimmed.starts_with('{') && trimmed.ends_with('}')) {
        return None;
    }
    let Value::Object(map) = serde_json::from_str::<Value>(trimmed).ok()? else {
        return None;
    };
    let level_key = first_present(&map, &LEVEL_KEYS);
    let message_key = first_present(&map, &MESSAGE_KEYS);
    let level = level_key.and_then(|key| map.get(key)).and_then(level_of);
    let message = message_key.and_then(|key| map.get(key)).map(render);
    let fields = map
        .iter()
        .filter(|(key, _)| Some(key.as_str()) != level_key && Some(key.as_str()) != message_key)
        .take(MAX_FIELDS)
        .map(|(key, value)| (key.clone(), render(value)))
        .collect();
    Some(JsonSummary {
        level,
        message,
        fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_message_and_fields_in_source_order() {
        let s =
            summarize_json(r#"{"level":"info","msg":"server started","port":8080,"env":"dev"}"#)
                .expect("json");
        assert_eq!(s.level, Some(Level::Info));
        assert_eq!(s.message.as_deref(), Some("server started"));
        assert_eq!(
            s.fields,
            vec![
                ("port".to_string(), "8080".to_string()),
                ("env".to_string(), "dev".to_string())
            ]
        );
    }

    #[test]
    fn severity_and_message_keys_are_understood() {
        let s = summarize_json(r#"{"severity":"ERROR","message":"payment failed","order":1042}"#)
            .expect("json");
        assert_eq!(s.level, Some(Level::Error));
        assert_eq!(s.message.as_deref(), Some("payment failed"));
    }

    #[test]
    fn numeric_levels_follow_pino() {
        let level = |n: i64| summarize_json(&format!(r#"{{"level":{n}}}"#)).and_then(|s| s.level);
        assert_eq!(level(10), Some(Level::Trace));
        assert_eq!(level(20), Some(Level::Debug));
        assert_eq!(level(30), Some(Level::Info));
        assert_eq!(level(40), Some(Level::Warn));
        assert_eq!(level(50), Some(Level::Error));
        assert_eq!(level(60), Some(Level::Error));
    }

    #[test]
    fn only_six_other_fields_are_kept_and_nested_values_are_compact() {
        let s = summarize_json(
            r#"{"msg":"m","a":1,"b":2,"c":3,"d":4,"e":5,"f":6,"g":7,"ctx":{"user":"anna"}}"#,
        )
        .expect("json");
        assert_eq!(s.fields.len(), 6);
        assert_eq!(s.fields[5], ("f".to_string(), "6".to_string()));
        let nested = summarize_json(r#"{"ctx":{"user":"anna"},"ok":null}"#).expect("json");
        assert_eq!(nested.fields[0].1, r#"{"user":"anna"}"#);
        assert_eq!(nested.fields[1].1, "null");
    }

    #[test]
    fn long_values_are_cut() {
        let long = "x".repeat(500);
        let s = summarize_json(&format!(r#"{{"blob":"{long}"}}"#)).expect("json");
        assert_eq!(s.fields[0].1.chars().count(), MAX_VALUE_CHARS + 1);
        assert!(s.fields[0].1.ends_with('…'));
    }

    #[test]
    fn non_objects_and_broken_json_are_none() {
        for text in ["{not json at all", r#"["an","array"]"#, "plain", "", "{}x"] {
            assert_eq!(summarize_json(text), None, "{text}");
        }
        assert!(summarize_json("  {}  ").is_some());
    }
}

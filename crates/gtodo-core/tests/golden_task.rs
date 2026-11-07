//! タスク行の解析・文字列化が Lua 版 (gtodo-md.nvim) と一致することを検証する。
//! 期待値は `scripts/gen-golden-task.lua` で Lua 版から生成する。

use chrono::NaiveDate;
use gtodo_core::task::{Status, Task};
use serde_json::{Map, Value, json};

fn load_golden() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/golden/task_parse.json");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    serde_json::from_str(&text).expect("golden file must be valid JSON")
}

/// Lua 版の parse 結果と同じ形 (`context` は `@` 付き、`status` は `" "`/`"x"`) に揃える。
fn to_lua_shape(t: &Task) -> Value {
    let mut m = Map::new();
    m.insert("indent".into(), json!(t.indent));
    m.insert(
        "status".into(),
        json!(match t.status {
            Status::Open => " ",
            Status::Done => "x",
        }),
    );
    if let Some(p) = t.priority {
        m.insert("priority".into(), json!(p.to_string()));
    }
    m.insert("content".into(), json!(t.content));
    let tags = [
        ("project", t.project.clone()),
        ("context", t.context.as_ref().map(|c| format!("@{c}"))),
        ("due", t.due.clone()),
        ("created", t.created.clone()),
        ("wait", t.wait.clone()),
        ("completed_at", t.completed_at.clone()),
        ("done", t.done.clone()),
        ("cancelled", t.cancelled.clone()),
        ("from", t.from.clone()),
        ("id", t.id.clone()),
    ];
    for (k, v) in tags {
        if let Some(v) = v {
            m.insert(k.into(), json!(v));
        }
    }
    Value::Object(m)
}

#[test]
fn parse_and_serialize_match_lua_implementation() {
    let golden = load_golden();
    let today = NaiveDate::parse_from_str(golden["today"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    let placeholder_id = golden["placeholder_id"].as_str().unwrap();
    let cases = golden["cases"].as_array().unwrap();
    assert!(!cases.is_empty());

    let mut failures = Vec::new();
    for case in cases {
        let input = case["input"].as_str().unwrap();
        let parsed = Task::parse_at(input, today);
        match (&case["parsed"], parsed) {
            (Value::Null, None) => {}
            (Value::Null, Some(t)) => failures.push(format!("{input:?}: expected non-task, got {t:?}")),
            (expected, None) => failures.push(format!("{input:?}: expected {expected}, got non-task")),
            (expected, Some(mut t)) => {
                let actual = to_lua_shape(&t);
                if &actual != expected {
                    failures.push(format!(
                        "{input:?}:\n  parsed   expected {expected}\n           actual   {actual}"
                    ));
                }
                if t.id.is_none() {
                    t.id = Some(placeholder_id.to_string());
                }
                let serialized = t.serialize();
                let expected = case["serialized"].as_str().unwrap();
                if serialized != expected {
                    failures.push(format!(
                        "{input:?}:\n  serialize expected {expected:?}\n            actual   {serialized:?}"
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

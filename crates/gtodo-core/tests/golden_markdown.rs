//! ファイル構造の解析と書き出しが Lua 版 (gtodo-md.nvim の `io.parse_markdown` / `io.write_todo_file`) と
//! 一致することを検証する。
//! 期待値は `scripts/gen-golden-markdown.lua` で Lua 版から生成する。

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::NaiveDate;
use gtodo_core::document::{Document, Item};
use gtodo_core::sections::{SectionKey, SectionNames};
use gtodo_core::store::read_file;
use serde_json::{Value, json};

mod common;
use common::mask_new_ids;

fn golden_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/golden"))
}

fn section_names(config: &Value, last: &Value) -> SectionNames {
    let current = SectionKey::ALL.map(|k| {
        config
            .get(k.state_key())
            .and_then(Value::as_str)
            .unwrap_or(k.default_name())
            .to_string()
    });
    SectionKey::ALL
        .into_iter()
        .fold(SectionNames::new(current).unwrap(), |names, k| {
            names.with_last(k, last.get(k.state_key()).and_then(Value::as_str).map(str::to_string))
        })
}

fn items_json(items: &[Item]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| match item {
                Item::Task(t) => json!({"kind": "task", "line": t.line}),
                Item::Text(line) => json!({"kind": "text", "line": line}),
            })
            .collect(),
    )
}

fn to_lua_shape(doc: &Document) -> Value {
    json!({
        "header": doc.header,
        "preamble": items_json(&doc.preamble),
        "sections": doc.sections.iter()
            .map(|s| json!({"name": s.name, "items": items_json(&s.items)}))
            .collect::<Vec<_>>(),
    })
}

#[test]
fn parse_matches_lua_implementation() {
    let text = std::fs::read_to_string(golden_dir().join("markdown_parse.json")).unwrap();
    let golden: Value = serde_json::from_str(&text).unwrap();
    let today = NaiveDate::parse_from_str(golden["today"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    let cases = golden["cases"].as_array().unwrap();
    assert!(!cases.is_empty());

    let mut failures = Vec::new();
    for case in cases {
        let file = case["file"].as_str().unwrap();
        let lines = read_file(&golden_dir().join("markdown").join(file)).unwrap().lines;
        let names = section_names(&case["sections_config"], &case["last_sections"]);
        let actual = to_lua_shape(&Document::parse(&lines, &names, today));
        let expected = json!({
            "header": case["header"],
            "preamble": case["preamble"],
            "sections": case["sections"],
        });
        if actual != expected {
            failures.push(format!("{file}:\n  expected {expected}\n  actual   {actual}"));
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

#[test]
fn write_matches_lua_implementation() {
    let text = std::fs::read_to_string(golden_dir().join("markdown_parse.json")).unwrap();
    let golden: Value = serde_json::from_str(&text).unwrap();
    let today = NaiveDate::parse_from_str(golden["today"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    let cases = golden["cases"].as_array().unwrap();

    let mut failures = Vec::new();
    for case in cases {
        let file = case["file"].as_str().unwrap();
        let lines = read_file(&golden_dir().join("markdown").join(file)).unwrap().lines;
        let names = section_names(&case["sections_config"], &case["last_sections"]);
        let mut doc = Document::parse(&lines, &names, today);
        let known: HashSet<String> = doc.tasks().filter_map(|(_, t)| t.task.id.clone()).collect();

        let actual = mask_new_ids(&doc.serialize(), &known, today);
        let expected: Vec<String> = case["written"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_string())
            .collect();
        let expected = mask_new_ids(&expected, &known, today);
        if actual != expected {
            failures.push(format!("{file}:\n  expected {expected:?}\n  actual   {actual:?}"));
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

#[test]
fn heading_named_default_is_kept_as_a_section() {
    let lines: Vec<String> = ["# Todo", "- [ ] 前", "## default", "- [ ] 後"]
        .map(String::from)
        .to_vec();
    let mut doc = Document::parse(
        &lines,
        &SectionNames::default(),
        NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
    );
    assert_eq!(doc.preamble.len(), 1);
    assert_eq!(doc.sections.len(), 1);
    assert_eq!(doc.sections[0].name, "default");
    assert_eq!(doc.sections[0].items.len(), 1);
    assert!(doc.serialize().contains(&"## default".to_string()));
}

#[test]
fn tasks_are_listed_in_file_order_even_when_duplicate_sections_are_merged() {
    let lines: Vec<String> = ["## Today", "- [ ] 1", "## Next", "- [ ] 2", "## Today", "- [ ] 3"]
        .map(String::from)
        .to_vec();
    let doc = Document::parse(
        &lines,
        &SectionNames::default(),
        NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
    );
    let order: Vec<_> = doc
        .tasks()
        .map(|(s, t)| (s.unwrap(), t.task.content.as_str()))
        .collect();
    assert_eq!(order, [("Today", "1"), ("Next", "2"), ("Today", "3")]);
}

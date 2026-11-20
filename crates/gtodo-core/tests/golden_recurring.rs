//! 定期タスクの規則の読み取り・判定と、テンプレートから inbox.md へ足す処理が Lua 版 (gtodo-md.nvim) と
//! 一致することを検証する。期待値は `scripts/gen-golden-recurring.lua` で Lua 版から生成する。

use std::collections::BTreeMap;
use std::fs;

use chrono::{Datelike, NaiveDate};
use gtodo_core::ops;
use gtodo_core::recurring::{self, Nth, Rule};
use gtodo_core::sections::SectionNames;
use gtodo_core::store::DataDir;
use serde_json::{Map, Value, json};

fn load_golden() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/golden/recurring.json");
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    serde_json::from_str(&text).expect("golden file must be valid JSON")
}

fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn fmt(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

/// Lua 版の規則と同じ形 (`weekday` は日曜が 0、`nth` は最終が -1)。
fn rule_json(rule: &Rule) -> Value {
    match *rule {
        Rule::Weekday(w) => json!({"kind": "weekday", "weekday": w.num_days_from_sunday()}),
        Rule::MonthDay(day) => json!({"kind": "month_day", "day": day}),
        Rule::MonthEnd => json!({"kind": "month_end"}),
        Rule::NthWeekday { nth, weekday } => json!({
            "kind": "nth_weekday",
            "nth": match nth { Nth::Week(n) => i64::from(n), Nth::Last => -1 },
            "weekday": weekday.num_days_from_sunday(),
        }),
        Rule::YearDay { month, day } => json!({"kind": "year_day", "month": month, "day": day}),
    }
}

fn rules_json(rules: &[Rule]) -> Value {
    Value::Array(rules.iter().map(rule_json).collect())
}

fn parse(spec: &str) -> Vec<Rule> {
    recurring::parse_repeat(spec).unwrap_or_else(|e| panic!("{spec:?}: {e}"))
}

#[test]
fn repeat_is_read_like_lua() {
    let golden = load_golden();
    let mut failures = Vec::new();
    for case in golden["repeat"].as_array().unwrap() {
        let spec = case["spec"].as_str().unwrap();
        let actual = match recurring::parse_repeat(spec) {
            Ok(rules) => rules_json(&rules),
            Err(_) => json!("error"),
        };
        if actual != case["rules"] {
            failures.push(format!("{spec:?}: expected {}, got {actual}", case["rules"]));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_same_days_are_matched_as_lua() {
    let golden = load_golden();
    let from = date(golden["occurs_from"].as_str().unwrap()).unwrap();
    let to = date(golden["occurs_to"].as_str().unwrap()).unwrap();
    let mut failures = Vec::new();
    for case in golden["occurs"].as_array().unwrap() {
        let spec = case["spec"].as_str().unwrap();
        let rules = parse(spec);
        let actual: Vec<String> = from
            .iter_days()
            .take_while(|d| *d <= to)
            .filter(|d| recurring::occurs_on(&rules, *d))
            .map(fmt)
            .collect();
        let expected = strings(&case["dates"]);
        assert!(!expected.is_empty(), "{spec:?}");
        if actual != expected {
            failures.push(format!("{spec:?}: expected {expected:?}, got {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn latest_and_next_occurrences_match_lua() {
    let golden = load_golden();
    for case in golden["latest"].as_array().unwrap() {
        let rules = parse(case["spec"].as_str().unwrap());
        // 読めない記録は、記録が無いのと同じ (Lua 版と同じ)。
        let after = case["after"].as_str().and_then(date);
        let actual = date(case["today"].as_str().unwrap())
            .and_then(|today| recurring::latest_occurrence(&rules, after, today))
            .map(fmt);
        assert_eq!(actual.as_deref(), case["result"].as_str(), "{case}");
    }
    for case in golden["next"].as_array().unwrap() {
        let rules = parse(case["spec"].as_str().unwrap());
        let actual = date(case["from"].as_str().unwrap())
            .and_then(|from| recurring::next_occurrence(&rules, from))
            .map(fmt);
        assert_eq!(actual.as_deref(), case["result"].as_str(), "{case}");
    }
}

#[test]
fn front_matter_is_read_like_lua() {
    let golden = load_golden();
    let mut failures = Vec::new();
    for case in golden["definitions"].as_array().unwrap() {
        let lines = strings(&case["lines"]);
        let def = recurring::read_definition(&lines);
        let values: Map<String, Value> = def.values.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        let actual = json!({
            "recurring": def.recurring,
            "rules": def.rules.as_deref().map_or(Value::Null, rules_json),
            "repeat_text": def.repeat_text,
            "values": values,
            "body": def.body,
            "error": def.error.is_some(),
        });
        let expected = json!({
            "recurring": case["recurring"],
            "rules": case["rules"],
            "repeat_text": case["repeat_text"],
            "values": case["values"],
            "body": case["body"],
            "error": case["error"],
        });
        if actual != expected {
            failures.push(format!("{lines:?}:\n  expected {expected}\n  got      {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// 足す処理は、ケースの入力 (`recurring/cases.json`) のテンプレートと記録から始める。
#[test]
fn tasks_are_added_and_recorded_like_lua() {
    let golden = load_golden();
    let cases_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/golden/recurring/cases.json"
    );
    let inputs: Value = serde_json::from_str(&fs::read_to_string(cases_path).unwrap()).unwrap();
    let inbox = strings(&golden["inbox"]);
    let names = SectionNames::default();

    let mut failures = Vec::new();
    let expected_cases = golden["create"].as_array().unwrap();
    let input_cases = inputs["create"].as_array().unwrap();
    assert_eq!(expected_cases.len(), input_cases.len());
    for (input, expected) in input_cases.iter().zip(expected_cases) {
        let name = input["name"].as_str().unwrap();
        assert_eq!(expected["name"].as_str(), Some(name));
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("templates")).unwrap();
        fs::write(dir.path().join("inbox.md"), inbox.join("\n") + "\n").unwrap();
        for (template, lines) in input["templates"].as_object().unwrap() {
            fs::write(
                dir.path().join(format!("templates/{template}.md")),
                strings(lines).join("\n") + "\n",
            )
            .unwrap();
        }
        let recorded: BTreeMap<String, String> = input["recurring"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect();
        let today = date(input["today"].as_str().unwrap()).unwrap();

        let outcome = ops::create_recurring(&DataDir::new(dir.path()), &names, today, &recorded).unwrap();

        let written = fs::read_to_string(dir.path().join("inbox.md")).unwrap();
        let written: Vec<&str> = written.lines().collect();
        assert_eq!(written[..inbox.len()], inbox, "{name}: the existing lines are kept");
        let added: Vec<String> = written[inbox.len()..]
            .iter()
            .map(|l| match l.rsplit_once(" id:") {
                Some((rest, id)) if id.len() == 6 && id.bytes().all(|b| b.is_ascii_hexdigit()) => rest.to_string(),
                _ => l.to_string(),
            })
            .collect();
        let records: Map<String, Value> = outcome.records.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        let skipped: Vec<&str> = outcome.skipped.iter().map(|(n, _)| n.as_str()).collect();
        let actual = json!({"added": added, "records": records, "skipped": skipped});
        let want = json!({"added": expected["added"], "records": expected["records"], "skipped": expected["skipped"]});
        if actual != want {
            failures.push(format!("{name}:\n  expected {want}\n  got      {actual}"));
        }
        assert_eq!(outcome.ids.len(), added.len(), "{name}");
        // 今日の曜日の確認 (ケースの日付を取り違えていないか)。
        assert_eq!(
            today.weekday().num_days_from_sunday(),
            4,
            "{name}: cases use a Thursday"
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

//! タスクの並べ替え・期日による昇格が Lua 版 (gtodo-md.nvim の `logic/`) と
//! 同じファイルを書くことを検証する。期待値は `scripts/gen-golden-logic.lua` で Lua 版から生成する。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use gtodo_core::ops;
use gtodo_core::sections::{SectionKey, SectionNames};
use gtodo_core::store::{DataDir, read_file};
use gtodo_core::task::Task;
use serde_json::Value;

mod common;
use common::mask_new_ids;

const FILES: [&str; 3] = ["inbox.md", "todo.md", "done.md"];

fn logic_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/golden/logic"))
}

fn section_names(config: &Value) -> SectionNames {
    SectionNames::new(SectionKey::ALL.map(|k| {
        config
            .get(k.state_key())
            .and_then(Value::as_str)
            .unwrap_or(k.default_name())
            .to_string()
    }))
    .unwrap()
}

fn read_lines(path: &Path) -> Option<Vec<String>> {
    path.exists().then(|| read_file(path).unwrap().lines)
}

#[test]
fn file_operations_match_lua_implementation() {
    let text = fs::read_to_string(logic_dir().with_extension("json")).unwrap();
    let golden: Value = serde_json::from_str(&text).unwrap();
    let today = NaiveDate::parse_from_str(golden["today"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    let cases = golden["cases"].as_array().unwrap();
    assert!(!cases.is_empty());

    let mut failures = Vec::new();
    for case in cases {
        let name = case["dir"].as_str().unwrap();
        let source = logic_dir().join(name);
        let work = tempfile::tempdir().unwrap();
        let mut known = HashSet::new();
        for file in FILES {
            if let Some(lines) = read_lines(&source.join(file)) {
                known.extend(lines.iter().filter_map(|l| Task::parse_at(l, today).and_then(|t| t.id)));
                fs::copy(source.join(file), work.path().join(file)).unwrap();
            }
        }

        let data_dir = DataDir::new(work.path());
        let names = section_names(&case["sections_config"]);
        let result = match case["op"].as_str().unwrap() {
            "sort" => ops::sort_todo(&data_dir, &names, today).map(drop),
            "check_dues" => {
                let auto_move = case["auto_move_inbox_to_today"].as_bool().unwrap();
                ops::check_dues(&data_dir, &names, today, auto_move).map(drop)
            }
            op => panic!("unknown op {op}"),
        };
        if let Err(e) = result {
            failures.push(format!("{name}: {e}"));
            continue;
        }

        // Lua 版は内容が変わらなくても書き込みを呼ぶため、書いた順ではなく最後の内容を比べる。
        // 順序 (追記してから削除) は ops の個別のテストで確かめる。
        for file in FILES {
            let lua_last = case["writes"]
                .as_array()
                .unwrap()
                .iter()
                .rfind(|w| w["file"] == file)
                .map(|w| {
                    w["lines"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|l| l.as_str().unwrap().to_string())
                        .collect::<Vec<_>>()
                });
            let expected = lua_last
                .or_else(|| read_lines(&source.join(file)))
                .map(|lines| mask_new_ids(&lines, &known, today));
            let actual = read_lines(&work.path().join(file)).map(|lines| mask_new_ids(&lines, &known, today));
            if actual != expected {
                failures.push(format!(
                    "{name}/{file}:\n  expected {expected:?}\n  actual   {actual:?}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

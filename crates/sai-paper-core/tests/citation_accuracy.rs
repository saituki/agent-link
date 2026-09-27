//! 引用抽出の精度（段階 8）。`tests/fixtures/citations/` の正解表と比べる。
//!
//! 論文ごとに「References の件数」と「本文の引用が正しい文献に結び付いた数」を出す。
//! 数字を見るには `cargo test -p sai-paper-core --test citation_accuracy -- --nocapture`。

use std::path::Path;

use sai_paper_core::citation::{cited_sentences, parsers, refs_from_markdown};
use serde_json::Value;

/// 論文 1 本の結果: (References の件数, 正しく結び付いた引用の数, 引用の数, 文の平均の長さ)。
fn evaluate(name: &str, expected: &Value) -> (usize, usize, usize, usize) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/citations");
    let markdown = std::fs::read_to_string(dir.join(format!("{name}.md"))).unwrap();
    let (refs, numbered) = refs_from_markdown(&markdown, parsers::Journal::Generic, name);
    let sentences = cited_sentences(&markdown, &refs, numbered);
    let cites = expected["cites"].as_array().unwrap();
    let mut correct = 0;
    for cite in cites {
        let needle = cite["contains"].as_str().unwrap();
        let want: Vec<i64> = cite["refs"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        // 引用を含む文のうち、正しい文献をすべて持つものがあれば正解
        let ok = sentences
            .iter()
            .filter(|(text, _)| text.contains(needle))
            .any(|(_, got)| want.iter().all(|no| got.contains(no)));
        correct += usize::from(ok);
    }
    let avg_len = if sentences.is_empty() {
        0
    } else {
        sentences.iter().map(|(t, _)| t.chars().count()).sum::<usize>() / sentences.len()
    };
    (refs.len(), correct, cites.len(), avg_len)
}

#[test]
fn citation_accuracy_does_not_regress() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/citations/expected.json");
    let table: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut total = (0, 0);
    println!("{:<30} {:>10} {:>10} {:>8}", "論文", "文献数", "引用の正解", "文の長さ");
    for (name, expected) in table["papers"].as_object().unwrap() {
        let (n_refs, correct, n_cites, avg_len) = evaluate(name, expected);
        let want_refs = expected["refs"].as_u64().unwrap() as usize;
        println!(
            "{name:<30} {:>4}/{:<5} {:>5}/{:<4} {:>8}",
            n_refs, want_refs, correct, n_cites, avg_len
        );
        total.0 += correct;
        total.1 += n_cites;
        // 下限（変更前の結果）より悪くならない
        if let Some(min) = expected.get("current_min") {
            assert!(
                correct >= min["cites"].as_u64().unwrap() as usize,
                "{name}: 引用の正解が減った（{correct} < {}）",
                min["cites"]
            );
            let min_refs_err = min["refs_error"].as_u64().unwrap() as usize;
            assert!(
                n_refs.abs_diff(want_refs) <= min_refs_err,
                "{name}: 文献数のずれが増えた（{} > {min_refs_err}）",
                n_refs.abs_diff(want_refs)
            );
        }
    }
    println!("合計: 引用の正解 {}/{}", total.0, total.1);
}

#[test]
#[ignore = "調べる用: 外れた引用と文献の読み取り結果を出す"]
fn dump_misses() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/citations");
    let table: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
    for (name, expected) in table["papers"].as_object().unwrap() {
        let md = std::fs::read_to_string(dir.join(format!("{name}.md"))).unwrap();
        let (refs, numbered) = refs_from_markdown(&md, parsers::Journal::Generic, name);
        let sentences = cited_sentences(&md, &refs, numbered);
        for cite in expected["cites"].as_array().unwrap() {
            let needle = cite["contains"].as_str().unwrap();
            let found: Vec<_> = sentences.iter().filter(|(t, _)| t.contains(needle)).collect();
            println!("== {name}: {needle} want={}", cite["refs"]);
            for (t, got) in &found {
                println!("   got={got:?} len={}", t.chars().count());
            }
            for no in cite["refs"].as_array().unwrap() {
                let no = no.as_i64().unwrap();
                if let Some(r) = refs.iter().find(|r| r.ref_no == no) {
                    println!("   ref {no}: {:?} {:?} {:?} | {}", r.first_author, r.year, r.year_suffix, r.raw_text.chars().take(60).collect::<String>());
                }
            }
        }
    }
}

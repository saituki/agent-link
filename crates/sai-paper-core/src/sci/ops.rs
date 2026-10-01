//! 研究方針の木（Research-tree）を変える 3 つの入口。提案・人の操作・仕分け。
//!
//! エージェントは木を直接書き換えない。代わりに [`propose`] で提案を出し、
//! 危険度が「人」のものは仕分け箱 [`triage`] で利用者が決めてから入る。
//! 利用者が直接入れるときは [`apply_user`] を使う。
//!
//! 検査は全部を先に行い、1 つでも通らなければ 1 件も書かない（`propose` は 1 つのトランザクションで書く）。
//! 文はすべて日本語で、利用者にそのまま見せる。
//!
//! 書き出し（`state.md`・`tree.md`）は呼び出し側の仕事。ここは export を呼ばない。

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::defs::Defs;
use super::store::{
    self, NewDecision, NewFinding, NewLink, NewNode, NewProposal, NewTombstone, Node,
    RejectedOption,
};
use super::{Result, SciError, fmt_ref, parse_ref};
use crate::paths::PaperRoots;

// ---- 扱える操作 -------------------------------------------------------------

/// すぐ反映する操作（提案でも自動で入る）。
const AUTO_OPS: &[&str] = &[
    "add_finding",
    "add_question",
    "answer_question",
    "link",
    "add_seed",
];
/// 利用者が採用してから反映する操作。
const HUMAN_OPS: &[&str] = &[
    "add_option",
    "add_child",
    "decide",
    "reopen",
    "exclude_item",
    "option_state",
    "exclude",
    "edit_text",
    "add_edge",
];
/// 提案には使えない、利用者だけの操作。
const USER_ONLY_OPS: &[&str] = &[
    "unlink",
    "remove_finding",
    "set_read_state",
    "withdraw_question",
    "remove_edge",
];
/// 扱える操作すべて（C-07 の表の 19 種類）。
const KNOWN_OPS: &[&str] = &[
    "add_finding",
    "add_question",
    "answer_question",
    "link",
    "add_seed",
    "add_option",
    "add_child",
    "decide",
    "reopen",
    "exclude_item",
    "option_state",
    "exclude",
    "edit_text",
    "add_edge",
    "unlink",
    "remove_finding",
    "set_read_state",
    "withdraw_question",
    "remove_edge",
];
/// 人の操作で理由が要る操作。
const REASON_REQUIRED_OPS: &[&str] = &["decide", "exclude_item", "option_state", "exclude"];
/// 線として引ける種類（`parent` は木の形そのものなので引けない）。
const EDGE_KINDS: &[&str] = &["depends", "derived", "analogous"];
/// 候補の状態として指定できるもの（`selected` は decide で選ぶ）。
const OPTION_TARGET_STATES: &[&str] = &["rejected", "dormant", "candidate"];
/// 論文の読み具合（C-01）。
const READ_STATES: &[&str] = &["unread", "read", "reflected", "irrelevant"];
/// 知見の向き（C-01）。
const POLARITIES: &[&str] = &["support", "refute", "neutral", "none"];
/// 論文の役割（C-01）。`paper` の結び付きのときだけ使う。
const ROLES: &[&str] = &[
    "exemplar",
    "compare",
    "issue_evidence",
    "method_source",
    "data_source",
    "background",
    "counter",
];
/// 項目の型（C-01）。
const TYPES: &[&str] = &["choose", "list", "write", "verify"];
/// 1 回の提案で受け付ける数。
const MAX_PROPOSALS: usize = 20;

/// 引用の形 `^#[1-9][0-9]{0,8}( .{1,80})?$`（C-07）。
static CITE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#([1-9][0-9]{0,8})( .{1,80})?$").expect("引用の形"));

/// `add_edge` の線の向き。schema の説明と、輪のエラーで同じ文を使う。
const EDGE_DIRECTION: &str = "from が上流、to が下流";

// ---- op の項目の表 -----------------------------------------------------------

/// 項目の型。検査（[`check_shapes`]）と schema（[`propose_schema`]）はこの表から作る。
///
/// 表と [`Ctx::check`] の実装（`check_*`）が食い違うときは `check_*` に合わせる。
/// 型の違う値を黙って空にしないこと（型が違うなら `check_*` の前に [`check_shapes`] で止める）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// 文字列。
    Str,
    /// 文字列の配列。
    StrArray,
    /// 整数。数字だけ書いた文字列でも受け付ける（Web は数で送る）。
    Int,
    /// 真偽。`"true"` / `"false"` の文字列でも受け付ける。
    Bool,
    /// 参照（`N12` の形）。
    Ref(char),
    /// 参照の配列。
    RefArray(char),
    /// 表に ある値のどれか。
    Enum(&'static [&'static str]),
    /// 写しの配列（1 要素がオブジェクト）。
    ObjArray,
}

/// op の 1 つの項目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// 項目の名前。
    pub key: &'static str,
    /// どの型であるべきか。
    pub kind: FieldKind,
    /// 省略できないか（検査は `check_*` がする。ここでは表に出すと schema の `required` になる）。
    pub required: bool,
    /// 「`cites` は文字列の配列**です**」の「です」の前の文。
    pub expect: &'static str,
    /// 「です」の後ろに足す例（例 `（例 ["#37 §3.2"]）`。無ければ空）。
    pub example: &'static str,
    /// この値だけは [`check_shapes`] で止めない値（ほかの検査をよりよい文で止める）。
    pub lenient: &'static [&'static str],
}

/// op の決まりの 1 つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpSpec {
    /// op の名前。
    pub op: &'static str,
    /// 何を確かめる操作かの説明（schema の `description` になる）。
    pub about: &'static str,
    /// この op の項目。
    pub fields: &'static [Field],
}

/// 省略できない項目（例つき）。
macro_rules! field {
    ($key:literal, $kind:expr, $expect:literal, $example:literal) => {
        Field {
            key: $key,
            kind: $kind,
            required: true,
            expect: $expect,
            example: $example,
            lenient: &[],
        }
    };
    ($key:literal, $kind:expr, $expect:literal) => {
        field!($key, $kind, $expect, "")
    };
}

/// 省略できる項目（例つき）。
macro_rules! opt_field {
    ($key:literal, $kind:expr, $expect:literal, $example:literal) => {
        Field {
            key: $key,
            kind: $kind,
            required: false,
            expect: $expect,
            example: $example,
            lenient: &[],
        }
    };
    ($key:literal, $kind:expr, $expect:literal) => {
        opt_field!($key, $kind, $expect, "")
    };
}

/// op ごとの項目（表の 1 行）。検査も schema もここだけを見る。
///
/// 線の種類は [`EDGE_KINDS`] と同じ値を使う。`depends` の向きは [`EDGE_DIRECTION`]
/// （`from` が上流、`to` が下流）。
///
/// `expect` が英字で始まるものは先頭に空白を付ける（日本語の文中に英字の列を書くときの体裁）。
pub const OP_FIELDS: &[OpSpec] = &[
    OpSpec {
        op: "add_finding",
        about: "ITEM に FINDING を足す。引用は cites に文字列の配列（例 [\"#37 §3.2\"]）で入れる",
        fields: &[
            field!(
                "node",
                FieldKind::Ref('N'),
                " ITEM・OPTION の NODE 参照（N12 の形）"
            ),
            opt_field!("question", FieldKind::Ref('Q'), " QUESTION の参照（Q7 の形）"),
            field!("text", FieldKind::Str, "1〜1200 字の文（3 文まで）"),
            field!(
                "cites",
                FieldKind::StrArray,
                "文字列の配列",
                "（例 [\"#37 §3.2\"]）"
            ),
            field!(
                "polarity",
                FieldKind::Enum(POLARITIES),
                " support / refute / neutral / none のどれか"
            ),
            opt_field!("level", FieldKind::Int, "1〜4 の数"),
            opt_field!("paper", FieldKind::Int, "論文の番号"),
            opt_field!("run", FieldKind::Int, "実行の番号"),
        ],
    },
    OpSpec {
        op: "add_question",
        about: "ITEM に QUESTION を足す",
        fields: &[
            field!(
                "node",
                FieldKind::Ref('N'),
                " ITEM・OPTION の NODE 参照（N12 の形）"
            ),
            field!("text", FieldKind::Str, "1〜300 字の QUESTION"),
        ],
    },
    OpSpec {
        op: "answer_question",
        about: "QUESTION に答えを書く",
        fields: &[
            field!("question", FieldKind::Ref('Q'), " QUESTION の参照（Q7 の形）"),
            field!("text", FieldKind::Str, "1〜1200 字の答え"),
        ],
    },
    OpSpec {
        op: "link",
        about: "ITEM を論文・ノート・報告・SEARCH_SET・READ_CARD に結ぶ。target は paper:37 のように種類:番号で書く",
        fields: &[
            field!(
                "node",
                FieldKind::Ref('N'),
                " ITEM・OPTION の NODE 参照（N12 の形）"
            ),
            field!(
                "target",
                FieldKind::Str,
                "結び付けの相手",
                "（paper:37 / report:R-0012 / set:5 / run:120）"
            ),
            opt_field!(
                "role",
                FieldKind::Enum(ROLES),
                " exemplar / compare / issue_evidence / method_source / data_source / background / counter のどれか（論文のときだけ）"
            ),
            opt_field!(
                "read_state",
                FieldKind::Enum(READ_STATES),
                " unread / read / reflected / irrelevant のどれか（論文のときだけ）"
            ),
        ],
    },
    OpSpec {
        op: "add_seed",
        about: "SEED（SEED_SENTENCE と題）を足す",
        fields: &[
            field!("title", FieldKind::Str, "1〜120 字の題"),
            field!("sentence", FieldKind::Str, "1〜600 字の一文"),
            opt_field!(
                "path",
                FieldKind::RefArray('N'),
                " NODE 参照の配列",
                "（[\"N11\"] の形）"
            ),
            opt_field!("doc", FieldKind::Str, "文書のパス"),
        ],
    },
    OpSpec {
        op: "add_option",
        about: "CHOOSE・LIST の ITEM に OPTION を足す",
        fields: &[
            field!("node", FieldKind::Ref('N'), " ITEM の NODE 参照（N12 の形）"),
            field!("title", FieldKind::Str, "1〜120 字の OPTION の題"),
            opt_field!("body", FieldKind::Str, "2000 字までの OPTION の本文"),
        ],
    },
    OpSpec {
        op: "add_child",
        about: "ITEM の下に子を足す。key を書くと題と ITEM_TYPE は ITEM_GUIDE から取る。key を書かないときは title と types で書く",
        fields: &[
            field!(
                "node",
                FieldKind::Ref('N'),
                " ITEM・OPTION の NODE 参照（N12 の形）"
            ),
            field!(
                "title",
                FieldKind::Str,
                "1〜120 字の題（key を書くときは省略できる）"
            ),
            field!(
                "types",
                FieldKind::StrArray,
                "文字列の配列",
                "（choose / list / write / verify）"
            ),
            opt_field!(
                "key",
                FieldKind::Str,
                " ITEM_GUIDE の ITEM キー（例 background.goal）"
            ),
        ],
    },
    OpSpec {
        op: "decide",
        about: "ITEM の状態を決める（DECIDED・TENTATIVE）。SELECTED にした OPTION・根拠にした FINDING・REJECTED にした OPTION を記録する",
        fields: &[
            field!("node", FieldKind::Ref('N'), " ITEM の NODE 参照（N12 の形）"),
            opt_field!(
                "option",
                FieldKind::Ref('N'),
                "選ぶ OPTION の NODE 参照（N21 の形）"
            ),
            opt_field!(
                "tentative",
                FieldKind::Bool,
                "真偽",
                "（一応の決めかどうか）"
            ),
            opt_field!(
                "evidence",
                FieldKind::RefArray('F'),
                "根拠にした FINDING の参照の配列",
                "（[\"F31\"] の形）"
            ),
            opt_field!(
                "rejected",
                FieldKind::ObjArray,
                " REJECTED にした OPTION の配列",
                "（[{\"option\": \"N21\", \"reason\": \"…\"}] の形）"
            ),
            opt_field!(
                "counter_searched",
                FieldKind::Bool,
                "真偽（REFUTE を探したかどうか）"
            ),
        ],
    },
    OpSpec {
        op: "reopen",
        about: "EXCLUDED にした ITEM を再開する",
        fields: &[field!(
            "node",
            FieldKind::Ref('N'),
            " ITEM の NODE 参照（N12 の形）"
        )],
    },
    OpSpec {
        op: "exclude_item",
        about: "ITEM を EXCLUDED にする",
        fields: &[field!(
            "node",
            FieldKind::Ref('N'),
            " ITEM の NODE 参照（N12 の形）"
        )],
    },
    OpSpec {
        op: "option_state",
        about: "OPTION の状態を変える（rejected / dormant / candidate。REJECTED・DORMANT・CANDIDATE）",
        fields: &[
            field!("node", FieldKind::Ref('N'), " OPTION の NODE 参照（N12 の形）"),
            field!(
                "state",
                FieldKind::Enum(OPTION_TARGET_STATES),
                " rejected / dormant / candidate のどれか"
            ),
        ],
    },
    OpSpec {
        op: "exclude",
        about: "論文・ノート・報告・SEARCH_SET・READ_CARD・OPTION を対象外にする",
        fields: &[
            field!(
                "target",
                FieldKind::Str,
                "外す相手",
                "（paper:37 / note:37 / report:R-0012 / set:5 / run:120 / candidate:重複キー）"
            ),
            opt_field!(
                "node",
                FieldKind::Ref('N'),
                "結び付ける ITEM の NODE 参照（N12 の形）"
            ),
            opt_field!("title", FieldKind::Str, "200 字までの題"),
        ],
    },
    OpSpec {
        op: "edit_text",
        about: "NODE の題か本文を直す。title と body のどちらかを 1 つ以上書く",
        fields: &[
            field!(
                "node",
                FieldKind::Ref('N'),
                " ITEM・OPTION の NODE 参照（N12 の形）"
            ),
            opt_field!("title", FieldKind::Str, "1〜120 字の題"),
            opt_field!("body", FieldKind::Str, "2000 字までの本文"),
        ],
    },
    OpSpec {
        op: "add_edge",
        about: "NODE どうしの線（depends＝DEPENDS 依存・derived＝DERIVED 由来・analogous＝ANALOGOUS 同型）を引く。kind に入れる値は depends などの小文字のまま。from が上流、to が下流（to のほうが from に依存する側）。parent（親子）は引けない",
        fields: &[
            Field {
                key: "kind",
                kind: FieldKind::Enum(EDGE_KINDS),
                required: true,
                expect: " depends / derived / analogous のどれか",
                example: "",
                // parent は引けないという専用の文を出すため、ここでは止めない。
                lenient: &["parent"],
            },
            field!("from", FieldKind::Ref('N'), " NODE の参照（N12 の形）"),
            field!("to", FieldKind::Ref('N'), " NODE の参照（N12 の形）"),
            opt_field!("note", FieldKind::Str, "200 字までの補足"),
        ],
    },
    OpSpec {
        op: "unlink",
        about: "LINK を外す",
        fields: &[field!(
            "link",
            FieldKind::Ref('L'),
            " LINK の参照（L3 の形）"
        )],
    },
    OpSpec {
        op: "remove_finding",
        about: "FINDING を消す",
        fields: &[field!(
            "finding",
            FieldKind::Ref('F'),
            " FINDING の参照（F31 の形）"
        )],
    },
    OpSpec {
        op: "set_read_state",
        about: "論文の LINK の READ_STATE を変える",
        fields: &[
            field!("link", FieldKind::Ref('L'), " LINK の参照（L3 の形）"),
            field!(
                "state",
                FieldKind::Enum(READ_STATES),
                " unread / read / reflected / irrelevant のどれか"
            ),
        ],
    },
    OpSpec {
        op: "withdraw_question",
        about: "QUESTION を消す",
        fields: &[field!(
            "question",
            FieldKind::Ref('Q'),
            " QUESTION の参照（Q7 の形）"
        )],
    },
    OpSpec {
        op: "remove_edge",
        about: "線を外す（向きは引いたときと同じ）",
        fields: &[
            Field {
                key: "kind",
                kind: FieldKind::Enum(EDGE_KINDS),
                required: true,
                expect: " depends / derived / analogous のどれか",
                example: "",
                lenient: &["parent"],
            },
            field!("from", FieldKind::Ref('N'), " NODE の参照（N12 の形）"),
            field!("to", FieldKind::Ref('N'), "下流の NODE 参照（N12 の形）"),
        ],
    },
];

/// op の表を 1 行探す。知らない op のときは `None`。
#[must_use]
pub fn op_spec(op: &str) -> Option<&'static OpSpec> {
    OP_FIELDS.iter().find(|spec| spec.op == op)
}

/// 提案 1 件の JSON Schema（MCP の `sci_propose` の `proposals.items` にする）。
///
/// 表（[`OP_FIELDS`]）から作る。提案に出せる op だけ（利用者だけの操作は除く）を
/// `oneOf` に入れ、`properties.op.const` で op を固定する。
#[must_use]
pub fn propose_schema() -> serde_json::Value {
    let mut one_of = Vec::new();
    for spec in OP_FIELDS {
        if USER_ONLY_OPS.contains(&spec.op) {
            continue;
        }
        let mut properties = serde_json::Map::new();
        properties.insert(
            "op".to_string(),
            serde_json::json!({
                "type": "string",
                "const": spec.op,
                "description": spec.about,
            }),
        );
        properties.insert(
            "reason".to_string(),
            serde_json::json!({
                "type": "string",
                "minLength": 1,
                "maxLength": 1000,
                "description": "なぜこの変更をするか（1〜1000 字）。PROPOSAL には必ず書く",
            }),
        );
        let mut required = vec!["op".to_string(), "reason".to_string()];
        for field in spec.fields {
            properties.insert(field.key.to_string(), field_schema(field));
            if field.required {
                required.push(field.key.to_string());
            }
        }
        one_of.push(serde_json::json!({
            "type": "object",
            "title": spec.op,
            "description": spec.about,
            "properties": properties,
            "required": required,
        }));
    }
    serde_json::json!({ "oneOf": one_of })
}

/// 項目 1 つの JSON Schema。
fn field_schema(field: &Field) -> serde_json::Value {
    match field.kind {
        // cites などの文字列の配列は、説明を書き足さず例そのものにする。
        FieldKind::StrArray => serde_json::json!({"type": "array", "items": {"type": "string"}}),
        FieldKind::Str => serde_json::json!({"type": "string", "description": say(field)}),
        FieldKind::Int => serde_json::json!({"type": "integer", "description": say(field)}),
        FieldKind::Bool => serde_json::json!({"type": "boolean", "description": say(field)}),
        FieldKind::Ref(prefix) => serde_json::json!({
            "type": "string",
            "pattern": ref_pattern(prefix),
            "description": say(field),
        }),
        FieldKind::RefArray(prefix) => serde_json::json!({
            "type": "array",
            "items": {"type": "string", "pattern": ref_pattern(prefix)},
            "description": say(field),
        }),
        FieldKind::Enum(values) => serde_json::json!({
            "type": "string",
            "enum": values,
            "description": say(field),
        }),
        FieldKind::ObjArray => serde_json::json!({
            "type": "array",
            "items": {"type": "object"},
            "description": say(field),
        }),
    }
}

/// 参照の形の正規表現（`parse_ref` と同じ `^[NQFLPS][1-9][0-9]{0,8}$`）。
fn ref_pattern(prefix: char) -> String {
    format!("^{prefix}[1-9][0-9]{{0,8}}$")
}

/// 提案 1 件の型を [`OP_FIELDS`] に従って確かめる（`check_*` の前に呼ぶ）。
///
/// ここで止めるのは「形が違う」だけ。無い項目・空・長すぎは、それぞれの `check_*` が
/// よりよい文で止める。表に無い項目は今どおり無視する。
pub fn check_shapes(op: &str, input: &serde_json::Value) -> Result<()> {
    let Some(spec) = op_spec(op) else {
        return Ok(());
    };
    for field in spec.fields {
        // 無印（null）は「無い」と同じ扱いにする。
        let Some(value) = input.get(field.key).filter(|value| !value.is_null()) else {
            continue;
        };
        if let Some(error) = shape_error(field, value) {
            return Err(SciError::Invalid(error));
        }
    }
    Ok(())
}

/// 項目 1 つの型が表と違えば、エラーの文を返す。違わなければ `None`。
fn shape_error(field: &Field, value: &serde_json::Value) -> Option<String> {
    match field.kind {
        FieldKind::Str => match value {
            serde_json::Value::String(_) => None,
            other => Some(shape_message(field, "形", shape_name(other))),
        },
        FieldKind::StrArray => match value {
            serde_json::Value::Array(items) => items
                .iter()
                .find(|item| !item.is_string())
                .map(|item| shape_message(field, "形", shape_name(item))),
            other => Some(shape_message(field, "形", shape_name(other))),
        },
        FieldKind::Int => match value {
            serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => None,
            serde_json::Value::Number(number) => {
                Some(shape_message(field, "値", &number.to_string()))
            }
            serde_json::Value::String(text) if text.trim().parse::<i64>().is_ok() => None,
            other => Some(shape_message(field, "形", shape_name(other))),
        },
        FieldKind::Bool => match value {
            serde_json::Value::Bool(_) => None,
            serde_json::Value::String(text) if matches!(text.trim(), "true" | "false") => None,
            other => Some(shape_message(field, "形", shape_name(other))),
        },
        FieldKind::Ref(prefix) => match value {
            serde_json::Value::String(text) if is_ref(text, prefix) => None,
            serde_json::Value::String(text) => {
                Some(shape_message(field, "値", &format!("\"{}\"", text.trim())))
            }
            other => Some(shape_message(field, "形", shape_name(other))),
        },
        FieldKind::RefArray(prefix) => match value {
            serde_json::Value::Array(items) => items
                .iter()
                .find(|item| match item.as_str() {
                    Some(text) => !is_ref(text, prefix),
                    None => true,
                })
                .map(|item| match item.as_str() {
                    Some(text) => shape_message(field, "値", &format!("\"{}\"", text.trim())),
                    None => shape_message(field, "形", shape_name(item)),
                }),
            other => Some(shape_message(field, "形", shape_name(other))),
        },
        FieldKind::Enum(values) => match value {
            serde_json::Value::String(text)
                if values.contains(&text.trim()) || field.lenient.contains(&text.trim()) =>
            {
                None
            }
            serde_json::Value::String(text) => {
                Some(shape_message(field, "値", &format!("\"{}\"", text.trim())))
            }
            other => Some(shape_message(field, "形", shape_name(other))),
        },
        FieldKind::ObjArray => match value {
            serde_json::Value::Array(items) => items
                .iter()
                .find(|item| !item.is_object())
                .map(|item| shape_message(field, "形", shape_name(item))),
            other => Some(shape_message(field, "形", shape_name(other))),
        },
    }
}

/// 参照の形かどうか（`parse_ref` と同じ）。
fn is_ref(text: &str, prefix: char) -> bool {
    let mut chars = text.trim().chars();
    let Some(head) = chars.next() else {
        return false;
    };
    let digits: String = chars.collect();
    head == prefix
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && matches!(digits.chars().next(), Some('1'..='9'))
        && (1..=999_999_999).contains(&digits.parse::<i64>().unwrap_or(0))
}

/// 項目が「何であるべきか」を 1 文にする（schema の `description` にする）。
fn say(field: &Field) -> String {
    format!("{}{}です。", field.expect, field.example)
}

/// 「cites は文字列の配列です（例 ["#37 §3.2"]）。受け取った形: オブジェクト」の文。
fn shape_message(field: &Field, label: &str, got: &str) -> String {
    format!(
        "{} は{}です{}。受け取った{}: {}",
        field.key, field.expect, field.example, label, got
    )
}

/// 受け取った JSON の形を日本語で言う。
fn shape_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "無印（null）",
        serde_json::Value::Bool(_) => "真偽",
        serde_json::Value::Number(_) => "数",
        serde_json::Value::String(_) => "文字列",
        serde_json::Value::Array(_) => "配列",
        serde_json::Value::Object(_) => "オブジェクト",
    }
}

// ---- 公開の型 ---------------------------------------------------------------

/// 操作の出所。人の操作なら [`Actor::User`]、エージェントの提案なら [`Actor::Run`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// 利用者。
    User,
    /// エージェントの実行。`run_id` が分からないときは `None`（`run:?` と書く）。
    Run {
        /// sai-link の実行 ID。
        run_id: Option<i64>,
        /// 出したブロックの id。
        block: String,
    },
}

/// 仕分けの操作。
#[derive(Debug, Clone, PartialEq)]
pub enum TriageAction {
    /// 採用する。`edit` があれば操作の中身を直してから検査する。
    Accept {
        /// payload に書き重ねる差し替え。無い項目はそのまま。
        edit: Option<serde_json::Value>,
    },
    /// 却下する。
    Reject,
    /// 取り消す（自動で反映したものを外す）。
    Revert,
}

/// 提案 1 件の答え（C-07 の「応答」）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProposalResult {
    /// 提案の番号（`P55`）。
    pub proposal: String,
    /// する操作。
    pub op: String,
    /// `applied`（自動で反映）か `pending`（仕分け待ち）。
    pub status: String,
    /// 反映した結果の参照（例 `F31`）。無いときは `None`。
    pub result: Option<String>,
}

/// 人の操作 1 件の答え。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApplyResult {
    /// した操作。
    pub op: String,
    /// 結果の参照（例 `N12`）。表せないときは `None`。
    pub result: Option<String>,
}

/// 仕分け 1 件の答え。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TriageResult {
    /// 仕分けた提案の番号（`P56`）。
    pub proposal: String,
    /// `accepted` / `rejected` / `reverted`。
    pub status: String,
    /// 結果の参照。無いときは `None`。
    pub result: Option<String>,
}

// ---- 入口 -------------------------------------------------------------------

/// ブロック定義（`<root>/.sai/blocks/<id>.md`、無ければ `~/.sai-moon/blocks/<id>.md`）が
/// 提案できる操作（frontmatter の `sci_ops`）を返す。
///
/// ブロックが分からない・`sci_ops` を書いていないときは「自動で入る種類」だけ。
#[must_use]
pub fn allowed_ops(roots: &PaperRoots, block: &str) -> Vec<String> {
    if block.is_empty() {
        return AUTO_OPS.iter().map(|op| (*op).to_string()).collect();
    }
    for path in block_def_paths(roots, block) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let found = crate::runs::read_frontmatter(&text)
            .into_iter()
            .find(|(key, _)| key == "sci_ops")
            .map_or_else(Vec::new, |(_, value)| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_string)
                    .collect()
            });
        if found.is_empty() {
            return AUTO_OPS.iter().map(|op| (*op).to_string()).collect();
        }
        return found;
    }
    AUTO_OPS.iter().map(|op| (*op).to_string()).collect()
}

/// 読むブロック定義の候補（プロジェクトを優先し、共有の置き場がその次）。
///
/// ブロックの ID は外から来る値なので、ファイル名にする前に
/// `^[a-z0-9-]+$` に通ることを確かめる。通らないときは空を返す
/// （`../` のような値で `.sai/blocks/` の外を読まないため）。
pub(crate) fn block_def_paths(roots: &PaperRoots, block: &str) -> Vec<std::path::PathBuf> {
    if !is_block_id(block) {
        return Vec::new();
    }
    let mut candidates = vec![roots
        .root
        .join(".sai")
        .join("blocks")
        .join(format!("{block}.md"))];
    if let Some(dir) = crate::paths::sai_home_path("blocks") {
        candidates.push(dir.join(format!("{block}.md")));
    }
    candidates
}

/// ブロックの ID として使える形かどうか（`^[a-z0-9-]+$`）。
pub(crate) fn is_block_id(block: &str) -> bool {
    !block.is_empty()
        && block
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 提案する（C-07 の「提案」）。
///
/// `input` は `op` を持つ 1 つの JSON か、`{"proposals": [ … ]}`（20 件まで）。
/// 全部を先に検査し、1 つでも通らなければ何も保存せず [`SciError::Invalid`] を返す。
/// 検査の文には `n 件目: ` を頭に付ける。
pub fn propose(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    actor: &Actor,
    input: &serde_json::Value,
) -> Result<Vec<ProposalResult>> {
    let (items, is_batch) = read_proposal_list(input)?;
    let block = match actor {
        Actor::Run { block, .. } => block.clone(),
        Actor::User => String::new(),
    };
    let ctx = Ctx {
        conn,
        roots,
        defs,
        nodes: RefCell::new(HashMap::new()),
        from_proposal: true,
        allowed: allowed_ops(roots, &block),
        actor: actor.clone(),
        decision_actor: actor_text(actor),
    };

    // 先に全部を検査する（ここでは何も書かない）。まとめて出したときだけ「何件目か」を付ける。
    let mut plans = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let index = is_batch.then_some(index);
        plans.push(ctx.check(item).map_err(|error| with_index(index, error))?);
    }

    let tx = conn.unchecked_transaction()?;
    let mut writer = Ctx { conn: &tx, ..ctx };
    let mut results = Vec::with_capacity(plans.len());
    for (plan, item) in plans.iter().zip(items.iter()) {
        let risk = risk_of(plan.op())?;
        let status = if risk == "auto" { "applied" } else { "pending" };
        let run_id = match actor {
            Actor::Run { run_id, .. } => *run_id,
            Actor::User => None,
        };
        let proposal_id = store::insert_proposal(
            &tx,
            &NewProposal {
                run_id,
                block: &block,
                op: plan.op(),
                node_id: plan.node_id(),
                payload: item,
                reason: &text_of(item, "reason"),
                risk,
                status,
            },
        )?;
        // 自動で入るものだけ、その場で反映する。
        let result = if status == "applied" {
            writer.write(plan, item, Some(proposal_id), true)?
        } else {
            None
        };
        if let Some(found) = &result {
            store::finish_proposal(&tx, proposal_id, "applied", Some(found), None, "")?;
        }
        results.push(ProposalResult {
            proposal: fmt_ref('P', proposal_id),
            op: plan.op().to_string(),
            status: status.to_string(),
            result,
        });
    }
    tx.commit()?;
    Ok(results)
}

/// 利用者の操作 1 件を反映する（C-07 の「人の操作」）。
pub fn apply_user(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    input: &serde_json::Value,
) -> Result<ApplyResult> {
    let ctx = Ctx {
        conn,
        roots,
        defs,
        nodes: RefCell::new(HashMap::new()),
        from_proposal: false,
        allowed: Vec::new(),
        actor: Actor::User,
        decision_actor: "user".to_string(),
    };
    let plan = ctx.check(input)?;

    let tx = conn.unchecked_transaction()?;
    let mut writer = Ctx { conn: &tx, ..ctx };
    let result = writer.write(&plan, input, None, false)?;
    tx.commit()?;
    Ok(ApplyResult {
        op: text_of(input, "op"),
        result,
    })
}

/// 仕分け箱で提案を 1 つ扱う（C-07 の「仕分け」）。
///
/// `accept` は `pending` だけ、`reject` は `pending` だけ、`revert` は `applied` だけを受け付ける。
/// 採用のときは中身（ID・字数・出典）を検査し直す。ブロックの `sci_ops` は検査し直さない
/// （提案のときに済んでおり、採用は利用者の判断だから）。
pub fn triage(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    proposal_id: i64,
    action: TriageAction,
    note: &str,
) -> Result<TriageResult> {
    let proposal = store::get_proposal(conn, proposal_id)?;
    let note = take(note, 1000);
    let label = fmt_ref('P', proposal_id);

    match action {
        TriageAction::Reject => {
            if proposal.status != "pending" {
                return Err(SciError::Invalid(format!(
                    "この PROPOSAL は {} なので却下できません",
                    status_name(&proposal.status)
                )));
            }
            let tx = conn.unchecked_transaction()?;
            store::finish_proposal(
                &tx,
                proposal_id,
                "rejected",
                proposal.result_ref.as_deref(),
                Some("user"),
                &note,
            )?;
            tx.commit()?;
            Ok(TriageResult {
                proposal: label,
                status: "rejected".to_string(),
                result: proposal.result_ref,
            })
        }
        TriageAction::Revert => {
            if proposal.status != "applied" {
                return Err(SciError::Invalid(format!(
                    "この PROPOSAL は {} なので取り消せません",
                    status_name(&proposal.status)
                )));
            }
            let tx = conn.unchecked_transaction()?;
            revert(&tx, &proposal)?;
            store::finish_proposal(
                &tx,
                proposal_id,
                "reverted",
                proposal.result_ref.as_deref(),
                Some("user"),
                &note,
            )?;
            tx.commit()?;
            Ok(TriageResult {
                proposal: label,
                status: "reverted".to_string(),
                result: proposal.result_ref,
            })
        }
        TriageAction::Accept { edit } => {
            if proposal.status != "pending" {
                return Err(SciError::Invalid(format!(
                    "この PROPOSAL は {} なので採用できません",
                    status_name(&proposal.status)
                )));
            }
            let payload = merge_payload(&proposal.payload, edit.as_ref());
            // 採用した操作は、作ったものは提案を出した実行のもの。決定の記録は利用者。
            let actor = Actor::Run {
                run_id: proposal.run_id,
                block: proposal.block.clone(),
            };
            let ctx = Ctx {
                conn,
                roots,
                defs,
                nodes: RefCell::new(HashMap::new()),
                from_proposal: false,
                allowed: Vec::new(),
                actor: actor.clone(),
                decision_actor: "user".to_string(),
            };
            let plan = ctx.check(&payload)?;
            if plan.op() != proposal.op {
                return Err(SciError::Invalid(format!(
                    "採用するときの操作は変えられません（PROPOSAL は {}、直したものは {}）",
                    proposal.op,
                    plan.op()
                )));
            }
            let tx = conn.unchecked_transaction()?;
            let mut writer = Ctx { conn: &tx, ..ctx };
            let result = writer.write(&plan, &payload, Some(proposal_id), false)?;
            store::finish_proposal(
                &tx,
                proposal_id,
                "accepted",
                result.as_deref(),
                Some("user"),
                &note,
            )?;
            tx.commit()?;
            Ok(TriageResult {
                proposal: label,
                status: "accepted".to_string(),
                result,
            })
        }
    }
}

// ---- 検査済みの操作 ---------------------------------------------------------

/// 検査を通した 1 件の操作。中身はすべて検査済みの値。
#[derive(Debug, Clone, PartialEq)]
enum Plan {
    AddFinding {
        node: i64,
        question: Option<i64>,
        text: String,
        cites: Vec<String>,
        polarity: String,
        level: Option<i64>,
        paper: Option<i64>,
        run: Option<i64>,
    },
    AddQuestion {
        node: i64,
        text: String,
    },
    AnswerQuestion {
        question: i64,
        text: String,
    },
    Link {
        node: i64,
        target_kind: String,
        target_ref: String,
        role: Option<String>,
        read_state: String,
    },
    AddSeed {
        title: String,
        sentence: String,
        path: Vec<i64>,
        doc: String,
    },
    AddOption {
        node: i64,
        title: String,
        body: String,
    },
    AddChild {
        node: i64,
        title: String,
        types: Vec<String>,
        key: Option<String>,
    },
    Decide {
        node: i64,
        option: Option<i64>,
        tentative: bool,
        evidence: Vec<i64>,
        rejected: Vec<RejectedOption>,
        counter_searched: bool,
    },
    Reopen {
        node: i64,
    },
    ExcludeItem {
        node: i64,
    },
    OptionState {
        node: i64,
        state: String,
    },
    Exclude {
        target_kind: String,
        target_ref: String,
        node: Option<i64>,
        title: String,
    },
    EditText {
        node: i64,
        title: Option<String>,
        body: Option<String>,
    },
    AddEdge {
        kind: String,
        from: i64,
        to: i64,
        note: String,
    },
    Unlink {
        link: i64,
    },
    RemoveFinding {
        finding: i64,
    },
    SetReadState {
        link: i64,
        state: String,
    },
    WithdrawQuestion {
        question: i64,
    },
    RemoveEdge {
        kind: String,
        from: i64,
        to: i64,
    },
}

impl Plan {
    /// 操作の名前。
    fn op(&self) -> &'static str {
        match self {
            Self::AddFinding { .. } => "add_finding",
            Self::AddQuestion { .. } => "add_question",
            Self::AnswerQuestion { .. } => "answer_question",
            Self::Link { .. } => "link",
            Self::AddSeed { .. } => "add_seed",
            Self::AddOption { .. } => "add_option",
            Self::AddChild { .. } => "add_child",
            Self::Decide { .. } => "decide",
            Self::Reopen { .. } => "reopen",
            Self::ExcludeItem { .. } => "exclude_item",
            Self::OptionState { .. } => "option_state",
            Self::Exclude { .. } => "exclude",
            Self::EditText { .. } => "edit_text",
            Self::AddEdge { .. } => "add_edge",
            Self::Unlink { .. } => "unlink",
            Self::RemoveFinding { .. } => "remove_finding",
            Self::SetReadState { .. } => "set_read_state",
            Self::WithdrawQuestion { .. } => "withdraw_question",
            Self::RemoveEdge { .. } => "remove_edge",
        }
    }

    /// 対象の節点（`sci_proposals.node_id` に入れる）。
    fn node_id(&self) -> Option<i64> {
        match self {
            Self::AddFinding { node, .. }
            | Self::AddQuestion { node, .. }
            | Self::Link { node, .. }
            | Self::AddOption { node, .. }
            | Self::AddChild { node, .. }
            | Self::Decide { node, .. }
            | Self::Reopen { node }
            | Self::ExcludeItem { node }
            | Self::OptionState { node, .. }
            | Self::EditText { node, .. } => Some(*node),
            Self::AddEdge { from, .. } | Self::RemoveEdge { from, .. } => Some(*from),
            Self::Exclude { node, .. } => *node,
            _ => None,
        }
    }
}

/// 検査と反映で使う共通の状態。
struct Ctx<'a> {
    /// DB への接続。反映のときはトランザクションを指す。
    conn: &'a Connection,
    /// プロジェクトルート（読む札の検索と、ブロック定義の置き場）。
    roots: &'a PaperRoots,
    /// 型と項目ガイド（`add_child` の `key` 用）。
    defs: &'a Defs,
    /// 読んだ節点の入れ物。同じ節点を何度も読み直さないため。
    nodes: RefCell<HashMap<i64, Node>>,
    /// 提案として出すか（人の操作・採用なら `false`）。
    from_proposal: bool,
    /// 提案のとき、そのブロックが提案できる操作。
    allowed: Vec<String>,
    /// 出した側。作ったものの `created_by` と、決定の記録の `actor` になる。
    actor: Actor,
    /// 決定の記録に入れる実行者。利用者の操作と採用は `user`。
    decision_actor: String,
}

impl<'a> Ctx<'a> {
    /// 節点を 1 つ読む。無ければ `節点 N<id> がありません`。
    fn node(&self, id: i64) -> Result<Node> {
        if let Some(found) = self.nodes.borrow().get(&id) {
            return Ok(found.clone());
        }
        let node = store::get_node(self.conn, id).map_err(|error| match error {
            SciError::NotFound(message) => SciError::Invalid(message),
            other => other,
        })?;
        self.nodes.borrow_mut().insert(id, node.clone());
        Ok(node)
    }

    /// `node` を読む。項目でも候補でもよい操作のとき用。
    fn node_input(&self, input: &serde_json::Value, key: &str) -> Result<Node> {
        let id = self.ref_input(input, key, 'N')?;
        self.node(id)
    }

    /// 項目を読む（候補の操作には使わない）。
    fn item_input(&self, input: &serde_json::Value, key: &str) -> Result<Node> {
        let node = self.node_input(input, key)?;
        if node.kind != "item" {
            return Err(SciError::Invalid(format!(
                "{key} は OPTION ではなく ITEM でなければなりません: {}",
                fmt_ref('N', node.id)
            )));
        }
        Ok(node)
    }

    /// 候補を読む。
    fn option_input(&self, input: &serde_json::Value, key: &str) -> Result<Node> {
        let node = self.node_input(input, key)?;
        if node.kind != "option" {
            return Err(SciError::Invalid(format!(
                "{key} は ITEM ではなく OPTION でなければなりません: {}",
                fmt_ref('N', node.id)
            )));
        }
        Ok(node)
    }

    /// `key` の節点を読み、種類が `kinds` のどれかであることを確かめる。
    fn node_input_kinds(
        &self,
        input: &serde_json::Value,
        key: &str,
        kinds: &[&str],
    ) -> Result<Node> {
        let node = self.node_input(input, key)?;
        if !kinds.contains(&node.kind.as_str()) {
            let names = kinds.join("か");
            return Err(SciError::Invalid(format!(
                "{key} は {names} でなければなりません: {}",
                fmt_ref('N', node.id)
            )));
        }
        Ok(node)
    }

    /// `key` の ID を読む（`N12` のような形）。
    fn ref_input(&self, input: &serde_json::Value, key: &str, prefix: char) -> Result<i64> {
        let text = text_of(input, key);
        if text.is_empty() {
            return Err(SciError::Invalid(format!(
                "{key}（{prefix}1 のような ID）を指定してください"
            )));
        }
        parse_ref(&text, prefix)
    }

    /// 問いを読む（`Q7`）。
    fn question_input(&self, input: &serde_json::Value, key: &str) -> Result<store::Question> {
        let id = self.ref_input(input, key, 'Q')?;
        store::get_question(self.conn, id).map_err(|error| match error {
            SciError::NotFound(message) => SciError::Invalid(message),
            other => other,
        })
    }

    /// 結び付きを読む（`L3`）。
    fn link_input(&self, input: &serde_json::Value, key: &str) -> Result<store::Link> {
        let id = self.ref_input(input, key, 'L')?;
        store::get_link(self.conn, id).map_err(|error| match error {
            SciError::NotFound(message) => SciError::Invalid(message),
            other => other,
        })
    }

    /// 知見を読む（`F31`）。
    fn finding_input(&self, input: &serde_json::Value, key: &str) -> Result<store::Finding> {
        let id = self.ref_input(input, key, 'F')?;
        store::get_finding(self.conn, id).map_err(|error| match error {
            SciError::NotFound(message) => SciError::Invalid(message),
            other => other,
        })
    }

    /// 論文が Library にあるか。
    fn paper_exists(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM papers WHERE id = ?1",
                params![id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// 検索セットがあるか。
    fn search_set_exists(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM search_sets WHERE id = ?1",
                params![id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// 報告があるか。
    fn report_exists(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM reports WHERE id = ?1", params![id], |_| {
                Ok(())
            })
            .optional()?
            .is_some())
    }

    /// 読む札（sai-link の実行フォルダ）ががあるか。
    fn run_exists(&self, id: i64) -> bool {
        crate::runs::list_run_dirs(&self.roots.root)
            .iter()
            .any(|run| run.run_id == id)
    }

    /// 引用を検査する。形が違う・番号の論文が Library に無ければ [`SciError::Invalid`]。
    fn check_cites(&self, cites: &[String]) -> Result<()> {
        if cites.len() > 10 {
            return Err(SciError::Invalid("引用は 10 個までです".to_string()));
        }
        for cite in cites {
            let Some(found) = CITE_PATTERN.captures(cite) else {
                return Err(SciError::Invalid(format!(
                    "引用は #<論文番号> の形（例 #37 §3.2）で書いてください: {cite}"
                )));
            };
            let number: i64 = found[1]
                .parse()
                .map_err(|_| SciError::Invalid(format!("引用の論文番号が読めません: {cite}")))?;
            if !self.paper_exists(number)? {
                return Err(SciError::Invalid(format!(
                    "論文 {number} が Library にありません"
                )));
            }
        }
        Ok(())
    }

    /// 1 件の操作を検査して [`Plan`] にする。
    fn check(&self, input: &serde_json::Value) -> Result<Plan> {
        if !input.is_object() {
            return Err(SciError::Invalid("PROPOSAL の形が正しくありません".to_string()));
        }
        let op = text_of(input, "op");
        if op.is_empty() {
            return Err(SciError::Invalid(
                "op（操作）を指定してください".to_string(),
            ));
        }
        if !KNOWN_OPS.contains(&op.as_str()) {
            return Err(SciError::Invalid(format!("知らない操作です: {op}")));
        }

        // 提案のときの追加の検査。
        if self.from_proposal {
            if USER_ONLY_OPS.contains(&op.as_str()) {
                return Err(SciError::Invalid(format!(
                    "{op} は PROPOSAL にできません（利用者だけの操作です）"
                )));
            }
            if !self.allowed.iter().any(|allowed| allowed == &op) {
                return Err(SciError::Invalid(format!(
                    "このブロックは {op} を PROPOSAL にできません"
                )));
            }
            self.check_reason(input)?;
        } else if REASON_REQUIRED_OPS.contains(&op.as_str()) && text_of(input, "reason").is_empty()
        {
            return Err(SciError::Invalid(format!(
                "{op} には reason（理由）が要ります"
            )));
        }

        // 型を先に確かめる（check_* は、型の違う値を空として黙って扱うため）。
        check_shapes(&op, input)?;

        match op.as_str() {
            "add_finding" => self.check_add_finding(input),
            "add_question" => self.check_add_question(input),
            "answer_question" => self.check_answer_question(input),
            "link" => self.check_link(input),
            "add_seed" => self.check_add_seed(input),
            "add_option" => self.check_add_option(input),
            "add_child" => self.check_add_child(input),
            "decide" => self.check_decide(input),
            "reopen" => self.check_reopen(input),
            "exclude_item" => self.check_exclude_item(input),
            "option_state" => self.check_option_state(input),
            "exclude" => self.check_exclude(input),
            "edit_text" => self.check_edit_text(input),
            "add_edge" => self.check_add_edge(input),
            "unlink" => self.check_unlink(input),
            "remove_finding" => self.check_remove_finding(input),
            "set_read_state" => self.check_set_read_state(input),
            "withdraw_question" => self.check_withdraw_question(input),
            "remove_edge" => self.check_remove_edge(input),
            _ => Err(SciError::Invalid(format!("知らない操作です: {op}"))),
        }
    }

    /// 提案の reason（1〜1000 字）。
    fn check_reason(&self, input: &serde_json::Value) -> Result<String> {
        let reason = text_of(input, "reason");
        if reason.is_empty() {
            return Err(SciError::Invalid(
                "reason（理由）を書いてください".to_string(),
            ));
        }
        if reason.chars().count() > 1000 {
            return Err(SciError::Invalid(
                "reason（理由）は 1000 字までです".to_string(),
            ));
        }
        Ok(reason)
    }

    fn check_add_finding(&self, input: &serde_json::Value) -> Result<Plan> {
        let node = self.node_input_kinds(input, "node", &["item", "option"])?;
        let question = self.opt_question(input, "question")?;
        let text = text_of(input, "text");
        check_len(&text, 1, 1200, "text", "FINDING")?;
        let sentences = sentence_count(&text);
        if sentences > 3 {
            return Err(SciError::Invalid(format!(
                "FINDING は 3 文までにしてください（今は {sentences} 文）"
            )));
        }
        let cites = text_list(input, "cites");
        self.check_cites(&cites)?;
        if self.from_proposal && cites.is_empty() {
            return Err(SciError::Invalid(
                "エージェントの FINDING には引用が要ります".to_string(),
            ));
        }
        if cites.is_empty() && text.chars().any(|ch| ch.is_ascii_digit()) {
            return Err(SciError::Invalid(
                "数値を含む FINDING には出典（CITE）が要ります".to_string(),
            ));
        }
        let polarity = text_of(input, "polarity");
        if !POLARITIES.contains(&polarity.as_str()) {
            return Err(SciError::Invalid(format!(
                "知らない向きです: {polarity}（support / refute / neutral / none）"
            )));
        }
        let level = opt_int(input, "level");
        if let Some(level) = level {
            if !(1..=4).contains(&level) {
                return Err(SciError::Invalid(format!(
                    "強さは 1〜4 で指定してください: {level}"
                )));
            }
        }
        let paper = self.opt_paper(input, "paper")?;
        let run = if self.from_proposal {
            None
        } else {
            self.opt_run(input, "run")?
        };
        Ok(Plan::AddFinding {
            node: node.id,
            question,
            text,
            cites,
            polarity,
            level,
            paper,
            run,
        })
    }

    fn check_add_question(&self, input: &serde_json::Value) -> Result<Plan> {
        let node = self.node_input_kinds(input, "node", &["item", "option"])?;
        let text = text_of(input, "text");
        check_len(&text, 1, 300, "text", "QUESTION")?;
        Ok(Plan::AddQuestion {
            node: node.id,
            text,
        })
    }

    fn check_answer_question(&self, input: &serde_json::Value) -> Result<Plan> {
        let question = self.question_input(input, "question")?;
        let text = text_of(input, "text");
        check_len(&text, 1, 1200, "text", "答え")?;
        Ok(Plan::AnswerQuestion {
            question: question.id,
            text,
        })
    }

    fn check_link(&self, input: &serde_json::Value) -> Result<Plan> {
        let node = self.node_input_kinds(input, "node", &["item", "option"])?;
        let (target_kind, target_ref) = self.check_target(&text_of(input, "target"), false)?;
        let role = opt_text(input, "role");
        let mut read_state = String::new();
        if target_kind == "paper" {
            let Some(found) = role.as_deref() else {
                return Err(SciError::Invalid(
                    "論文を結ぶときは role（役割）を指定してください".to_string(),
                ));
            };
            if !ROLES.contains(&found) {
                return Err(SciError::Invalid(format!(
                    "知らない ROLE です: {found}"
                )));
            }
            read_state = opt_text(input, "read_state").unwrap_or_else(|| "unread".to_string());
            if !READ_STATES.contains(&read_state.as_str()) {
                return Err(SciError::Invalid(format!(
                    "知らない READ_STATE です: {read_state}"
                )));
            }
        }
        // 論文でない結び付けに役割は入れない（DB の制約にもそう書いてある）。
        let role = target_kind.eq("paper").then_some(role).flatten();
        let read_state = if read_state.is_empty() {
            "unread".to_string()
        } else {
            read_state
        };
        Ok(Plan::Link {
            node: node.id,
            target_kind,
            target_ref,
            role,
            read_state,
        })
    }

    /// `paper:37` / `report:R-0012` / `note:37` / `set:5` / `run:120` を確かめる。
    /// `exclude` のときは `candidate:<dedupe_key>` も受け付ける。
    fn check_target(&self, target: &str, allow_candidate: bool) -> Result<(String, String)> {
        if target.is_empty() {
            return Err(SciError::Invalid(
                "target（結び付ける相手）を指定してください".to_string(),
            ));
        }
        let Some((kind, rest)) = target.split_once(':') else {
            return Err(SciError::Invalid(format!(
                "target は paper:37 のように、種類:番号の形にしてください: {target}"
            )));
        };
        let rest = rest.trim();
        if rest.is_empty() {
            return Err(SciError::Invalid(format!(
                "target の番号が空です: {target}"
            )));
        }
        // 候補は外すときだけ指定できる（重複キーが数ではない）。
        if kind == "candidate" {
            if !allow_candidate {
                return Err(SciError::Invalid(
                    "OPTION は exclude のときだけ指定できます".to_string(),
                ));
            }
            check_len(rest, 1, 200, "target", "OPTION の重複キー")?;
            return Ok(("candidate".to_string(), rest.to_string()));
        }
        if kind == "report" {
            if !self.report_exists(rest)? {
                return Err(SciError::Invalid(format!("報告 {rest} がありません")));
            }
            return Ok((kind.to_string(), rest.to_string()));
        }
        let number: i64 = rest
            .parse()
            .map_err(|_| SciError::Invalid(format!("target の番号が読めません: {target}")))?;
        if number <= 0 {
            return Err(SciError::Invalid(format!(
                "target の番号が読めません: {target}"
            )));
        }
        match kind {
            "paper" | "note" => {
                if !self.paper_exists(number)? {
                    return Err(SciError::Invalid(format!(
                        "論文 {number} が Library にありません"
                    )));
                }
            }
            "set" => {
                if !self.search_set_exists(number)? {
                    return Err(SciError::Invalid(format!(
                        "検索セット {number} がありません"
                    )));
                }
            }
            "run" => {
                if !self.run_exists(number) {
                    return Err(SciError::Invalid(format!("読む札 {number} がありません")));
                }
            }
            _ => {
                return Err(SciError::Invalid(format!(
                    "target の種類が分かりません: {kind}（paper / report / note / set / run）"
                )));
            }
        }
        Ok((kind.to_string(), rest.to_string()))
    }

    fn check_add_seed(&self, input: &serde_json::Value) -> Result<Plan> {
        let title = text_of(input, "title");
        check_len(&title, 1, 120, "title", "SEED の題名")?;
        let sentence = text_of(input, "sentence");
        check_len(&sentence, 1, 600, "sentence", "SEED_SENTENCE")?;
        let mut path = Vec::new();
        for (index, value) in text_list(input, "path").into_iter().enumerate() {
            let id = parse_ref(&value, 'N').map_err(|_| {
                SciError::Invalid(format!(
                    "path の {} 番目は N の形で指定してください: {value}",
                    index + 1
                ))
            })?;
            self.node(id)?;
            path.push(id);
        }
        Ok(Plan::AddSeed {
            title,
            sentence,
            path,
            doc: opt_text(input, "doc").unwrap_or_default(),
        })
    }

    fn check_add_option(&self, input: &serde_json::Value) -> Result<Plan> {
        let node = self.item_input(input, "node")?;
        if !node
            .types
            .iter()
            .any(|found| found == "choose" || found == "list")
        {
            return Err(SciError::Invalid(format!(
                "OPTION を足せるのは、CHOOSE・LIST の ITEM だけです: {}",
                fmt_ref('N', node.id)
            )));
        }
        let title = text_of(input, "title");
        check_len(&title, 1, 120, "title", "OPTION の題名")?;
        let body = opt_text(input, "body").unwrap_or_default();
        check_len(&body, 0, 2000, "body", "OPTION の本文")?;
        Ok(Plan::AddOption {
            node: node.id,
            title,
            body,
        })
    }

    fn check_add_child(&self, input: &serde_json::Value) -> Result<Plan> {
        let node = self.node_input_kinds(input, "node", &["item", "option"])?;
        let key = opt_text(input, "key");
        // key がありそのガイドがあれば、題と型はガイドから取る。
        if let Some(found) = key.as_deref().and_then(|key| self.defs.guide(key)) {
            return Ok(Plan::AddChild {
                node: node.id,
                title: found.name.clone(),
                types: found.types.clone(),
                key,
            });
        }
        let title = text_of(input, "title");
        check_len(&title, 1, 120, "title", "ITEM の題名")?;
        let types = text_list(input, "types");
        if types.is_empty() {
            return Err(SciError::Invalid(
                "types（ITEM_TYPE）を 1 つ以上指定してください（choose / list / write / verify）"
                    .to_string(),
            ));
        }
        for found in &types {
            if !TYPES.contains(&found.as_str()) {
                return Err(SciError::Invalid(format!("知らない ITEM_TYPE です: {found}")));
            }
        }
        Ok(Plan::AddChild {
            node: node.id,
            title,
            types,
            key,
        })
    }

    fn check_decide(&self, input: &serde_json::Value) -> Result<Plan> {
        let node = self.item_input(input, "node")?;
        let option = match opt_text(input, "option") {
            Some(text) => Some(parse_ref(&text, 'N')?),
            None => None,
        };
        if let Some(option) = option {
            self.require_child_option(node.id, option, "option")?;
        }
        // 選ぶ型で候補があれば、選ぶ候補を指定してください。
        if option.is_none()
            && node.types.iter().any(|found| found == "choose")
            && self.child_options(node.id)?.iter().any(|child| {
                matches!(
                    store::get_node(self.conn, *child).map(|child_node| child_node.state),
                    Ok(state) if state == "candidate" || state == "selected"
                )
            })
        {
            return Err(SciError::Invalid(
                "CHOOSE の ITEM なので、選ぶ OPTION を指定してください".to_string(),
            ));
        }
        let mut evidence = Vec::new();
        for (index, value) in text_list(input, "evidence").into_iter().enumerate() {
            let id = parse_ref(&value, 'F').map_err(|_| {
                SciError::Invalid(format!(
                    "evidence の {} 番目は F の形で指定してください: {value}",
                    index + 1
                ))
            })?;
            if store::get_finding(self.conn, id).is_err() {
                return Err(SciError::Invalid(format!(
                    "根拠にした {} がありません",
                    fmt_ref('F', id)
                )));
            }
            evidence.push(id);
        }
        let mut rejected = Vec::new();
        for entry in object_list(input, "rejected") {
            let text = text_field(&entry, "option");
            let id = parse_ref(&text, 'N')?;
            self.require_child_option(node.id, id, "rejected")?;
            let reason = text_field(&entry, "reason");
            check_len(&reason, 1, 1000, "reason", "却下の理由")?;
            rejected.push(RejectedOption { option: id, reason });
        }
        Ok(Plan::Decide {
            node: node.id,
            option,
            tentative: flag(input, "tentative"),
            evidence,
            rejected,
            counter_searched: flag(input, "counter_searched"),
        })
    }

    /// 候補が、その項目の子であることを確かめる。
    fn require_child_option(&self, item: i64, option: i64, key: &str) -> Result<()> {
        if !self.child_options(item)?.contains(&option) {
            return Err(SciError::Invalid(format!(
                "{key} の {} は {} の OPTION ではありません",
                fmt_ref('N', option),
                fmt_ref('N', item)
            )));
        }
        Ok(())
    }

    /// 項目の子の候補（番号の昇順）。
    fn child_options(&self, item: i64) -> Result<Vec<i64>> {
        let mut out = Vec::new();
        for child in store::children_of(self.conn, item)? {
            if self.node(child)?.kind == "option" {
                out.push(child);
            }
        }
        Ok(out)
    }

    fn check_reopen(&self, input: &serde_json::Value) -> Result<Plan> {
        Ok(Plan::Reopen {
            node: self.item_input(input, "node")?.id,
        })
    }

    fn check_exclude_item(&self, input: &serde_json::Value) -> Result<Plan> {
        Ok(Plan::ExcludeItem {
            node: self.item_input(input, "node")?.id,
        })
    }

    fn check_option_state(&self, input: &serde_json::Value) -> Result<Plan> {
        let node = self.option_input(input, "node")?;
        let state = text_of(input, "state");
        if !OPTION_TARGET_STATES.contains(&state.as_str()) {
            return Err(SciError::Invalid(format!(
                "OPTION の状態は rejected / dormant / candidate のどれかにしてください: {state}"
            )));
        }
        Ok(Plan::OptionState {
            node: node.id,
            state,
        })
    }

    fn check_exclude(&self, input: &serde_json::Value) -> Result<Plan> {
        let (target_kind, target_ref) = self.check_target(&text_of(input, "target"), true)?;
        let node = match opt_text(input, "node") {
            Some(text) => Some(parse_ref(&text, 'N')?),
            None => None,
        };
        if let Some(node) = node {
            self.node(node)?;
        }
        let title = opt_text(input, "title").unwrap_or_default();
        check_len(&title, 0, 200, "title", "題名")?;
        Ok(Plan::Exclude {
            target_kind,
            target_ref,
            node,
            title,
        })
    }

    fn check_edit_text(&self, input: &serde_json::Value) -> Result<Plan> {
        let node = self.node_input_kinds(input, "node", &["item", "option"])?;
        let title = raw_text(input, "title");
        let body = raw_text(input, "body");
        if title.is_none() && body.is_none() {
            return Err(SciError::Invalid(
                "title（題名）か body（本文）を 1 つ以上指定してください".to_string(),
            ));
        }
        if let Some(title) = title.as_deref() {
            check_len(title, 1, 120, "title", "題名")?;
        }
        if let Some(body) = body.as_deref() {
            check_len(body, 0, 2000, "body", "本文")?;
        }
        Ok(Plan::EditText {
            node: node.id,
            title,
            body,
        })
    }

    fn check_add_edge(&self, input: &serde_json::Value) -> Result<Plan> {
        let kind = self.edge_kind(input)?;
        let from = self.ref_input(input, "from", 'N')?;
        let to = self.ref_input(input, "to", 'N')?;
        self.node(from)?;
        self.node(to)?;
        if from == to {
            return Err(SciError::Invalid(
                "from と to に同じ NODE は指定できません".to_string(),
            ));
        }
        if kind == "depends" && self.depends_reaches(to, from)? {
            return Err(SciError::Invalid(format!(
                "DEPENDS の線が輪になります。{EDGE_DIRECTION}です。向きが逆でないか確かめてください"
            )));
        }
        let note = opt_text(input, "note").unwrap_or_default();
        check_len(&note, 0, 200, "note", "補足")?;
        Ok(Plan::AddEdge {
            kind,
            from,
            to,
            note,
        })
    }

    fn check_remove_edge(&self, input: &serde_json::Value) -> Result<Plan> {
        let kind = self.edge_kind(input)?;
        let from = self.ref_input(input, "from", 'N')?;
        let to = self.ref_input(input, "to", 'N')?;
        self.node(from)?;
        self.node(to)?;
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM sci_edges WHERE kind = ?1 AND from_node = ?2 AND to_node = ?3",
                params![kind, from, to],
                |row| row.get(0),
            )
            .optional()?;
        if found.is_none() {
            return Err(SciError::Invalid(format!(
                "その線がありません: {kind} {}→{}",
                fmt_ref('N', from),
                fmt_ref('N', to)
            )));
        }
        Ok(Plan::RemoveEdge { kind, from, to })
    }

    /// 線の種類を読む（`parent` は引けない）。
    fn edge_kind(&self, input: &serde_json::Value) -> Result<String> {
        let kind = text_of(input, "kind");
        if kind == "parent" {
            return Err(SciError::Invalid(
                "parent（親子）の線は引けません".to_string(),
            ));
        }
        if !EDGE_KINDS.contains(&kind.as_str()) {
            return Err(SciError::Invalid(format!(
                "知らない線です: {kind}（depends / derived / analogous）"
            )));
        }
        Ok(kind)
    }

    /// `depends` の線をたどって `from` から `target` に着くか（線を足すと輪になるかの検査）。
    fn depends_reaches(&self, from: i64, target: i64) -> Result<bool> {
        let edges = store::list_edges(self.conn)?;
        let mut visited: Vec<i64> = vec![from];
        let mut queue: Vec<i64> = vec![from];
        while let Some(current) = queue.first().copied() {
            queue.remove(0);
            for edge in &edges {
                if edge.kind != "depends" || edge.from != current {
                    continue;
                }
                if edge.to == target {
                    return Ok(true);
                }
                if visited.contains(&edge.to) {
                    continue;
                }
                visited.push(edge.to);
                queue.push(edge.to);
            }
        }
        Ok(false)
    }

    fn check_unlink(&self, input: &serde_json::Value) -> Result<Plan> {
        Ok(Plan::Unlink {
            link: self.link_input(input, "link")?.id,
        })
    }

    fn check_remove_finding(&self, input: &serde_json::Value) -> Result<Plan> {
        Ok(Plan::RemoveFinding {
            finding: self.finding_input(input, "finding")?.id,
        })
    }

    fn check_set_read_state(&self, input: &serde_json::Value) -> Result<Plan> {
        let link = self.link_input(input, "link")?;
        if link.target_kind != "paper" {
            return Err(SciError::Invalid(format!(
                "LINK {} は論文ではないため、READ_STATE を決められません",
                fmt_ref('L', link.id)
            )));
        }
        let state = text_of(input, "state");
        if !READ_STATES.contains(&state.as_str()) {
            return Err(SciError::Invalid(format!("知らない READ_STATE です: {state}")));
        }
        Ok(Plan::SetReadState {
            link: link.id,
            state,
        })
    }

    fn check_withdraw_question(&self, input: &serde_json::Value) -> Result<Plan> {
        Ok(Plan::WithdrawQuestion {
            question: self.question_input(input, "question")?.id,
        })
    }

    /// 任意で指定できる問い（`question`）。
    fn opt_question(&self, input: &serde_json::Value, key: &str) -> Result<Option<i64>> {
        match opt_text(input, key) {
            Some(text) => {
                let id = parse_ref(&text, 'Q')?;
                store::get_question(self.conn, id).map_err(|error| match error {
                    SciError::NotFound(message) => SciError::Invalid(message),
                    other => other,
                })?;
                Ok(Some(id))
            }
            None => Ok(None),
        }
    }

    /// 任意で指定できる論文番号。
    fn opt_paper(&self, input: &serde_json::Value, key: &str) -> Result<Option<i64>> {
        match opt_number_text(input, key) {
            Some(text) => {
                let id: i64 = text.parse().map_err(|_| {
                    SciError::Invalid(format!("{key} は論文の番号で指定してください: {text}"))
                })?;
                if !self.paper_exists(id)? {
                    return Err(SciError::Invalid(format!(
                        "論文 {id} が Library にありません"
                    )));
                }
                Ok(Some(id))
            }
            None => Ok(None),
        }
    }

    /// 任意で指定できる読む札の番号。
    fn opt_run(&self, input: &serde_json::Value, key: &str) -> Result<Option<i64>> {
        match opt_number_text(input, key) {
            Some(text) => {
                let id: i64 = text.parse().map_err(|_| {
                    SciError::Invalid(format!("{key} は実行の番号で指定してください: {text}"))
                })?;
                if !self.run_exists(id) {
                    return Err(SciError::Invalid(format!("読む札 {id} がありません")));
                }
                Ok(Some(id))
            }
            None => Ok(None),
        }
    }

    // ---- 反映 ---------------------------------------------------------------

    /// 検査済みの操作を反映して、できたものの参照を返す。
    ///
    /// 最初に節点のキャッシュを捨てる。検査の段で貯めた値は、この実行が始めた
    /// ときの状態なので、同じまとめの 1 件目が書いた状態を 2 件目が読めない。
    /// 書き込みの段は節点の状態をすべて DB から読む。
    fn write(
        &mut self,
        plan: &Plan,
        input: &serde_json::Value,
        proposal_id: Option<i64>,
        auto: bool,
    ) -> Result<Option<String>> {
        self.nodes.borrow_mut().clear();
        match plan {
            Plan::AddFinding {
                node,
                question,
                text,
                cites,
                polarity,
                level,
                paper,
                run,
            } => {
                // 出所は出主に合わせる。人の操作に run があれば、その読む札の答えになる。
                let (source, run_id) = match &self.actor {
                    Actor::Run { run_id, .. } => ("run", *run_id),
                    Actor::User => ("user", *run),
                };
                // paper が無ければ、最初に使った論文を、出所の論文にする。
                let paper_id = paper.or_else(|| cites.first().and_then(|c| first_cite_number(c)));
                let id = store::insert_finding(
                    self.conn,
                    &NewFinding {
                        node_id: *node,
                        question_id: *question,
                        text,
                        cites,
                        polarity,
                        level: *level,
                        source,
                        report_id: None,
                        paper_id,
                        run_id,
                        proposal_id,
                        auto,
                        created_by: &actor_text(&self.actor),
                    },
                )?;
                // 読んだのに反映していない論文は、反映済みにする。
                if let Some(paper_id) = paper_id {
                    for link in store::list_links(self.conn, Some(*node), false)? {
                        if link.target_kind == "paper"
                            && link.target_ref == paper_id.to_string()
                            && link.read_state.as_deref() == Some("read")
                        {
                            store::set_read_state(self.conn, link.id, "reflected")?;
                        }
                    }
                }
                Ok(Some(fmt_ref('F', id)))
            }
            Plan::AddQuestion { node, text } => {
                let id = store::insert_question(
                    self.conn,
                    *node,
                    text,
                    false,
                    &actor_text(&self.actor),
                )?;
                Ok(Some(fmt_ref('Q', id)))
            }
            Plan::AnswerQuestion { question, text } => {
                store::answer_question(self.conn, *question, text)?;
                Ok(Some(fmt_ref('Q', *question)))
            }
            Plan::Link {
                node,
                target_kind,
                target_ref,
                role,
                read_state,
            } => {
                // 論文でない結び付けには読み具合を入れない（DB の制約）。
                let read = target_kind
                    .as_str()
                    .eq("paper")
                    .then_some(read_state.as_str());
                let id = store::upsert_link(
                    self.conn,
                    &NewLink {
                        node_id: *node,
                        target_kind,
                        target_ref,
                        role: role.as_deref(),
                        read_state: read,
                        reason: &text_of(input, "reason"),
                        proposal_id,
                        auto,
                        created_by: &actor_text(&self.actor),
                    },
                )?;
                Ok(Some(fmt_ref('L', id)))
            }
            Plan::AddSeed {
                title,
                sentence,
                path,
                doc,
            } => {
                let run_id = match &self.actor {
                    Actor::Run { run_id, .. } => *run_id,
                    Actor::User => None,
                };
                let id = store::insert_seed(self.conn, title, sentence, path, doc, run_id)?;
                Ok(Some(fmt_ref('S', id)))
            }
            Plan::AddOption { node, title, body } => {
                let ord = i64::try_from(store::children_of(self.conn, *node)?.len()).unwrap_or(0);
                let id = store::insert_node(
                    self.conn,
                    &NewNode {
                        kind: "option",
                        item_key: None,
                        types: &[],
                        title,
                        body,
                        state: "candidate",
                        fixed: false,
                        ord,
                        created_by: &actor_text(&self.actor),
                    },
                )?;
                store::insert_edge(self.conn, "parent", *node, id)?;
                Ok(Some(fmt_ref('N', id)))
            }
            Plan::AddChild {
                node,
                title,
                types,
                key,
            } => {
                let ord = i64::try_from(store::children_of(self.conn, *node)?.len()).unwrap_or(0);
                let created_by = actor_text(&self.actor);
                let id = match key {
                    // ガイドがあれば、既定の問いと子の項目ごと作る。
                    Some(key) => super::defs::create_item_from_guide(
                        self.conn,
                        self.defs,
                        key,
                        Some(*node),
                        ord,
                        &created_by,
                    )?,
                    None => store::insert_node(
                        self.conn,
                        &NewNode {
                            kind: "item",
                            item_key: None,
                            types,
                            title,
                            body: "",
                            state: "open",
                            fixed: false,
                            ord,
                            created_by: &created_by,
                        },
                    )?,
                };
                Ok(Some(fmt_ref('N', id)))
            }
            Plan::Decide {
                node,
                option,
                tentative,
                evidence,
                rejected,
                counter_searched,
            } => {
                // 選んだ候補を selected に、同じ項目の他の selected は candidate に戻す。
                for child in self.child_options(*node)? {
                    let child_node = self.node(child)?;
                    if Some(child) == *option {
                        store::set_node_state(self.conn, child, "selected")?;
                    } else if child_node.state == "selected" {
                        store::set_node_state(self.conn, child, "candidate")?;
                    }
                }
                for entry in rejected {
                    store::set_node_state(self.conn, entry.option, "rejected")?;
                }
                store::set_node_state(
                    self.conn,
                    *node,
                    if *tentative { "tentative" } else { "decided" },
                )?;
                self.record(
                    *node,
                    if *tentative { "tentative" } else { "decide" },
                    *option,
                    input,
                    evidence,
                    rejected,
                    *counter_searched,
                    proposal_id,
                )?;
                Ok(Some(fmt_ref('N', *node)))
            }
            Plan::Reopen { node } => {
                store::set_node_state(self.conn, *node, "open")?;
                for child in self.child_options(*node)? {
                    if self.node(child)?.state == "selected" {
                        store::set_node_state(self.conn, child, "candidate")?;
                    }
                }
                self.record(*node, "reopen", None, input, &[], &[], false, proposal_id)?;
                Ok(Some(fmt_ref('N', *node)))
            }
            Plan::ExcludeItem { node } => {
                store::set_node_state(self.conn, *node, "excluded")?;
                self.record(
                    *node,
                    "exclude_item",
                    None,
                    input,
                    &[],
                    &[],
                    false,
                    proposal_id,
                )?;
                Ok(Some(fmt_ref('N', *node)))
            }
            Plan::OptionState { node, state } => {
                // 記録は候補の親の項目に残す（親が無くても記録は要るため、候補自身を親にする）。
                let parent = store::parents_of(self.conn, *node)?
                    .into_iter()
                    .find(|id| {
                        self.node(*id)
                            .map(|found| found.kind == "item")
                            .unwrap_or(false)
                    })
                    .unwrap_or(*node);
                let was_selected = self.node(*node)?.state == "selected";
                store::set_node_state(self.conn, *node, state)?;
                let action = match state.as_str() {
                    "rejected" => "reject_option",
                    "dormant" => "dormant_option",
                    _ => "restore_option",
                };
                self.record(
                    parent,
                    action,
                    Some(*node),
                    input,
                    &[],
                    &[],
                    false,
                    proposal_id,
                )?;
                // 選んだ候補の無い決定を残さない。
                if was_selected && matches!(state.as_str(), "rejected" | "dormant") {
                    let parent_node = self.node(parent)?;
                    if matches!(parent_node.state.as_str(), "decided" | "tentative") {
                        store::set_node_state(self.conn, parent, "open")?;
                        self.record(parent, "reopen", None, input, &[], &[], false, proposal_id)?;
                    }
                }
                Ok(Some(fmt_ref('N', *node)))
            }
            Plan::Exclude {
                target_kind,
                target_ref,
                node,
                title,
            } => {
                // 論文のときは DOI・arXiv・題を写す。候補のときは重複キーだけ。
                let (doi, arxiv_id) = if target_kind == "paper" {
                    let number: i64 = target_ref.parse().unwrap_or(0);
                    self.conn
                        .query_row(
                            "SELECT doi, arxiv_id FROM papers WHERE id = ?1",
                            params![number],
                            |row| {
                                Ok((
                                    row.get::<_, Option<String>>(0)?,
                                    row.get::<_, Option<String>>(1)?,
                                ))
                            },
                        )
                        .optional()?
                        .unwrap_or((None, None))
                } else {
                    (None, None)
                };
                let paper_id = if target_kind == "paper" {
                    target_ref.parse::<i64>().ok()
                } else {
                    None
                };
                store::insert_tombstone(
                    self.conn,
                    &NewTombstone {
                        kind: target_kind,
                        paper_id,
                        doi: doi.as_deref(),
                        arxiv_id: arxiv_id.as_deref(),
                        dedupe_key: (target_kind == "candidate").then_some(target_ref.as_str()),
                        title,
                        reason: &text_of(input, "reason"),
                        node_id: *node,
                        run_id: None,
                        created_by: &actor_text(&self.actor),
                    },
                )?;
                // 結び付けた論文は「関係なし」にする。
                if let Some(paper_id) = paper_id {
                    self.conn.execute(
                        "UPDATE sci_links SET read_state = 'irrelevant', updated_at = ?1
                          WHERE target_kind = 'paper' AND target_ref = ?2 AND removed_at IS NULL",
                        params![super::now_text(), paper_id.to_string()],
                    )?;
                }
                Ok(node.map(|id| fmt_ref('N', id)))
            }
            Plan::EditText { node, title, body } => {
                store::update_node_text(self.conn, *node, title.as_deref(), body.as_deref())?;
                Ok(Some(fmt_ref('N', *node)))
            }
            Plan::AddEdge {
                kind,
                from,
                to,
                note,
            } => {
                let id = store::insert_edge(self.conn, kind, *from, *to)?;
                if !note.is_empty() {
                    self.conn.execute(
                        "UPDATE sci_edges SET note = ?1 WHERE id = ?2",
                        params![note, id],
                    )?;
                }
                Ok(Some(fmt_ref('N', *from)))
            }
            Plan::Unlink { link } => {
                store::remove_link(self.conn, *link)?;
                Ok(Some(fmt_ref('L', *link)))
            }
            Plan::RemoveFinding { finding } => {
                store::remove_finding(self.conn, *finding)?;
                Ok(Some(fmt_ref('F', *finding)))
            }
            Plan::SetReadState { link, state } => {
                store::set_read_state(self.conn, *link, state)?;
                Ok(Some(fmt_ref('L', *link)))
            }
            Plan::WithdrawQuestion { question } => {
                store::withdraw_question(self.conn, *question)?;
                Ok(Some(fmt_ref('Q', *question)))
            }
            Plan::RemoveEdge { kind, from, to } => {
                self.conn.execute(
                    "DELETE FROM sci_edges WHERE kind = ?1 AND from_node = ?2 AND to_node = ?3",
                    params![kind, from, to],
                )?;
                Ok(Some(fmt_ref('N', *from)))
            }
        }
    }

    /// 決定の記録を 1 行足す。消す・直すことはしない（DB のトリガーが止める）。
    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        node_id: i64,
        action: &str,
        option_id: Option<i64>,
        input: &serde_json::Value,
        evidence: &[i64],
        rejected: &[RejectedOption],
        counter_searched: bool,
        proposal_id: Option<i64>,
    ) -> Result<()> {
        store::insert_decision(
            self.conn,
            &NewDecision {
                node_id,
                action,
                option_id,
                reason: &text_of(input, "reason"),
                evidence,
                rejected,
                counter_searched,
                actor: &self.decision_actor,
                proposal_id,
            },
        )?;
        Ok(())
    }
}

// ---- 取り消し ---------------------------------------------------------------

/// 自動で反映した提案が作ったものを外す。
///
/// 知見は外す（行は残す）・結び付きは外す・問いは引く・問いへの答えは問いを未解決に戻す。
/// 種は行を残し（提案の `reverted` の印だけで足りる）。
fn revert(conn: &Connection, proposal: &store::Proposal) -> Result<()> {
    let Some(reference) = proposal.result_ref.clone() else {
        return Ok(());
    };
    match proposal.op.as_str() {
        "add_finding" => {
            let id = parse_ref(&reference, 'F')?;
            if store::get_finding(conn, id).is_ok() {
                store::remove_finding(conn, id)?;
            }
        }
        "link" => {
            let id = parse_ref(&reference, 'L')?;
            if store::get_link(conn, id).is_ok() {
                store::remove_link(conn, id)?;
            }
        }
        "add_question" => {
            let id = parse_ref(&reference, 'Q')?;
            if store::get_question(conn, id).is_ok() {
                store::withdraw_question(conn, id)?;
            }
        }
        "answer_question" => {
            let id = parse_ref(&reference, 'Q')?;
            if store::get_question(conn, id).is_ok() {
                conn.execute(
                    "UPDATE sci_questions SET state = 'open', answer = '', updated_at = ?1 WHERE id = ?2",
                    params![super::now_text(), id],
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

// ---- 補助 -------------------------------------------------------------------

/// 提案の入力を 1 件ずつのリストにする。2 つ目の戻り値は「まとめて出したか」。
fn read_proposal_list(input: &serde_json::Value) -> Result<(Vec<&serde_json::Value>, bool)> {
    let Some(object) = input.as_object() else {
        return Err(SciError::Invalid("PROPOSAL の形が正しくありません".to_string()));
    };
    let Some(list) = object.get("proposals") else {
        if !object.contains_key("op") {
            return Err(SciError::Invalid("PROPOSAL の形が正しくありません".to_string()));
        }
        return Ok((vec![input], false));
    };
    let Some(items) = list.as_array() else {
        return Err(SciError::Invalid("PROPOSAL の形が正しくありません".to_string()));
    };
    if items.is_empty() {
        return Err(SciError::Invalid("PROPOSAL は 1 回に 1〜20 件です".to_string()));
    }
    if items.len() > MAX_PROPOSALS {
        return Err(SciError::Invalid("PROPOSAL は 1 回に 20 件までです".to_string()));
    }
    if items.iter().any(|item| !item.is_object()) {
        return Err(SciError::Invalid("PROPOSAL の形が正しくありません".to_string()));
    }
    Ok((items.iter().collect(), true))
}

/// 提案の何件目かをエラー文の頭に付ける。1 件だけなら付けない。
fn with_index(index: Option<usize>, error: SciError) -> SciError {
    let Some(index) = index else {
        return error;
    };
    SciError::Invalid(format!("{} 件目: {error}", index + 1))
}

/// 提案の `payload` に edit を書き重ねる（無い項目はそのまま）。
fn merge_payload(
    payload: &serde_json::Value,
    edit: Option<&serde_json::Value>,
) -> serde_json::Value {
    let Some(edit) = edit else {
        return payload.clone();
    };
    let Some(patch) = edit.as_object() else {
        return payload.clone();
    };
    let mut merged = payload.clone();
    let Some(target) = merged.as_object_mut() else {
        return payload.clone();
    };
    for (key, value) in patch {
        target.insert(key.clone(), value.clone());
    }
    merged
}

/// 操作の危険度。自動で入る種類は `auto`、利用者が採用する種類は `human`。
///
/// どちらの表にも無いのは利用者だけの操作なので、提案では受け付けない。
fn risk_of(op: &str) -> Result<&'static str> {
    if AUTO_OPS.contains(&op) {
        Ok("auto")
    } else if HUMAN_OPS.contains(&op) {
        Ok("human")
    } else {
        Err(SciError::Invalid(format!(
            "{op} は PROPOSAL にできません（利用者だけの操作です）"
        )))
    }
}

/// 提案の actor の文字列（`user` / `run:120` / `run:?`）。
fn actor_text(actor: &Actor) -> String {
    match actor {
        Actor::User => "user".to_string(),
        Actor::Run { run_id, .. } => match run_id {
            Some(id) => format!("run:{id}"),
            None => "run:?".to_string(),
        },
    }
}

/// 提案の状態の表示名。
fn status_name(status: &str) -> &'static str {
    match status {
        "applied" => "AUTO_OP で反映",
        "pending" => "REVIEW_OP（TRIAGE 待ち）",
        "accepted" => "採用",
        "rejected" => "却下",
        "reverted" => "取り消し",
        _ => "不明の状態",
    }
}

/// JSON から文字列を取り出す。無い・文字列でないものは空。
fn text_of(input: &serde_json::Value, key: &str) -> String {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// JSON から、あれば空でない文字列を取り出す。
fn opt_text(input: &serde_json::Value, key: &str) -> Option<String> {
    let found = text_of(input, key);
    (!found.is_empty()).then_some(found)
}

/// JSON から番号を文字列で取り出す（数でも文字列でもよい。Web は数で送る）。
fn opt_number_text(input: &serde_json::Value, key: &str) -> Option<String> {
    match input.get(key) {
        Some(serde_json::Value::Number(number)) => Some(number.to_string()),
        _ => opt_text(input, key),
    }
}

/// JSON から、あれば空文字でも取り出す（空にするつもりで書くとき用）。
fn raw_text(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(|text| text.trim().to_string())
}

/// 真偽の項目。文字列の "true" / "false" も読む。
fn flag(input: &serde_json::Value, key: &str) -> bool {
    match input.get(key) {
        Some(serde_json::Value::Bool(value)) => *value,
        Some(serde_json::Value::String(text)) => text.trim() == "true",
        _ => false,
    }
}

/// 任意の数。
fn opt_int(input: &serde_json::Value, key: &str) -> Option<i64> {
    input.get(key).and_then(serde_json::Value::as_i64)
}

/// 文字列の配列を読む。文字列でない要素は落とす。
fn text_list(input: &serde_json::Value, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|text| text.trim().to_string()))
                .filter(|text| !text.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// 写しの配列を読む（`rejected`）。
fn object_list(input: &serde_json::Value, key: &str) -> Vec<serde_json::Value> {
    input
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.is_object())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// 写しの項目を取り出す。
fn text_field(item: &serde_json::Value, key: &str) -> String {
    text_of(item, key)
}
/// 文字数が `min`〜`max` の間であることを確かめる。
fn check_len(text: &str, min: usize, max: usize, key: &str, name: &str) -> Result<()> {
    let count = text.chars().count();
    if count < min {
        return Err(SciError::Invalid(format!("{name}（{key}）が短すぎます")));
    }
    if count > max {
        return Err(SciError::Invalid(format!(
            "{name}（{key}）は {max} 字までです"
        )));
    }
    Ok(())
}

/// 先頭から `max` 字に切る。
fn take(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 文の数を数える。
///
/// 「。」「？」「！」は文の終わり。`"."` `?` は後ろが空白・行末のときだけ文の終わりとする
/// （`0.85` や `§3.2` の「.」は数えない）。0 なら 1 文とみなす。
fn sentence_count(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut count = 0;
    for (index, ch) in chars.iter().enumerate() {
        let end = match ch {
            '。' | '？' | '！' => true,
            '.' | '?' => chars.get(index + 1).is_none_or(|next| next.is_whitespace()),
            _ => false,
        };
        if end {
            count += 1;
        }
    }
    count.max(1)
}

/// 引用から論文の番号を取り出す。
fn first_cite_number(cite: &str) -> Option<i64> {
    CITE_PATTERN.captures(cite)?[1].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::sci::defs;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// 全表のメモリ DB。
    fn db() -> Database {
        Database::in_memory().expect("メモリ DB")
    }

    /// 埋め込みだけを読むための Defs（存在しないフォルダを渡す）。
    fn defs() -> Defs {
        defs::load(std::path::Path::new("target/sci-ops-defs-not-found"))
    }

    /// テスト用の置き場（一時フォルダ）。Dispose で消す。
    struct Fixture {
        _dir: PathBuf,
        roots: PaperRoots,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self._dir);
        }
    }

    /// テスト用の置き場。ブロック定義を 2 つ（`sci-t` `sci-decide`）と読む札 120 を作る。
    fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-ops-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".sai").join("blocks")).expect("ブロックの置き場");
        fs::create_dir_all(dir.join(".sai").join("outputs").join("todo").join("120"))
            .expect("読む札の置き場");
        write_block(&dir, "sci-t", "調べる", "add_finding, link");
        write_block(
            &dir,
            "sci-decide",
            "比べる",
            "add_option, decide, option_state, add_child",
        );
        let roots = PaperRoots {
            root: dir.clone(),
            data_dir: dir.join(".sai").join("paper"),
        };
        Fixture { _dir: dir, roots }
    }

    /// ブロック定義を 1 つ書く。
    fn write_block(dir: &Path, id: &str, name: &str, ops: &str) {
        fs::write(
            dir.join(".sai").join("blocks").join(format!("{id}.md")),
            format!(
                "---\nname: {name}\nid: {id}\ntemplates:\n  - sci-core\nsci_ops: {ops}\n---\n\n本文\n"
            ),
        )
        .expect("ブロック定義を書く");
    }

    /// 論文・報告・検索セットを 1 つずつ入れる。
    fn fill_library(conn: &Connection) {
        conn.execute("INSERT INTO papers (id, title) VALUES (37, '論文 37')", [])
            .expect("論文 37");
        conn.execute("INSERT INTO papers (id, title) VALUES (40, '論文 40')", [])
            .expect("論文 40");
        conn.execute(
            "INSERT INTO reports (id, title, md_path, created_at)
             VALUES ('R-0012', '報告 12', '/tmp/R-0012.md', '2026-01-01 00:00:00')",
            [],
        )
        .expect("報告 12");
        conn.execute(
            "INSERT INTO search_sets (id, name, source_dbs, query) VALUES (5, 'set-5', 's2', 'epilepsy')",
            [],
        )
        .expect("検索セット 5");
    }

    /// 項目を 1 つ足す。
    fn add_item(conn: &Connection, types: &[&str], title: &str) -> i64 {
        let types: Vec<String> = types.iter().map(|found| (*found).to_string()).collect();
        store::insert_node(
            conn,
            &NewNode {
                kind: "item",
                item_key: None,
                types: &types,
                title,
                body: "",
                state: "open",
                fixed: false,
                ord: 0,
                created_by: "user",
            },
        )
        .expect("項目を足す")
    }

    /// 候補。
    fn add_option(conn: &Connection, item: i64, title: &str) -> i64 {
        let id = store::insert_node(
            conn,
            &NewNode {
                kind: "option",
                item_key: None,
                types: &[],
                title,
                body: "",
                state: "candidate",
                fixed: false,
                ord: 0,
                created_by: "user",
            },
        )
        .expect("候補を足す");
        store::insert_edge(conn, "parent", item, id).expect("親子の線");
        id
    }

    /// 実行側の提案。
    fn agent() -> Actor {
        Actor::Run {
            run_id: Some(120),
            block: "sci-t".to_string(),
        }
    }

    /// エラー文を取り出す。
    fn message(error: &SciError) -> String {
        match error {
            SciError::Invalid(text) | SciError::NotFound(text) => text.clone(),
            other => panic!("見せるためのエラーではない: {other}"),
        }
    }

    #[test]
    fn op_list_is_split_into_three_risks() {
        // 19 種類が 3 つに分かれていて、重なっていない。
        assert_eq!(KNOWN_OPS.len(), 19);
        let mut all: Vec<&str> = AUTO_OPS
            .iter()
            .chain(HUMAN_OPS)
            .chain(USER_ONLY_OPS)
            .copied()
            .collect();
        all.sort_unstable();
        let mut known = KNOWN_OPS.to_vec();
        known.sort_unstable();
        all.dedup();
        assert_eq!(all, known);
    }

    #[test]
    fn allowed_ops_reads_block_frontmatter() {
        let placed = fixture();
        assert_eq!(
            allowed_ops(&placed.roots, "sci-t"),
            vec!["add_finding".to_string(), "link".to_string()]
        );
        // ブロックが分からないときは自動で入る 5 種類だけ。
        assert_eq!(allowed_ops(&placed.roots, "sci-nothing"), AUTO_OPS.to_vec());
    }

    #[test]
    fn block_id_must_be_lowercase_before_it_becomes_a_file_name() {
        // `^[a-z0-9-]+$` に通る ID だけを読む。外へ出る値をそのままファイル名にしない。
        let placed = fixture();
        for good in ["sci-t", "a", "0", "a-b-9"] {
            assert!(is_block_id(good), "{good} を弾いてはいけない");
            assert!(!block_def_paths(&placed.roots, good).is_empty(), "{good}");
        }
        for bad in ["", "..", "../sci-t", "a/b", "Sci-T", "sci_t", "sci.t", "a b"] {
            assert!(!is_block_id(bad), "{bad} を通してはいけない");
            assert!(
                block_def_paths(&placed.roots, bad).is_empty(),
                "{bad} のファイルを探してはいけない"
            );
        }

        // 外へ出る値を渡しても、`blocks/` の外のファイルは読まない（自動で入る 5 種類だけ）。
        let outside = placed.roots.root.join("outside.md");
        fs::write(
            &outside,
            "---\nname: 外\nid: outside\nsci_ops: decide, exclude\nsci_scope: 外の範囲\n---\n\n本文\n",
        )
        .expect("外側のファイルを書く");
        assert_eq!(
            allowed_ops(&placed.roots, "../outside"),
            AUTO_OPS.to_vec()
        );
    }

    #[test]
    fn auto_ops_apply_immediately_with_marks() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["write"], "書きたいこと");
        fill_library(conn);

        let results = propose(
            conn,
            &placed.roots,
            &defs,
            &agent(),
            &json!({
                "op": "add_finding",
                "node": format!("N{node}"),
                "text": "発作的脳波は発作前に 3 回出る。",
                "cites": ["#37 §3.2"],
                "polarity": "support",
                "reason": "論文 37 の図 3 が示している"
            }),
        )
        .expect("提案を通す");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].op, "add_finding");
        assert_eq!(results[0].status, "applied");
        let result = results[0].result.clone().expect("結果の ID");
        assert!(result.starts_with('F'), "{result}");

        // 反映したものは proposal_id と auto の印が付く。出所も提案した実行。
        let finding = store::get_finding(conn, result[1..].parse().expect("番号")).expect("知見");
        assert!(finding.auto);
        let proposal = store::get_proposal(conn, results[0].proposal[1..].parse().expect("番号"))
            .expect("提案");
        assert_eq!(finding.proposal_id, Some(proposal.id));
        assert_eq!(finding.source, "run");
        assert_eq!(finding.run_id, Some(120));
        assert_eq!(finding.created_by, "run:120");
        assert_eq!(finding.cites, vec!["#37 §3.2".to_string()]);
    }

    #[test]
    fn human_ops_wait_as_pending() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["choose"], "どれにする");
        let before = store::list_nodes(conn).expect("節点").len();

        // 危険度「人」の操作は、反映せずに仕分け待ちになる。
        let results = propose(
            conn,
            &placed.roots,
            &defs,
            &Actor::Run {
                run_id: Some(120),
                block: "sci-decide".to_string(),
            },
            &json!({
                "op": "add_option",
                "node": format!("N{node}"),
                "title": "候補 A",
                "reason": "出てきた候補"
            }),
        )
        .expect("提案を通す");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].op, "add_option");
        assert_eq!(results[0].status, "pending");
        assert_eq!(results[0].result, None);
        // 節点は増えていない（採用するまで入らない）。
        assert_eq!(store::list_nodes(conn).expect("節点").len(), before);
        let proposal = store::get_proposal(conn, results[0].proposal[1..].parse().expect("番号"))
            .expect("提案");
        assert_eq!(proposal.risk, "human");
        assert_eq!(proposal.status, "pending");
        assert_eq!(proposal.result_ref, None);
    }

    #[test]
    fn batch_is_all_or_nothing() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["write"], "書きたいこと");
        fill_library(conn);

        let error = propose(
            conn,
            &placed.roots,
            &defs,
            &agent(),
            &json!({
                "proposals": [
                    {
                        "op": "add_finding",
                        "node": format!("N{node}"),
                        "text": "一つめの知見。",
                        "cites": ["#37 §1"],
                        "polarity": "support",
                        "reason": "反映できた"
                    },
                    {
                        "op": "add_finding",
                        "node": "N9999",
                        "text": "二つめ。",
                        "cites": ["#40 §2"],
                        "polarity": "support",
                        "reason": "存在しない節点"
                    },
                    {
                        "op": "link",
                        "node": format!("N{node}"),
                        "target": "paper:37",
                        "role": "exemplar",
                        "reason": "代表例"
                    }
                ]
            }),
        )
        .expect_err("2 件目が不正なら通らない");

        assert!(
            message(&error).starts_with("2 件目: "),
            "{}",
            message(&error)
        );
        // 何も残らない。
        assert!(store::list_proposals(conn, None).expect("提案").is_empty());
        assert!(
            store::list_findings(conn, None, true)
                .expect("知見")
                .is_empty()
        );
        assert!(
            store::list_links(conn, None, true)
                .expect("結び付き")
                .is_empty()
        );
    }

    #[test]
    fn block_sci_ops_limit_proposals() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["choose"], "どれにする");
        let option = add_option(conn, node, "候補 A");

        // sci-t は add_finding と link だけ。decide は出せない。
        let error = propose(
            conn,
            &placed.roots,
            &defs,
            &agent(),
            &json!({
                "op": "decide",
                "node": format!("N{node}"),
                "option": format!("N{option}"),
                "reason": "再現性が高かった"
            }),
        )
        .expect_err("このブロックは decide を提案できない");
        assert_eq!(message(&error), "このブロックは decide を PROPOSAL にできません");
    }

    #[test]
    fn user_only_ops_rejected_in_proposals() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["write"], "書きたいこと");
        let finding = store::insert_finding(
            conn,
            &NewFinding {
                node_id: node,
                question_id: None,
                text: "知見。",
                cites: &[],
                polarity: "neutral",
                level: None,
                source: "user",
                report_id: None,
                paper_id: None,
                run_id: None,
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("知見を足す");

        // remove_finding を sci_ops に書いたブロック。即使そうしても、提案には出せない。
        write_block(
            &placed.roots.root,
            "sci-wide",
            "消す",
            "add_finding, remove_finding",
        );
        let error = propose(
            conn,
            &placed.roots,
            &defs,
            &Actor::Run {
                run_id: Some(120),
                block: "sci-wide".to_string(),
            },
            &json!({
                "op": "remove_finding",
                "finding": format!("F{finding}"),
                "reason": "読み違えた"
            }),
        )
        .expect_err("提案には remove_finding を使えない");
        assert_eq!(
            message(&error),
            "remove_finding は PROPOSAL にできません（利用者だけの操作です）"
        );
        assert!(
            store::get_finding(conn, finding)
                .expect("知見")
                .removed_at
                .is_none()
        );
    }

    #[test]
    fn finding_checks_cites_numbers_and_sentences() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["write"], "書きたいこと");
        fill_library(conn);
        let base = |text: &str, cites: serde_json::Value| {
            json!({
                "op": "add_finding",
                "node": format!("N{node}"),
                "text": text,
                "cites": cites,
                "polarity": "support",
                "reason": "根拠があった"
            })
        };
        let send = |input: serde_json::Value| -> Result<Vec<ProposalResult>> {
            propose(conn, &placed.roots, &defs, &agent(), &input)
        };

        // 「0.85」は 1 文。
        let results = send(base("精度は 0.85 だった。", json!(["#37 §1"]))).expect("通る");
        assert!(results[0].result.is_some());

        // 数字だけ・出典が無い知見は通らない。
        let error = send(base("精度は 0.85 だった。", json!([]))).expect_err("出典が要る");
        assert_eq!(message(&error), "エージェントの FINDING には引用が要ります");

        // 4 文の知見は通らない。
        let error = send(base("一。二。三。四。", json!(["#37 §1"]))).expect_err("3 文まで");
        assert!(message(&error).contains("3 文まで"), "{}", message(&error));

        // 数字を含む知見に出典が無いのは、人の操作でもだめ。
        let error = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "add_finding",
                "node": format!("N{node}"),
                "text": "精度は 0.85 だった。",
                "cites": [],
                "polarity": "support"
            }),
        )
        .expect_err("出典が要る");
        assert_eq!(message(&error), "数値を含む FINDING には出典（CITE）が要ります");

        // 数字の無い知見は、人の操作なら出典無くても通る。
        let result = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "add_finding",
                "node": format!("N{node}"),
                "text": "発作の部位は側頭葉だった。",
                "cites": [],
                "polarity": "support"
            }),
        )
        .expect("通る");
        assert!(result.result.is_some());

        // 無い論文の番号は通らない。
        let error = send(base("ある。", json!(["#999 §1"]))).expect_err("論文が無い");
        assert_eq!(message(&error), "論文 999 が Library にありません");

        // 形の違う引用は通らない。
        for bad in ["37 §3.2", "#037 §3.2", "#37§3.2", "37"] {
            let error = send(base("ある。", json!([bad]))).expect_err("形が違う");
            assert!(
                message(&error).starts_with("引用は #<論文番号> の形"),
                "{bad}: {}",
                message(&error)
            );
        }

        // 引用は 10 個まで。
        let many: Vec<String> = (0..11).map(|_| "#37 §1".to_string()).collect();
        let error = send(base("ある。", json!(many))).expect_err("10 個まで");
        assert_eq!(message(&error), "引用は 10 個までです");
    }

    #[test]
    fn decide_selects_option_and_records() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["choose"], "どれにする");
        let first = add_option(conn, node, "候補 A");
        let second = add_option(conn, node, "候補 B");
        // 一度選んでおく（選んだ候補が選ばれ続けることの確認のため）。
        store::set_node_state(conn, second, "selected").expect("選んでおく");

        let result = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "decide",
                "node": format!("N{node}"),
                "option": format!("N{first}"),
                "evidence": [],
                "reason": "再現性が高かった"
            }),
        )
        .expect("決める");
        assert_eq!(result.op, "decide");
        assert_eq!(result.result, Some(format!("N{node}")));

        assert_eq!(
            store::get_node(conn, first).expect("候補").state,
            "selected"
        );
        assert_eq!(
            store::get_node(conn, second).expect("候補").state,
            "candidate"
        );
        assert_eq!(store::get_node(conn, node).expect("項目").state, "decided");

        let decisions = store::list_decisions(conn, Some(node)).expect("決定の記録");
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].action, "decide");
        assert_eq!(decisions[0].option_id, Some(first));
        assert_eq!(decisions[0].actor, "user");

        // 選ぶ項目で候補があれば、選ぶ候補を省略できない。
        let error = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({ "op": "decide", "node": format!("N{node}"), "reason": "どれかにする" }),
        )
        .expect_err("選ぶ候補が要る");
        assert_eq!(
            message(&error),
            "CHOOSE の ITEM なので、選ぶ OPTION を指定してください"
        );

        // 項目でない候補の option は受け付けない。
        let error = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "decide",
                "node": format!("N{node}"),
                "option": format!("N{node}"),
                "reason": "項目自身は選べない"
            }),
        )
        .expect_err("項目は選べない");
        assert!(
            message(&error).contains("OPTION ではありません"),
            "{}",
            message(&error)
        );
    }

    #[test]
    fn reject_selected_option_reopens_item() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["choose"], "どれにする");
        let option = add_option(conn, node, "候補 A");
        store::set_node_state(conn, option, "selected").expect("選んでおく");
        store::set_node_state(conn, node, "decided").expect("決めておく");

        apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "option_state",
                "node": format!("N{option}"),
                "state": "rejected",
                "reason": "再現しなかった"
            }),
        )
        .expect("却下する");

        assert_eq!(
            store::get_node(conn, option).expect("候補").state,
            "rejected"
        );
        // 選んだ候補の無い決定を残さない。
        assert_eq!(store::get_node(conn, node).expect("項目").state, "open");
        let actions: Vec<String> = store::list_decisions(conn, Some(node))
            .expect("決定の記録")
            .into_iter()
            .map(|decision| decision.action)
            .collect();
        assert_eq!(
            actions,
            vec!["reject_option".to_string(), "reopen".to_string()]
        );
    }

    #[test]
    fn write_phase_reads_node_state_from_db() {
        // 検査の段で貯めた節点のキャッシュは、書き込みの段では使わない。
        // 同じまとめで 1 件目が書いた状態を 2 件目が読まないと、
        // 「選んだ候補の無い決定」が残ってしまう。
        //
        // `decide` と `option_state` は利用者が採用してから反映する操作なので、
        // `propose` ではこの並びにならない。ここでは書き込みの段を直接 2 回呼び、
        // 2 回目が DB の最新を見ていることを確かめる。
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["choose"], "どれにする");
        let option = add_option(conn, node, "候補 A");
        fill_library(conn);

        let decide_input = json!({
            "op": "decide",
            "node": format!("N{node}"),
            "option": format!("N{option}"),
            "reason": "再現したから"
        });
        let reject_input = json!({
            "op": "option_state",
            "node": format!("N{option}"),
            "state": "rejected",
            "reason": "再現しなかった"
        });

        let ctx = Ctx {
            conn,
            roots: &placed.roots,
            defs: &defs,
            nodes: RefCell::new(HashMap::new()),
            from_proposal: true,
            allowed: allowed_ops(&placed.roots, "sci-decide"),
            actor: agent(),
            decision_actor: "run:120".to_string(),
        };
        // 検査の段で「候補」は `candidate` としてキャッシュされる。
        let decide = ctx.check(&decide_input).expect("決定を検査");
        let reject = ctx.check(&reject_input).expect("却下を検査");

        let tx = conn.unchecked_transaction().expect("トランザクション");
        let mut writer = Ctx { conn: &tx, ..ctx };
        writer.write(&decide, &decide_input, None, false).expect("決定を反映");
        writer.write(&reject, &reject_input, None, false).expect("却下を反映");
        tx.commit().expect("書き込む");

        assert_eq!(
            store::get_node(conn, option).expect("候補").state,
            "rejected"
        );
        // 選んだ候補の無い決定を残さない（2 件目が DB から「selected」を読む）。
        assert_eq!(store::get_node(conn, node).expect("項目").state, "open");
        let actions: Vec<String> = store::list_decisions(conn, Some(node))
            .expect("決定の記録")
            .into_iter()
            .map(|decision| decision.action)
            .collect();
        assert_eq!(
            actions,
            vec![
                "decide".to_string(),
                "reject_option".to_string(),
                "reopen".to_string()
            ]
        );
    }

    #[test]
    fn exclude_paper_makes_tombstone_and_irrelevant_links() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["write"], "書きたいこと");
        let other = add_item(conn, &["write"], "別の項目");
        fill_library(conn);
        conn.execute(
            "UPDATE papers SET doi = '10.1/x', arxiv_id = '2401.00001' WHERE id = 37",
            [],
        )
        .expect("DOI を入れる");
        let link = store::upsert_link(
            conn,
            &NewLink {
                node_id: node,
                target_kind: "paper",
                target_ref: "37",
                role: Some("exemplar"),
                read_state: Some("read"),
                reason: "",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("論文を結ぶ");

        let result = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "exclude",
                "target": "paper:37",
                "node": format!("N{node}"),
                "title": "論文 37",
                "reason": "テーマが違った"
            }),
        )
        .expect("外す");
        assert_eq!(result.result, Some(format!("N{node}")));

        let tombstones = store::list_tombstones(conn).expect("墓標");
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].kind, "paper");
        assert_eq!(tombstones[0].paper_id, Some(37));
        assert_eq!(tombstones[0].doi.as_deref(), Some("10.1/x"));
        assert_eq!(tombstones[0].reason, "テーマが違った");

        // 結び付けた論文は「関係なし」になる。
        assert_eq!(
            store::get_link(conn, link)
                .expect("結び付き")
                .read_state
                .as_deref(),
            Some("irrelevant")
        );
        // 墓標の論文は、節点の論文の一覧にも入らない。
        assert!(
            store::node_paper_ids(conn, node, true)
                .expect("論文")
                .is_empty()
        );
        assert!(
            store::node_paper_ids(conn, other, true)
                .expect("論文")
                .is_empty()
        );

        // 候補の重複キーでも外せる。
        apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "exclude",
                "target": "candidate:smith-2024-abc",
                "reason": "取り違え"
            }),
        )
        .expect("候補を外す");
        assert_eq!(store::list_tombstones(conn).expect("墓標").len(), 2);
    }

    #[test]
    fn accept_with_edit_revalidates() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["choose"], "どれにする");

        let results = propose(
            conn,
            &placed.roots,
            &defs,
            &Actor::Run {
                run_id: Some(120),
                block: "sci-decide".to_string(),
            },
            &json!({
                "op": "add_option",
                "node": format!("N{node}"),
                "title": "候補 A",
                "reason": "比べて先行した結果が出ていた"
            }),
        )
        .expect("提案を通す");
        assert_eq!(results[0].status, "pending");
        let proposal_id: i64 = results[0].proposal[1..].parse().expect("番号");

        // 直した中身が不正なら採用できない。
        let error = triage(
            conn,
            &placed.roots,
            &defs,
            proposal_id,
            TriageAction::Accept {
                edit: Some(json!({ "title": "" })),
            },
            "",
        )
        .expect_err("空の題名はだめ");
        assert!(
            message(&error).contains("OPTION の題名"),
            "{}",
            message(&error)
        );
        assert_eq!(
            store::get_proposal(conn, proposal_id).expect("提案").status,
            "pending"
        );

        // 直して採用する。
        let result = triage(
            conn,
            &placed.roots,
            &defs,
            proposal_id,
            TriageAction::Accept {
                edit: Some(json!({ "title": "候補 A（直した）" })),
            },
            "lear",
        )
        .expect("採用する");
        assert_eq!(result.status, "accepted");
        let option_id: i64 = result.result.expect("結果の ID")[1..]
            .parse()
            .expect("番号");
        assert_eq!(
            store::get_node(conn, option_id).expect("候補").title,
            "候補 A（直した）"
        );
        assert_eq!(
            store::get_proposal(conn, proposal_id).expect("提案").status,
            "accepted"
        );
    }

    #[test]
    fn revert_removes_auto_items() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        // add_question も出せるようにブロック定義を 1 つ書く。
        write_block(
            &placed.roots.root,
            "sci-all",
            "全部出す",
            "add_finding, add_question, answer_question, link, add_seed",
        );
        let node = add_item(conn, &["write"], "書きたいこと");
        fill_library(conn);
        let wide = Actor::Run {
            run_id: Some(120),
            block: "sci-all".to_string(),
        };

        let results = propose(
            conn,
            &placed.roots,
            &defs,
            &wide,
            &json!({
                "proposals": [
                    {
                        "op": "add_finding",
                        "node": format!("N{node}"),
                        "text": "一つめの知見。",
                        "cites": ["#37 §1"],
                        "polarity": "support",
                        "reason": "論文にある"
                    },
                    {
                        "op": "add_question",
                        "node": format!("N{node}"),
                        "text": "三つ目は何だろう",
                        "reason": "資料がまだ足りていない論点",
                        "reason": " belum 届いていない論点"
                    },
                    {
                        "op": "link",
                        "node": format!("N{node}"),
                        "target": "paper:40",
                        "role": "compare",
                        "reason": "比べる"
                    }
                ]
            }),
        )
        .expect("提案を通す");
        let question = results[1].result.clone().expect("問いの ID");

        // 問いに答える。
        let answered = propose(
            conn,
            &placed.roots,
            &defs,
            &wide,
            &json!({
                "op": "answer_question",
                "question": question,
                "text": "答えはこれ。",
                "reason": "読む札の中で分かった"
            }),
        )
        .expect("答えの提案を通す");
        let answer_proposal: i64 = answered[0].proposal[1..].parse().expect("番号");

        assert_eq!(
            store::list_findings(conn, None, false).expect("知見").len(),
            1
        );
        assert_eq!(
            store::list_links(conn, None, false)
                .expect("結び付き")
                .len(),
            1
        );

        let reverted = triage(
            conn,
            &placed.roots,
            &defs,
            answer_proposal,
            TriageAction::Revert,
            "取り消す",
        )
        .expect("取り消す");
        assert_eq!(reverted.status, "reverted");
        let question_id: i64 = question[1..].parse().expect("番号");
        let found = store::get_question(conn, question_id).expect("問い");
        assert_eq!(found.state, "open");
        assert_eq!(found.answer, "");

        // 知見・結び付き・問いも、それぞれ取り消せる。
        for result in &results {
            let id: i64 = result.proposal[1..].parse().expect("番号");
            triage(
                conn,
                &placed.roots,
                &defs,
                id,
                TriageAction::Revert,
                "取り消す",
            )
            .expect("取り消す");
        }
        assert!(
            store::list_findings(conn, None, false)
                .expect("知見")
                .is_empty()
        );
        assert!(
            store::list_links(conn, None, false)
                .expect("結び付き")
                .is_empty()
        );
        let questions = store::list_questions(conn, None).expect("問い");
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].state, "withdrawn");
    }

    #[test]
    fn add_edge_rejects_cycle_and_parent() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let first = add_item(conn, &["write"], "一つ目");
        let second = add_item(conn, &["write"], "二つ目");
        let third = add_item(conn, &["write"], "三つ目");

        for (from, to) in [(first, second), (second, third)] {
            apply_user(
                conn,
                &placed.roots,
                &defs,
                &json!({
                    "op": "add_edge",
                    "kind": "depends",
                    "from": format!("N{from}"),
                    "to": format!("N{to}"),
                    "note": "先に一つ目"
                }),
            )
            .expect("線を引く");
        }
        // 輪は引けない。
        let error = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "add_edge",
                "kind": "depends",
                "from": format!("N{third}"),
                "to": format!("N{first}")
            }),
        )
        .expect_err("輪はだめ");
        assert_eq!(
            message(&error),
            "DEPENDS の線が輪になります。from が上流、to が下流です。向きが逆でないか確かめてください"
        );
        // 向きの説明は schema の説明にも同じ文で入っている。
        let spec = op_spec("add_edge").expect("add_edge の表");
        assert!(spec.about.contains(EDGE_DIRECTION), "{}", spec.about);

        // 同じ組は 2 度引いても増えない。
        apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "add_edge",
                "kind": "depends",
                "from": format!("N{first}"),
                "to": format!("N{second}")
            }),
        )
        .expect("同じ組を引く");
        let edges = store::list_edges(conn).expect("線");
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].note, "先に一つ目");

        // 同じ節点同士は引けない。
        let error = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "add_edge",
                "kind": "derived",
                "from": format!("N{first}"),
                "to": format!("N{first}")
            }),
        )
        .expect_err("自分自身はだめ");
        assert!(message(&error).contains("同じ NODE"), "{}", message(&error));

        // 親子は引けない。
        let error = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "add_edge",
                "kind": "parent",
                "from": format!("N{first}"),
                "to": format!("N{third}")
            }),
        )
        .expect_err("親子はだめ");
        assert_eq!(message(&error), "parent（親子）の線は引けません");

        // 線は消せる（履歴は残さない）。
        apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "remove_edge",
                "kind": "depends",
                "from": format!("N{second}"),
                "to": format!("N{third}")
            }),
        )
        .expect("線を消す");
        assert_eq!(store::list_edges(conn).expect("線").len(), 1);
    }

    #[test]
    fn reject_and_accept_record_decider_and_note() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["list"], "並べるもの");
        let actor = Actor::Run {
            run_id: Some(120),
            block: "sci-decide".to_string(),
        };
        let results = propose(
            conn,
            &placed.roots,
            &defs,
            &actor,
            &json!({
                "proposals": [
                    { "op": "add_option", "node": format!("N{node}"), "title": "一つ目", "reason": "出た" },
                    { "op": "add_option", "node": format!("N{node}"), "title": "二つ目", "reason": "出た" }
                ]
            }),
        )
        .expect("提案を通す");
        assert!(
            results
                .iter()
                .all(|result| result.status == "pending" && result.result.is_none())
        );

        // 却下。
        let rejected = triage(
            conn,
            &placed.roots,
            &defs,
            results[0].proposal[1..].parse().expect("番号"),
            TriageAction::Reject,
            "今は要らない",
        )
        .expect("却下する");
        assert_eq!(rejected.status, "rejected");
        let proposal =
            store::get_proposal(conn, rejected.proposal[1..].parse().expect("番号")).expect("提案");
        assert_eq!(proposal.decided_by.as_deref(), Some("user"));
        assert_eq!(proposal.decide_note, "今は要らない");
        assert!(proposal.decided_at.is_some());

        // 採用。作ったものは提案を出した実行のもの、決定の記録は利用者。
        let accepted = triage(
            conn,
            &placed.roots,
            &defs,
            results[1].proposal[1..].parse().expect("番号"),
            TriageAction::Accept { edit: None },
            "採用する",
        )
        .expect("採用する");
        assert_eq!(accepted.status, "accepted");
        let option_id: i64 = accepted.result.expect("結果")[1..].parse().expect("番号");
        assert_eq!(
            store::get_node(conn, option_id).expect("候補").created_by,
            "run:120"
        );

        // 却下した提案は採用できない。
        let error = triage(
            conn,
            &placed.roots,
            &defs,
            rejected.proposal[1..].parse().expect("番号"),
            TriageAction::Accept { edit: None },
            "",
        )
        .expect_err("却下したものは採用できない");
        assert_eq!(message(&error), "この PROPOSAL は 却下 なので採用できません");

        // 取り消しは自動で反映したものだけ。
        let error = triage(
            conn,
            &placed.roots,
            &defs,
            accepted.proposal[1..].parse().expect("番号"),
            TriageAction::Revert,
            "",
        )
        .expect_err("採用したものは取り消せない");
        assert_eq!(message(&error), "この PROPOSAL は 採用 なので取り消せません");
    }

    #[test]
    fn user_ops_write_questions_and_read_state() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["write"], "書きたいこと");
        fill_library(conn);

        let question = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({ "op": "add_question", "node": format!("N{node}"), "text": "三家目は何だろう" }),
        )
        .expect("問いを足す")
        .result
        .expect("結果");
        let question_id: i64 = question[1..].parse().expect("番号");
        apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({ "op": "answer_question", "question": question, "text": "答え。" }),
        )
        .expect("答える");
        apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({ "op": "withdraw_question", "question": question }),
        )
        .expect("引く");
        assert_eq!(
            store::get_question(conn, question_id).expect("問い").state,
            "withdrawn"
        );

        // 論文を結ぶ → 読み具合を入れる → 外す。
        let link = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "link",
                "node": format!("N{node}"),
                "target": "paper:37",
                "role": "method_source"
            }),
        )
        .expect("結ぶ")
        .result
        .expect("結果");
        assert_eq!(
            store::get_link(conn, link[1..].parse().expect("番号"))
                .expect("結び付き")
                .read_state
                .as_deref(),
            Some("unread")
        );
        apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({ "op": "set_read_state", "link": link, "state": "read" }),
        )
        .expect("読み具合を入れる");
        assert_eq!(
            store::get_link(conn, link[1..].parse().expect("番号"))
                .expect("結び付き")
                .read_state
                .as_deref(),
            Some("read")
        );
        apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({ "op": "unlink", "link": link }),
        )
        .expect("外す");
        assert!(
            store::get_link(conn, link[1..].parse().expect("番号"))
                .expect("結び付き")
                .removed_at
                .is_some()
        );

        // 種の置き場と読む札・報告・検索セットの結び付け。
        let seed = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "add_seed",
                "title": "論文 37 の話",
                "sentence": "三家目を出したときのこと。",
                "path": [format!("N{node}")],
                "doc": "waggle:ABCDEF12"
            }),
        )
        .expect("種を足す")
        .result
        .expect("結果");
        assert!(seed.starts_with('S'));
        for (target, key) in [
            ("report:R-0012", "report"),
            ("set:5", "set"),
            ("run:120", "run"),
        ] {
            let mut input = json!({ "op": "link", "node": format!("N{node}"), "target": target });
            if key == "report" {
                input["reason"] = json!("報告が根拠");
            }
            apply_user(conn, &placed.roots, &defs, &input).expect("結ぶ");
        }
        assert_eq!(
            store::list_links(conn, None, false)
                .expect("結び付き")
                .len(),
            3
        );

        // 本文を直す。
        apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({ "op": "edit_text", "node": format!("N{node}"), "body": "書き直した" }),
        )
        .expect("直す");
        assert_eq!(
            store::get_node(conn, node).expect("項目").body,
            "書き直した"
        );

        // 項目を足す（ガイドのキー付き）。
        let child = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({ "op": "add_child", "node": format!("N{node}"), "key": "data.type" }),
        )
        .expect("子を足す")
        .result
        .expect("結果");
        let child_id: i64 = child[1..].parse().expect("番号");
        assert_eq!(store::parents_of(conn, child_id).expect("親"), vec![node]);
        assert!(
            store::get_node(conn, child_id)
                .expect("項目")
                .item_key
                .is_some()
        );
    }

    #[test]
    fn user_only_ops_and_unknown_op_are_rejected() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["write"], "書きたいこと");

        let error = apply_user(conn, &placed.roots, &defs, &json!({ "op": "触らない" }))
            .expect_err("知らない操作");
        assert_eq!(message(&error), "知らない操作です: 触らない");

        // 理由の要る操作に理由が無い。
        let error = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({ "op": "exclude_item", "node": format!("N{node}") }),
        )
        .expect_err("理由が要る");
        assert_eq!(
            message(&error),
            "exclude_item には reason（理由）が要ります"
        );

        // 提案の形が違う。
        let error = propose(
            conn,
            &placed.roots,
            &defs,
            &agent(),
            &json!({ "proposals": [] }),
        )
        .expect_err("空の提案");
        assert_eq!(message(&error), "PROPOSAL は 1 回に 1〜20 件です");
        let error = propose(
            conn,
            &placed.roots,
            &defs,
            &agent(),
            &json!({ "proposals": (0..21).map(|_| json!({ "op": "link" })).collect::<Vec<_>>() }),
        )
        .expect_err("21 件");
        assert_eq!(message(&error), "PROPOSAL は 1 回に 20 件までです");
        let error = propose(
            conn,
            &placed.roots,
            &defs,
            &agent(),
            &json!({ "text": "だけ" }),
        )
        .expect_err("形が違う");
        assert_eq!(message(&error), "PROPOSAL の形が正しくありません");
    }

    /// 表は 19 種類の op を 1 つずつ持つ（利用者だけの操作も表にある）。
    #[test]
    fn op_fields_table_covers_every_op() {
        assert_eq!(OP_FIELDS.len(), KNOWN_OPS.len());
        let mut listed: Vec<&str> = OP_FIELDS.iter().map(|spec| spec.op).collect();
        listed.sort_unstable();
        let mut known: Vec<&str> = KNOWN_OPS.to_vec();
        known.sort_unstable();
        assert_eq!(listed, known);
        // 項目名が重複しない。
        for spec in OP_FIELDS {
            let mut keys: Vec<&str> = spec.fields.iter().map(|field| field.key).collect();
            keys.sort_unstable();
            let before = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), before, "{} の項目が重複している", spec.op);
        }
    }

    /// 形の違う提案は、どの項目が悪いのかを名指しするエラーになる（B10）。
    #[test]
    fn wrong_shapes_name_the_field() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        let node = add_item(conn, &["write"], "一つ目");

        // cites をオブジェクトで送る（以前は「引用が要ります」のような別の理由で断られていた）。
        let error = propose(
            conn,
            &placed.roots,
            &defs,
            &agent(),
            &json!({
                "op": "add_finding",
                "node": format!("N{node}"),
                "text": "知見",
                "cites": {"1": "#37"},
                "polarity": "support",
                "reason": "確かめた",
            }),
        )
        .expect_err("オブジェクトの cites");
        let text = message(&error);
        assert!(text.starts_with("cites は文字列の配列です"), "{text}");

        // 知らない向き。
        let error = propose(
            conn,
            &placed.roots,
            &defs,
            &agent(),
            &json!({
                "op": "add_finding",
                "node": format!("N{node}"),
                "text": "知見",
                "cites": ["#37"],
                "polarity": "positive",
                "reason": "確かめた",
            }),
        )
        .expect_err("知らない向き");
        let text = message(&error);
        assert!(
            text.starts_with("polarity は support / refute / neutral / none のどれかです"),
            "{text}"
        );
        assert!(text.contains("受け取った値: \"positive\""), "{text}");

        // from を数で送る（線の op は sci-t の sci_ops に無いので人の操作で確かめる）。
        let error = apply_user(
            conn,
            &placed.roots,
            &defs,
            &json!({
                "op": "add_edge",
                "kind": "depends",
                "from": node,
                "to": "N2",
            }),
        )
        .expect_err("数の from");
        let text = message(&error);
        assert!(
            text.starts_with("from は NODE の参照（N12 の形）です"),
            "{text}"
        );
        assert!(text.contains("受け取った形: 数"), "{text}");

        // N の形ではない参照。
        let error = check_shapes("reopen", &json!({ "node": "12" })).expect_err("N の形");
        let text = message(&error);
        assert!(
            text.starts_with("node は ITEM の NODE 参照（N12 の形）です"),
            "{text}"
        );
        assert!(text.contains("受け取った値: \"12\""), "{text}");

        // 要素が文字列でない配列。
        let error = check_shapes("add_finding", &json!({ "cites": ["#37", 3] }))
            .expect_err("数を含む cites");
        assert!(
            message(&error).contains("cites は文字列の配列です"),
            "{}",
            message(&error)
        );

        // 写しの配列。
        let error = check_shapes("decide", &json!({ "rejected": ["N21"] })).expect_err("rejected");
        assert!(message(&error).starts_with("rejected は REJECTED にした OPTION の配列です"));

        // 真偽。
        let error = check_shapes("decide", &json!({ "tentative": "yes" })).expect_err("真偽");
        assert!(
            message(&error).starts_with("tentative は真偽です（一応の決めかどうか）"),
            "{}",
            message(&error)
        );

        // 表に無い項目は今どおり無視する。
        check_shapes("reopen", &json!({ "node": "N1", "unknown": {"a": 1} })).expect("未知の項目");
        // 無印（null）は「無い」と同じ。
        check_shapes("reopen", &json!({ "node": "N1", "text": null })).expect("無印");
    }

    /// 形の違う提案は、まとめて出しても何件目かが付く。
    #[test]
    fn wrong_shapes_report_the_index() {
        let database = db();
        let conn = database.connection();
        let placed = fixture();
        let defs = defs();
        fill_library(conn);
        let node = add_item(conn, &["write"], "一つ目");
        let error = propose(
            conn,
            &placed.roots,
            &defs,
            &agent(),
            &json!({
                "proposals": [
                    {"op": "link", "node": format!("N{node}"), "target": "paper:37",
                     "role": "exemplar", "reason": "結ぶ"},
                    {"op": "add_finding", "node": format!("N{node}"), "text": "知見",
                     "cites": {"1": "#37"}, "polarity": "support", "reason": "確かめた"},
                ],
            }),
        )
        .expect_err("2 件目が悪い");
        let text = message(&error);
        assert!(
            text.starts_with("2 件目: cites は文字列の配列です"),
            "{text}"
        );
    }

    /// schema は表から作られ、14 種類の op を `oneOf` で持つ（AC2）。
    #[test]
    fn propose_schema_comes_from_the_table() {
        let schema = propose_schema();
        let one_of = schema["oneOf"].as_array().expect("oneOf");
        // 提案に出せる op は 14 種類（KNOWN_OPS 19 種類から利用者だけの 5 種類を引く）。
        assert_eq!(one_of.len(), 14);
        assert_eq!(one_of.len(), KNOWN_OPS.len() - USER_ONLY_OPS.len());
        for spec in OP_FIELDS {
            if USER_ONLY_OPS.contains(&spec.op) {
                assert!(
                    !one_of.iter().any(|part| part["title"] == spec.op),
                    "利用者だけの操作が並んでいる: {}",
                    spec.op
                );
                continue;
            }
            let part = one_of
                .iter()
                .find(|part| part["title"] == spec.op)
                .unwrap_or_else(|| panic!("{} が oneOf に無い", spec.op));
            assert_eq!(part["properties"]["op"]["const"], spec.op);
            // op と reason は提案に必ず要る。
            let required = part["required"].as_array().expect("required");
            assert!(required.contains(&json!("op")), "{}", spec.op);
            assert!(required.contains(&json!("reason")), "{}", spec.op);
            for field in spec.fields {
                assert!(
                    required.contains(&json!(field.key)) == field.required,
                    "{} の {}",
                    spec.op,
                    field.key
                );
                assert!(
                    part["properties"].get(field.key).is_some(),
                    "{} に {} が無い",
                    spec.op,
                    field.key
                );
            }
        }
        // cites は文字列の配列。
        let finding = one_of
            .iter()
            .find(|part| part["title"] == "add_finding")
            .expect("add_finding");
        assert_eq!(
            finding["properties"]["cites"],
            json!({"type": "array", "items": {"type": "string"}})
        );
        // from / to は N の形。
        let edge = one_of
            .iter()
            .find(|part| part["title"] == "add_edge")
            .expect("add_edge");
        assert_eq!(edge["properties"]["from"]["pattern"], "^N[1-9][0-9]{0,8}$");
        assert_eq!(edge["properties"]["to"]["pattern"], "^N[1-9][0-9]{0,8}$");
        // 線の向きが説明に入る。
        assert!(
            edge["description"]
                .as_str()
                .unwrap_or("")
                .contains(EDGE_DIRECTION),
            "{}",
            edge["description"]
        );
    }

    /// 検査は表と食い違わない（表の項目が `check_*` の形を壊さない）。
    #[test]
    fn every_table_field_is_known_to_the_check() {
        for spec in OP_FIELDS {
            for field in spec.fields {
                // 参照の表は parse_ref と同じ形。
                if let FieldKind::Ref(prefix) | FieldKind::RefArray(prefix) = field.kind {
                    assert!(
                        is_ref(&format!("{prefix}12"), prefix),
                        "{} の {}",
                        spec.op,
                        field.key
                    );
                    assert!(!is_ref("12", prefix), "{} の {}", spec.op, field.key);
                    assert!(
                        !is_ref(&format!("{prefix}0"), prefix),
                        "{} の {}",
                        spec.op,
                        field.key
                    );
                }
                // 値の表は KNOWN のどれか。
                if let FieldKind::Enum(values) = field.kind {
                    assert!(!values.is_empty(), "{} の {}", spec.op, field.key);
                }
            }
        }
    }
}

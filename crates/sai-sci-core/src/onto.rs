//! `sai-sci onto` — 医療の ontology（MONDO・MeSH）を読み取り専用で引く。
//!
//! 入力は `SAI_SHARED_DIR` のフォルダにある `onto.db`（U09 が作る）。事実だけを返す。
//! 粒度のずれの警告・score・閾値は作らない（判断は LabAgent に任せる）。
//! 木の DB・プロジェクトのフォルダ・ネットワークは使わない。

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

/// 共有データのフォルダを指す環境変数。
const SHARED_DIR_ENV: &str = "SAI_SHARED_DIR";
/// 対応する `onto.db` のスキーマの版。
const SCHEMA_VERSION: &str = "1";
/// 祖先をたどる段数の上限。DAG なので通常は届かない（循環したデータで無限に回らないための見張り）。
const MAX_WALK: i64 = 100;
/// 子・別名を返す上限。
const CHILD_LIMIT: usize = 50;
const SYNONYM_LIMIT: usize = 30;

/// 読み取り専用で開いた `onto.db`。
#[derive(Debug)]
pub struct Onto {
    conn: Connection,
}

/// `onto` の入力の誤り（終了コード 2）と見つからない（終了コード 3）。CLI が約束の終了コードに直す。
#[derive(Debug)]
pub enum OntoError {
    /// 引数・ID の形の誤り。
    Input(String),
    /// 共有データ・ID が見つからない。
    NotFound(String),
}

impl OntoError {
    fn input(message: impl Into<String>) -> Self {
        Self::Input(message.into())
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }
}

impl std::fmt::Display for OntoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(m) | Self::NotFound(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for OntoError {}

/// 概念 1 件の参照（ID と label）。
#[derive(Debug, Clone, Serialize)]
pub struct Link {
    /// 概念の ID（`MONDO:<数字>` か `MESH:<UI>`）。
    pub id: String,
    /// 項目名。
    pub label: String,
}

/// 版の情報（`meta` から）。全部の応答に載る。
#[derive(Debug, Clone, Serialize)]
pub struct MapVersion {
    /// MONDO の版。
    pub mondo: Option<String>,
    /// MeSH の版。
    pub mesh: Option<String>,
    /// 作った日時。
    pub built_at: Option<String>,
}

/// `lookup` の応答。
#[derive(Debug, Serialize)]
pub struct Lookup {
    /// 利用者が入れた語。
    pub query: String,
    /// 版の情報。
    pub map_version: MapVersion,
    /// 候補（完全一致→前方→部分の順）。
    pub candidates: Vec<Candidate>,
}

/// 検索の候補 1 件。
#[derive(Debug, Serialize)]
pub struct Candidate {
    /// 概念の ID。
    pub id: String,
    /// `mondo` か `mesh`。
    pub source: String,
    /// 項目名。
    pub label: String,
    /// 一致の強さ（`exact`・`prefix`・`contains`）。
    #[serde(rename = "match")]
    pub kind: &'static str,
    /// 一致した語（別名の元の書き方）。
    pub matched: String,
    /// 直接の親。
    pub parents: Vec<Link>,
}

/// 子の一覧（総数と先頭の件）。
#[derive(Debug, Serialize)]
pub struct Children {
    /// 子の総数。
    pub total: usize,
    /// 先頭の件（ID 順・上限あり）。
    pub items: Vec<Link>,
}

/// `show` の応答。
#[derive(Debug, Serialize)]
pub struct Detail {
    /// 概念の ID。
    pub id: String,
    /// `mondo` か `mesh`。
    pub source: String,
    /// 項目名。
    pub label: String,
    /// 別名（label 以外・上限あり）。
    pub synonyms: Vec<String>,
    /// 直接の親（全部）。
    pub parents: Vec<Link>,
    /// 直接の子。
    pub children: Children,
    /// 同じ概念の他の ID。
    pub xref: Vec<String>,
    /// 根までの最短距離（参考値。判定には使わない）。
    pub depth: Option<i64>,
    /// 版の情報。
    pub map_version: MapVersion,
}

/// 最も近い共通の祖先（`relation` の unrelated のとき）。
#[derive(Debug, Serialize)]
pub struct CommonAncestor {
    /// 概念の ID。
    pub id: String,
    /// 項目名。
    pub label: String,
    /// a からの距離。
    pub a_distance: i64,
    /// b からの距離。
    pub b_distance: i64,
}

/// `relation` の応答。
#[derive(Debug, Serialize)]
pub struct Relation {
    /// 1 つ目の概念。
    pub a: Link,
    /// 2 つ目の概念。
    pub b: Link,
    /// `same`・`descendant`（a は b の子孫）・`ancestor`（a は b の祖先）・`unrelated`・`different_source`。
    pub relation: &'static str,
    /// 祖先・子孫のときの最短の段数。それ以外は null。
    pub distance: Option<i64>,
    /// unrelated のときの共通の祖先（最大 3 件）。それ以外は空。
    pub nearest_common_ancestors: Vec<CommonAncestor>,
    /// a の根までの最短距離（参考値）。
    pub depth_a: Option<i64>,
    /// b の根までの最短距離（参考値）。
    pub depth_b: Option<i64>,
    /// 双方の xref に相手の ID があるか。
    pub xref_between: bool,
    /// 版の情報。
    pub map_version: MapVersion,
}

/// 祖先 1 件。
#[derive(Debug, Serialize)]
pub struct Ancestor {
    /// 概念の ID。
    pub id: String,
    /// 項目名。
    pub label: String,
    /// 最短の段数。
    pub distance: i64,
}

/// `ancestors` の応答。
#[derive(Debug, Serialize)]
pub struct Ancestors {
    /// 対象の ID。
    pub id: String,
    /// 祖先（距離の昇順・同じなら ID 順）。
    pub ancestors: Vec<Ancestor>,
    /// 版の情報。
    pub map_version: MapVersion,
}

impl Onto {
    /// `dir/onto.db` を読み取り専用で開く。無ければ「見つからない」（終了コード 3）。
    pub fn open(dir: &Path) -> Result<Onto> {
        let path = dir.join("onto.db");
        if !path.is_file() {
            return Err(not_found_data());
        }
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA query_only = ON;")?;
        let version: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if version.as_deref() != Some(SCHEMA_VERSION) {
            return Err(anyhow!(
                "onto.db の schema_version が {SCHEMA_VERSION} ではありません（{version:?}）"
            ));
        }
        Ok(Onto { conn })
    }

    /// 環境変数 `SAI_SHARED_DIR` の `onto.db` を開く。未設定なら「見つからない」（終了コード 3）。
    pub fn from_env() -> Result<Onto> {
        match std::env::var(SHARED_DIR_ENV) {
            Ok(dir) if !dir.is_empty() => Onto::open(Path::new(&dir)),
            _ => Err(not_found_data()),
        }
    }

    /// 語を引く。完全一致→前方一致→部分一致の順。同じ概念は 1 件にまとめる。
    pub fn lookup(&self, query: &str, source: Option<&str>, limit: usize) -> Result<Lookup> {
        let q = norm(query);
        if q.is_empty() {
            return Err(OntoError::input("検索の語が空です").into());
        }
        // ponytail: 部分一致は全件走査（LIKE '%…%' と同じく索引が効かない）。遅くなったら FTS を足す。
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.source, t.label, s.text, s.norm FROM synonym s \
             JOIN term t ON t.id = s.term_id \
             WHERE instr(s.norm, ?1) > 0 AND (?2 IS NULL OR t.source = ?2)",
        )?;
        let rows = stmt.query_map(rusqlite::params![q, source], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        // 概念ごとに最も強い一致を採る（同じ強さなら短い別名・辞書順）。
        let mut best: HashMap<String, ((u8, usize, String), Candidate)> = HashMap::new();
        for row in rows {
            let (id, source, label, text, norm_text) = row?;
            let kind: u8 = if norm_text == q {
                0
            } else if norm_text.starts_with(&q) {
                1
            } else {
                2
            };
            let rank = (kind, text.chars().count(), text.clone());
            if best.get(&id).is_some_and(|(old, _)| *old <= rank) {
                continue;
            }
            let candidate = Candidate {
                id: id.clone(),
                source,
                label,
                kind: ["exact", "prefix", "contains"][usize::from(kind)],
                matched: text,
                parents: Vec::new(),
            };
            best.insert(id, (rank, candidate));
        }
        // 完全一致→前方→部分の順。同じ強さなら label が短い順→ID 順。
        let mut found: Vec<(u8, usize, Candidate)> = best
            .into_values()
            .map(|((kind, _, _), c)| (kind, c.label.chars().count(), c))
            .collect();
        found.sort_by(|x, y| (x.0, x.1, &x.2.id).cmp(&(y.0, y.1, &y.2.id)));
        found.truncate(limit);
        let mut candidates = Vec::with_capacity(found.len());
        for (_, _, mut c) in found {
            c.parents = self.parents(&c.id)?;
            candidates.push(c);
        }
        Ok(Lookup {
            query: query.to_string(),
            map_version: self.map_version()?,
            candidates,
        })
    }

    /// 概念 1 件の詳細。ID が無ければ「見つからない」。
    pub fn show(&self, id: &str) -> Result<Detail> {
        check_id(id)?;
        let (source, label) = self.term(id)?;
        let synonyms = self.column_strings(
            "SELECT DISTINCT text FROM synonym WHERE term_id = ?1 AND is_label = 0 ORDER BY text LIMIT ?2",
            rusqlite::params![id, SYNONYM_LIMIT as i64],
        )?;
        let total: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM edge WHERE parent = ?1", [id], |row| {
                    row.get(0)
                })?;
        let items = self.links(
            "SELECT e.child, t.label FROM edge e JOIN term t ON t.id = e.child \
             WHERE e.parent = ?1 ORDER BY e.child LIMIT ?2",
            rusqlite::params![id, CHILD_LIMIT as i64],
        )?;
        Ok(Detail {
            id: id.to_string(),
            source,
            label,
            synonyms,
            parents: self.parents(id)?,
            children: Children {
                total: usize::try_from(total)?,
                items,
            },
            xref: self.column_strings(
                "SELECT other FROM xref WHERE term_id = ?1 ORDER BY other",
                [id],
            )?,
            depth: self.depth(id)?,
            map_version: self.map_version()?,
        })
    }

    /// 2 概念の関係。同じ source なら祖先の集合で判定する（深さは参考値で判定に使わない）。
    pub fn relation(&self, a: &str, b: &str) -> Result<Relation> {
        check_id(a)?;
        check_id(b)?;
        let (source_a, label_a) = self.term(a)?;
        let (source_b, label_b) = self.term(b)?;
        let xref_between: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM xref WHERE (term_id = ?1 AND other = ?2) OR (term_id = ?2 AND other = ?1)",
            [a, b],
            |row| row.get(0),
        )?;
        let mut out = Relation {
            a: Link {
                id: a.to_string(),
                label: label_a.clone(),
            },
            b: Link {
                id: b.to_string(),
                label: label_b.clone(),
            },
            relation: "unrelated",
            distance: None,
            nearest_common_ancestors: Vec::new(),
            depth_a: self.depth(a)?,
            depth_b: self.depth(b)?,
            xref_between: xref_between > 0,
            map_version: self.map_version()?,
        };
        if source_a != source_b {
            out.relation = "different_source";
            return Ok(out);
        }
        if a == b {
            out.relation = "same";
            return Ok(out);
        }
        let anc_a = self.closure(a)?;
        let anc_b = self.closure(b)?;
        if let Some(&d) = anc_a.get(b) {
            out.relation = "descendant";
            out.distance = Some(d);
        } else if let Some(&d) = anc_b.get(a) {
            out.relation = "ancestor";
            out.distance = Some(d);
        } else {
            // 共通の祖先は、距離の合計が小さい順に最大 3 件。
            let mut common: Vec<(&String, i64, i64)> = anc_a
                .iter()
                .filter_map(|(id, &da)| anc_b.get(id).map(|&db| (id, da, db)))
                .collect();
            common.sort_by(|x, y| (x.1 + x.2, x.0).cmp(&(y.1 + y.2, y.0)));
            for (id, a_distance, b_distance) in common.into_iter().take(3) {
                let (_, label) = self.term(id)?;
                out.nearest_common_ancestors.push(CommonAncestor {
                    id: id.clone(),
                    label,
                    a_distance,
                    b_distance,
                });
            }
        }
        Ok(out)
    }

    /// 祖先（最短距離の昇順・同じなら ID 順）。`max_depth` があればその段数までにする。
    pub fn ancestors(&self, id: &str, max_depth: Option<usize>) -> Result<Ancestors> {
        check_id(id)?;
        self.term(id)?;
        let limit = max_depth.map_or(i64::MAX, |n| i64::try_from(n).unwrap_or(i64::MAX));
        let mut list = Vec::new();
        for (ancestor, distance) in self.closure(id)? {
            if distance == 0 || distance > limit {
                continue;
            }
            let (_, label) = self.term(&ancestor)?;
            list.push(Ancestor {
                id: ancestor,
                label,
                distance,
            });
        }
        list.sort_by(|x, y| (x.distance, &x.id).cmp(&(y.distance, &y.id)));
        Ok(Ancestors {
            id: id.to_string(),
            ancestors: list,
            map_version: self.map_version()?,
        })
    }

    /// 版の情報。
    fn map_version(&self) -> Result<MapVersion> {
        let mut version = MapVersion {
            mondo: None,
            mesh: None,
            built_at: None,
        };
        let mut stmt = self.conn.prepare("SELECT key, value FROM meta")?;
        for row in stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (key, value) = row?;
            match key.as_str() {
                "mondo_version" => version.mondo = Some(value),
                "mesh_version" => version.mesh = Some(value),
                "built_at" => version.built_at = Some(value),
                _ => {}
            }
        }
        Ok(version)
    }

    /// 概念の source と label。無ければ「見つからない」。
    fn term(&self, id: &str) -> Result<(String, String)> {
        self.conn
            .query_row(
                "SELECT source, label FROM term WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| OntoError::not_found(format!("ID が見つかりません: {id}")).into())
    }

    /// 直接の親。
    fn parents(&self, id: &str) -> Result<Vec<Link>> {
        self.links(
            "SELECT e.parent, t.label FROM edge e JOIN term t ON t.id = e.parent \
             WHERE e.child = ?1 ORDER BY e.parent",
            [id],
        )
    }

    /// 自分を含む祖先の集合（ID → 最短の段数）。循環の見張りは [`MAX_WALK`]。
    fn closure(&self, id: &str) -> Result<HashMap<String, i64>> {
        let mut stmt = self.conn.prepare(&format!(
            "WITH RECURSIVE anc(id, d) AS (\
               SELECT ?1, 0 \
               UNION \
               SELECT e.parent, anc.d + 1 FROM anc JOIN edge e ON e.child = anc.id WHERE anc.d < {MAX_WALK}\
             ) SELECT id, MIN(d) FROM anc GROUP BY id"
        ))?;
        let rows = stmt.query_map([id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
    }

    /// 根（親の無い概念）までの最短距離。根の無い経路だけなら None。
    fn depth(&self, id: &str) -> Result<Option<i64>> {
        let depth: Option<i64> = self.conn.query_row(
            &format!(
                "WITH RECURSIVE anc(id, d) AS (\
                   SELECT ?1, 0 \
                   UNION \
                   SELECT e.parent, anc.d + 1 FROM anc JOIN edge e ON e.child = anc.id WHERE anc.d < {MAX_WALK}\
                 ) SELECT MIN(anc.d) FROM anc \
                 WHERE NOT EXISTS (SELECT 1 FROM edge WHERE edge.child = anc.id)"
            ),
            [id],
            |row| row.get(0),
        )?;
        Ok(depth)
    }

    /// 文字列の 1 列を読む。
    fn column_strings(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(sql)?;
        Ok(stmt
            .query_map(params, |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// ID と label の組を読む。
    fn links(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Link>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params, |row| {
            Ok(Link {
                id: row.get(0)?,
                label: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// 共有データが無いときのエラー（終了コード 3）。
fn not_found_data() -> anyhow::Error {
    OntoError::not_found(
        "共有データが見つかりません。SAI_SHARED_DIR が未設定か、onto.db がありません",
    )
    .into()
}

/// ID の形（`MONDO:<数字>` か `MESH:<英数字>`）を確かめる。違えば入力の誤り（終了コード 2）。
fn check_id(id: &str) -> Result<()> {
    let ok = match (id.strip_prefix("MONDO:"), id.strip_prefix("MESH:")) {
        (Some(n), _) => !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()),
        (None, Some(u)) => !u.is_empty() && u.bytes().all(|b| b.is_ascii_alphanumeric()),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(OntoError::input(format!(
            "ID の形が違います: {id}（MONDO:<数字> か MESH:<英数字>）"
        ))
        .into())
    }
}

/// 検索の正規化（U09 の `synonym.norm` と同じ）。
///
/// ponytail: NFKC の代わりに、全角 ASCII（U+FF01〜U+FF5E）と全角空白だけを半角にする。
/// 他の互換文字（丸数字・合字など）は変わらない。`to_lowercase` は casefold の代わり。
/// 足りなければ `unicode-normalization` を足すが、新しい依存なので今は入れない。
pub fn norm(text: &str) -> String {
    let mapped: String = text
        .chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect();
    mapped
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 試験の中だけで使う `onto.db` を作る（スキーマは U09 と同じ。script には依存しない）。
    fn fixture(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sai-sci-onto-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("onto.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE term(id TEXT PRIMARY KEY, source TEXT NOT NULL, label TEXT NOT NULL);
             CREATE TABLE synonym(term_id TEXT NOT NULL, text TEXT NOT NULL, norm TEXT NOT NULL, is_label INTEGER NOT NULL);
             CREATE TABLE edge(child TEXT NOT NULL, parent TEXT NOT NULL, PRIMARY KEY(child, parent));
             CREATE TABLE xref(term_id TEXT NOT NULL, other TEXT NOT NULL);
             INSERT INTO meta VALUES ('schema_version','1'), ('mondo_version','releases/2026-10-06'),
               ('mesh_version','2026'), ('built_at','2026-10-12T00:00:00+09:00');
             INSERT INTO term VALUES
               ('MONDO:0000001','mondo','disease'),
               ('MONDO:0000100','mondo','nervous system disorder'),
               ('MONDO:0000200','mondo','brain disorder'),
               ('MONDO:0005027','mondo','epilepsy'),
               ('MONDO:0000300','mondo','spasm group'),
               ('MONDO:0018097','mondo','infantile spasms'),
               ('MONDO:0000500','mondo','spasm'),
               ('MONDO:0000600','mondo','spasms, epileptic'),
               ('MONDO:0007743','mondo','attention deficit hyperactivity disorder'),
               ('MONDO:0700092','mondo','neurodevelopmental disorder'),
               ('MESH:D013036','mesh','Spasms, Infantile'),
               ('MESH:D000001','mesh','parent A'),
               ('MESH:D000002','mesh','parent B');
             INSERT INTO synonym SELECT id, label, lower(label), 1 FROM term;
             INSERT INTO synonym VALUES
               ('MONDO:0018097','West syndrome','west syndrome',0),
               ('MONDO:0018097','IESS','iess',0),
               ('MONDO:0018097','infantile epileptic spasms syndrome','infantile epileptic spasms syndrome',0),
               ('MESH:D013036','Infantile Spasms','infantile spasms',0);
             INSERT INTO edge VALUES
               ('MONDO:0000100','MONDO:0000001'), ('MONDO:0000200','MONDO:0000100'),
               ('MONDO:0005027','MONDO:0000200'), ('MONDO:0000300','MONDO:0000001'),
               ('MONDO:0018097','MONDO:0005027'), ('MONDO:0018097','MONDO:0000300'),
               ('MONDO:0000500','MONDO:0000001'),
               ('MONDO:0007743','MONDO:0000100'), ('MONDO:0700092','MONDO:0000100'),
               ('MESH:D013036','MESH:D000001'), ('MESH:D013036','MESH:D000002');
             INSERT INTO xref VALUES ('MONDO:0018097','MESH:D013036'), ('MESH:D013036','MONDO:0018097');",
        )
        .unwrap();
        dir
    }

    fn code_of(err: &anyhow::Error) -> i32 {
        match err.downcast_ref::<OntoError>() {
            Some(OntoError::Input(_)) => 2,
            Some(OntoError::NotFound(_)) => 3,
            None => 1,
        }
    }

    #[test]
    fn lookup_orders_exact_prefix_contains_and_merges_per_concept() {
        let onto = Onto::open(&fixture("lookup")).unwrap();
        let out = onto.lookup("spasm", None, 10).unwrap();
        let ids: Vec<&str> = out.candidates.iter().map(|c| c.id.as_str()).collect();
        // 前方一致の中は label が短い順（同じ長さは ID 順）
        assert_eq!(
            ids,
            [
                "MONDO:0000500",
                "MONDO:0000300",
                "MESH:D013036",
                "MONDO:0000600",
                "MONDO:0018097"
            ]
        );
        let kinds: Vec<&str> = out.candidates.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, ["exact", "prefix", "prefix", "prefix", "contains"]);
        // 0018097 は 2 つの別名で当たるが 1 件にまとまり、短い方の別名を一致として採る
        assert_eq!(out.candidates[4].matched, "infantile spasms");
        assert_eq!(
            out.map_version.mondo.as_deref(),
            Some("releases/2026-10-06")
        );
    }

    #[test]
    fn lookup_normalizes_fullwidth_and_case_and_filters_source() {
        let onto = Onto::open(&fixture("norm")).unwrap();
        let out = onto.lookup("ＩＥＳＳ", None, 10).unwrap();
        assert_eq!(out.candidates[0].id, "MONDO:0018097");
        assert_eq!(out.candidates[0].matched, "IESS");
        assert_eq!(out.candidates[0].parents.len(), 2);
        let mesh = onto.lookup("Infantile SPASMS", Some("mesh"), 10).unwrap();
        assert_eq!(mesh.candidates.len(), 1);
        assert_eq!(mesh.candidates[0].id, "MESH:D013036");
        assert_eq!(norm("　Ａｚ ９  "), "az 9");
    }

    #[test]
    fn show_returns_parents_children_xref_synonyms_and_depth() {
        let onto = Onto::open(&fixture("show")).unwrap();
        let d = onto.show("MONDO:0018097").unwrap();
        assert_eq!(d.label, "infantile spasms");
        assert_eq!(
            d.synonyms,
            [
                "IESS",
                "West syndrome",
                "infantile epileptic spasms syndrome"
            ]
        );
        let parents: Vec<&str> = d.parents.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(parents, ["MONDO:0000300", "MONDO:0005027"]);
        assert_eq!(d.children.total, 0);
        assert_eq!(d.xref, ["MESH:D013036"]);
        // 浅い方の親（spasm group）から根まで 2 段
        assert_eq!(d.depth, Some(2));
        let root = onto.show("MONDO:0000001").unwrap();
        assert_eq!(root.depth, Some(0));
        assert_eq!(root.children.total, 3);
    }

    #[test]
    fn relation_judges_by_ancestor_sets_even_with_multiple_parents() {
        let onto = Onto::open(&fixture("relation")).unwrap();
        let r = onto.relation("MONDO:0018097", "MONDO:0005027").unwrap();
        assert_eq!((r.relation, r.distance), ("descendant", Some(1)));
        let r = onto.relation("MONDO:0005027", "MONDO:0018097").unwrap();
        assert_eq!((r.relation, r.distance), ("ancestor", Some(1)));
        // 多重親: IESS から nervous system disorder へは epilepsy 経由の 3 段（浅い親の経路は届かない）
        let r = onto.relation("MONDO:0018097", "MONDO:0000100").unwrap();
        assert_eq!((r.relation, r.distance), ("descendant", Some(3)));
        // 多重親: disease へは浅い親（spasm group）経由の 2 段
        let r = onto.relation("MONDO:0018097", "MONDO:0000001").unwrap();
        assert_eq!(r.distance, Some(2));
        let r = onto.relation("MONDO:0007743", "MONDO:0700092").unwrap();
        assert_eq!(r.relation, "unrelated");
        assert_eq!(r.distance, None);
        let nearest = &r.nearest_common_ancestors;
        assert_eq!(nearest[0].id, "MONDO:0000100");
        assert_eq!((nearest[0].a_distance, nearest[0].b_distance), (1, 1));
        assert!(nearest.len() <= 3);
        let r = onto.relation("MONDO:0000100", "MONDO:0000100").unwrap();
        assert_eq!(r.relation, "same");
        let r = onto.relation("MONDO:0018097", "MESH:D013036").unwrap();
        assert_eq!(r.relation, "different_source");
        assert!(r.xref_between);
        assert!(r.nearest_common_ancestors.is_empty());
    }

    #[test]
    fn ancestors_sorted_by_distance_and_limited() {
        let onto = Onto::open(&fixture("anc")).unwrap();
        let all = onto.ancestors("MONDO:0018097", None).unwrap();
        let got: Vec<(&str, i64)> = all
            .ancestors
            .iter()
            .map(|a| (a.id.as_str(), a.distance))
            .collect();
        assert_eq!(
            got,
            [
                ("MONDO:0000300", 1),
                ("MONDO:0005027", 1),
                ("MONDO:0000001", 2),
                ("MONDO:0000200", 2),
                ("MONDO:0000100", 3),
            ]
        );
        assert_eq!(
            onto.ancestors("MONDO:0018097", Some(1))
                .unwrap()
                .ancestors
                .len(),
            2
        );
    }

    #[test]
    fn errors_have_contract_codes() {
        let dir = fixture("errors");
        let onto = Onto::open(&dir).unwrap();
        assert_eq!(code_of(&onto.show("MONDO:9999999").unwrap_err()), 3);
        assert_eq!(code_of(&onto.show("foo").unwrap_err()), 2);
        assert_eq!(
            code_of(&onto.relation("MONDO:", "MONDO:0000001").unwrap_err()),
            2
        );
        assert_eq!(code_of(&onto.lookup("   ", None, 10).unwrap_err()), 2);
        // 読み取り専用（書き込みは失敗する）
        assert!(onto.conn.execute("DELETE FROM term", []).is_err());
        assert_eq!(code_of(&Onto::open(&dir.join("nope")).unwrap_err()), 3);
    }

    #[test]
    fn open_rejects_other_schema_version() {
        let dir = fixture("schema");
        Connection::open(dir.join("onto.db"))
            .unwrap()
            .execute(
                "UPDATE meta SET value = '2' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        assert!(Onto::open(&dir).is_err());
    }

    /// 合成データ（3 万語・多重親あり）の速さを見る。`cargo test -p sai-sci-core -- --ignored --nocapture` で動かす。
    #[test]
    #[ignore]
    fn perf_thirty_thousand_terms() {
        let dir = fixture("perf");
        let conn = Connection::open(dir.join("onto.db")).unwrap();
        let mut seed: u64 = 42;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        conn.execute_batch(
            "DELETE FROM term; DELETE FROM synonym; DELETE FROM edge; DELETE FROM xref;",
        )
        .unwrap();
        let n = 30_000;
        let tx = conn.unchecked_transaction().unwrap();
        for i in 0..n {
            let id = format!("MONDO:{i:07}");
            let label = format!("term {i}");
            tx.execute(
                "INSERT INTO term VALUES (?1, 'mondo', ?2)",
                rusqlite::params![id, label],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO synonym VALUES (?1, ?2, ?3, 1)",
                rusqlite::params![id, label, norm(&label)],
            )
            .unwrap();
            if i > 0 {
                let p1 = next() % i;
                tx.execute(
                    "INSERT INTO edge VALUES (?1, ?2)",
                    rusqlite::params![id, format!("MONDO:{p1:07}")],
                )
                .unwrap();
                if i > 1 && next() % 2 == 0 {
                    let p2 = next() % i;
                    if p2 != p1 {
                        tx.execute(
                            "INSERT INTO edge VALUES (?1, ?2)",
                            rusqlite::params![id, format!("MONDO:{p2:07}")],
                        )
                        .unwrap();
                    }
                }
            }
        }
        tx.commit().unwrap();
        let onto = Onto::open(&dir).unwrap();
        let t = std::time::Instant::now();
        let deep = format!("MONDO:{:07}", n - 1);
        let anc = onto.ancestors(&deep, None).unwrap();
        println!(
            "ancestors({deep}): {} 件, {:?}",
            anc.ancestors.len(),
            t.elapsed()
        );
        let t = std::time::Instant::now();
        let r = onto.relation(&deep, "MONDO:0000000").unwrap();
        println!("relation: {} {:?}", r.relation, t.elapsed());
        let t = std::time::Instant::now();
        let l = onto.lookup("term 1234", None, 10).unwrap();
        println!("lookup: {} 件, {:?}", l.candidates.len(), t.elapsed());
        let t = std::time::Instant::now();
        let d = onto.show(&deep).unwrap();
        println!("show: depth {:?}, {:?}", d.depth, t.elapsed());
        let t = std::time::Instant::now();
        let l = onto.lookup("term", None, 10).unwrap();
        println!("lookup（全件に当たる語）: {:?}", t.elapsed());
        assert_eq!(l.candidates.len(), 10);
    }
}

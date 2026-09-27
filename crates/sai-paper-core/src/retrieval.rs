//! ベクトル検索と近傍チャンク検索。
//!
//! `vector_search` は埋め込みクエリから全文検索を実行し、`neighbors` は
//! 指定論文の埋め込みを平均化して似た論文チャンクを検索する。

use crate::embeddings::{self, EmbedClient};
use crate::paths::{paper_file_path, paper_folder, paper_roots};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 検索エラー。
#[derive(Debug, Error)]
pub enum RetrievalError {
    /// 埋め込みクライアントエラー。
    #[error("embedding error: {0}")]
    Embedding(#[from] embeddings::EmbeddingError),
    /// データベースエラー。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// その他のエラー。
    #[error("retrieval error: {0}")]
    Other(String),
}

/// 検索結果の型。
pub type Result<T> = std::result::Result<T, RetrievalError>;

/// 検索ヒット。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    /// コサイン類似度スコア（0-1）
    pub score: f32,
    /// チャンク ID
    pub chunk_id: i64,
    /// ソース種別（abstract/body/note/document）
    pub source_kind: String,
    /// ソース参照（paper_id や note_id など）
    pub source_ref: String,
    /// 論文 ID（ある場合）
    pub paper_id: Option<i64>,
    /// 論文タイトル（ある場合）
    pub title: Option<String>,
    /// 著者（JSON 配列文字列、ある場合）
    pub authors: Option<String>,
    /// 論文出版年（ある場合）
    pub year: Option<i32>,
    /// チャンク本文
    pub text: String,
    /// テキスト内の開始位置
    pub char_start: Option<i64>,
    /// テキスト内の終了位置
    pub char_end: Option<i64>,
    /// Markdown パス（ある場合）
    pub md_path: Option<String>,
    /// その文があった章（見出しを ` > ` でつないだもの、ある場合）
    pub section: Option<String>,
}

/// ベクトル検索オプション。
#[derive(Debug, Clone)]
pub struct VectorSearchOptions {
    /// 返す上位 K 件
    pub top_k: i64,
    /// 使用するモデル名（省略時は config のデフォルト）
    pub model: Option<String>,
    /// 索引の名前で絞る（`None` なら全索引）
    pub index: Option<String>,
    /// スコープ（papers/notes/figures/all）
    pub scope: Option<String>,
}

impl Default for VectorSearchOptions {
    fn default() -> Self {
        Self {
            top_k: 20,
            model: None,
            index: None,
            scope: None,
        }
    }
}

/// 全文検索オプション。
#[derive(Debug, Clone)]
pub struct WordSearchOptions {
    /// 返す上位 K 件
    pub top_k: i64,
    /// スコープ（papers/notes/documents/all）
    pub scope: Option<String>,
    /// 論文 ID（指定時はその論文に絞り込む）
    pub paper_id: Option<i64>,
    /// FTS5 式として渡すか（true = 生式、false = 自然言語でクォート）
    pub fts_expr: bool,
    /// 索引の名前で絞る（`None` なら全索引）
    pub index: Option<String>,
}

impl Default for WordSearchOptions {
    fn default() -> Self {
        Self {
            top_k: 20,
            scope: None,
            paper_id: None,
            fts_expr: false,
            index: None,
        }
    }
}

/// ハイブリッド検索オプション。
#[derive(Debug, Clone)]
pub struct HybridSearchOptions {
    /// 返す上位 K 件
    pub top_k: i64,
    /// スコープ（papers/notes/documents/all）
    pub scope: Option<String>,
    /// 論文 ID（指定時はその論文に絞り込む）
    pub paper_id: Option<i64>,
    /// 埋め込みモデル名（省略時は config のデフォルト）
    pub model: Option<String>,
    /// ベクトル側の重み（alpha）。word 側は 1.0 - alpha
    pub alpha: f64,
    /// 融合方法（weighted/rrf）
    pub fusion: String,
    /// FTS5 式として渡すか（true = 生式、false = 自然言語でクォート）
    pub fts_expr: bool,
    /// 索引の名前で絞る（`None` なら全索引）
    pub index: Option<String>,
}

impl Default for HybridSearchOptions {
    fn default() -> Self {
        Self {
            top_k: 20,
            scope: None,
            paper_id: None,
            model: None,
            alpha: 0.7,
            fusion: "weighted".to_string(),
            fts_expr: false,
            index: None,
        }
    }
}

/// 検索結果 1 件ぶんの行（本文とメタデータ）を読む SELECT の列。
const HIT_COLUMNS: &str = "c.id, c.source_kind, c.source_ref, c.paper_id, c.text, c.char_start, c.char_end,
     p.title, NULL, CAST(SUBSTR(p.published_at, 1, 4) AS INTEGER), c.section";

/// 論文フォルダの extract.md の場所（検索結果の `md_path`）。
fn md_path_for(paper_id: Option<i64>, title: Option<&String>) -> Option<String> {
    let (pid, title) = (paper_id?, title?);
    let roots = paper_roots()?;
    let dir = paper_folder(&roots.data_dir, pid, title);
    paper_file_path(&dir, "extract").to_str().map(str::to_string)
}

/// `HIT_COLUMNS` の 1 行を検索結果にする。
fn hit_from_row(row: &rusqlite::Row<'_>, score: f32) -> rusqlite::Result<Hit> {
    let paper_id: Option<i64> = row.get(3)?;
    let title: Option<String> = row.get(7)?;
    Ok(Hit {
        score,
        chunk_id: row.get(0)?,
        source_kind: row.get(1)?,
        source_ref: row.get(2)?,
        paper_id,
        md_path: md_path_for(paper_id, title.as_ref()),
        title,
        authors: row.get(8)?,
        year: row.get(9)?,
        text: row.get(4)?,
        char_start: row.get(5)?,
        char_end: row.get(6)?,
        section: row.get(10)?,
    })
}

/// 索引名の絞り込み（`None` なら全索引）。
fn index_where(index: Option<&str>) -> String {
    index.map_or_else(String::new, |name| {
        format!("AND c.index_name = '{}'", name.replace('\'', "''"))
    })
}

/// スコープ（papers/notes/figures/all）を `source_kind` の WHERE 句にする。
///
/// `papers` は図を含めない（図は `figures` または `all` のときだけ出す）。
fn scope_where_clause(scope: Option<&str>) -> &'static str {
    match scope {
        Some("papers") => "AND c.source_kind IN ('abstract', 'body')",
        Some("notes") => "AND c.source_kind = 'note'",
        Some("figures") => "AND c.source_kind = 'figure'",
        _ => "",
    }
}

/// 全文検索（BM25）を実行する。
///
/// # Errors
///
/// データベースエラー。
pub fn word_search(db: &Connection, query: &str, opts: WordSearchOptions) -> Result<Vec<Hit>> {
    // クエリをサニタイズまたはそのまま渡す
    let fts_query = if opts.fts_expr {
        // FTS5 式として渡す（ユーザー責任）
        query.to_string()
    } else {
        // 自然言語：各トークンをクォートして AND でつなぐ
        // FTS5 の operator characters（AND, OR, NOT, *, "） を避けるため、
        // 単語ごとにダブルクォートで囲む。
        query
            .split_whitespace()
            .map(|word| format!("\"{}\"", word))
            .collect::<Vec<_>>()
            .join(" AND ")
    };

    // スコープの where 句
    let scope_where = scope_where_clause(opts.scope.as_deref());

    // 論文 ID でのフィルタ
    let paper_where = opts
        .paper_id
        .map_or_else(String::new, |pid| format!("AND c.paper_id = {pid}"));

    // FTS5 検索結果を取得。natural order で返される（relevance descending）。
    let query_str = format!(
        "SELECT {HIT_COLUMNS}
         FROM chunks c
         LEFT JOIN papers p ON c.paper_id = p.id
         JOIN chunk_fts ON c.id = chunk_fts.rowid
         WHERE chunk_fts MATCH ?1
         {scope_where}
         {paper_where}
         {}
         LIMIT ?2",
        index_where(opts.index.as_deref())
    );

    let mut stmt = db.prepare(&query_str)?;
    let mut rank = 0usize;
    let hits = stmt
        .query_map(rusqlite::params![&fts_query, opts.top_k], |row| {
            // スコアを順位に基づいて正規化（1位 = 1.0、最後 = 接近 0.0）
            // ponytail: BM25 スコアは ORDER BY で使用。複数メトリクスとの融合時は正規化が必要。
            let score = (1.0 - (rank as f32 / opts.top_k as f32)).clamp(0.0, 1.0);
            rank += 1;
            hit_from_row(row, score)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(hits)
}

/// ベクトルの近い順に上位 `top_k` 件の (chunk_id, 類似度) を返す。
///
/// 埋め込みは 1 回の問い合わせ（`chunk_embeddings JOIN chunks`）でまとめて読み、
/// 本文は読まない（本文は上位の分だけ [`load_hits`] で後から読む）。
/// ponytail: 全走査のコサイン計算。断片が 10 万を超えて遅くなったら sqlite-vec に置き換える。
fn rank_by_vector(
    db: &Connection,
    query_vec: &[f32],
    model_id: i64,
    extra_where: &str,
    top_k: i64,
) -> Result<Vec<(i64, f32)>> {
    let mut stmt = db.prepare(&format!(
        "SELECT ce.chunk_id, ce.vec FROM chunk_embeddings ce
         JOIN chunks c ON c.id = ce.chunk_id
         WHERE ce.model_id = ?1 {extra_where}"
    ))?;
    let mut scored: Vec<(i64, f32)> = stmt
        .query_map([model_id], |row| {
            let blob: Vec<u8> = row.get(1)?;
            let vec = embeddings::from_blob(&blob).unwrap_or_default();
            Ok((row.get::<_, i64>(0)?, cosine_similarity(query_vec, &vec)))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k.max(0) as usize);
    Ok(scored)
}

/// 上位の (chunk_id, 類似度) の本文とメタデータを読み、その順で返す。
fn load_hits(db: &Connection, ranked: &[(i64, f32)]) -> Result<Vec<Hit>> {
    let mut stmt = db.prepare(&format!(
        "SELECT {HIT_COLUMNS} FROM chunks c LEFT JOIN papers p ON c.paper_id = p.id WHERE c.id = ?1"
    ))?;
    let mut hits = Vec::with_capacity(ranked.len());
    for (chunk_id, score) in ranked {
        hits.push(stmt.query_row([chunk_id], |row| hit_from_row(row, *score))?);
    }
    Ok(hits)
}

/// 埋め込みモデルの ID（未登録ならエラー）。
fn model_id_of(db: &Connection, model: &str) -> Result<i64> {
    db.query_row(
        "SELECT id FROM embedding_models WHERE name = ?1",
        [model],
        |row| row.get(0),
    )
    .map_err(|_| RetrievalError::Other(format!("model '{model}' not registered")))
}

/// ベクトル検索を実行する。
///
/// クエリテキストを埋め込み、索引の断片とのコサイン類似度で上位 K 件を返す。
///
/// # Errors
///
/// - 埋め込み生成失敗
/// - データベースエラー
pub async fn vector_search(
    db: &Connection,
    query: &str,
    client: &EmbedClient,
    opts: VectorSearchOptions,
) -> Result<Vec<Hit>> {
    let query_embeddings = client.embed(&[query.to_string()]).await?;
    let Some(query_vec) = query_embeddings.first() else {
        return Ok(Vec::new());
    };
    let model_id = model_id_of(db, client.model())?;
    let extra_where = format!(
        "{} {}",
        index_where(opts.index.as_deref()),
        scope_where_clause(opts.scope.as_deref())
    );
    let ranked = rank_by_vector(db, query_vec, model_id, &extra_where, opts.top_k)?;
    load_hits(db, &ranked)
}

/// ハイブリッド検索を実行する。
///
/// 全文検索（BM25）とベクトル検索（コサイン類似度）の結果を組み合わせて返す。
/// 融合方法は `alpha` （ウェイト合成）または RRF（逆順位融合）から選択できる。
/// ベクトル側に埋め込みがない場合は、全文検索結果のみを返す。
///
/// # Errors
///
/// - 埋め込み生成失敗
/// - データベースエラー
pub async fn hybrid_search(
    db: &Connection,
    query: &str,
    client: &EmbedClient,
    opts: HybridSearchOptions,
) -> Result<Vec<Hit>> {
    // 各側から多めに取得（融合前の材料用）
    const FETCH_MULTIPLIER: usize = 3;
    let fetch_k = (opts.top_k as usize * FETCH_MULTIPLIER).max(opts.top_k as usize);

    // 全文検索側を実行
    let word_opts = WordSearchOptions {
        top_k: fetch_k as i64,
        scope: opts.scope.clone(),
        paper_id: opts.paper_id,
        fts_expr: opts.fts_expr,
        index: opts.index.clone(),
    };
    let word_hits = word_search(db, query, word_opts)?;

    // ベクトル検索側を実行
    let vector_opts = VectorSearchOptions {
        top_k: fetch_k as i64,
        model: opts.model.clone(),
        index: opts.index.clone(),
        scope: opts.scope.clone(),
    };
    let vector_hits = match vector_search(db, query, client, vector_opts).await {
        Ok(hits) => hits,
        Err(_) => {
            // ベクトル側が失敗した場合は、全文検索結果のみを返す
            return Ok(word_hits.into_iter().take(opts.top_k as usize).collect());
        }
    };

    // ベクトル側が空の場合も全文検索のみ
    if vector_hits.is_empty() {
        return Ok(word_hits.into_iter().take(opts.top_k as usize).collect());
    }

    // 融合方法に応じて結果を統合
    let fused = if opts.fusion.to_lowercase() == "rrf" {
        fuse_rrf(&word_hits, &vector_hits)
    } else {
        // デフォルト: weighted
        fuse_weighted(&word_hits, &vector_hits, opts.alpha)
    };

    Ok(fused.into_iter().take(opts.top_k as usize).collect())
}

/// RRF（逆順位融合）で 2 つの結果セットを統合する。
///
/// 各リストでのランク（0 から始まる）に対して 1/(k + rank) を計算し、
/// チャンク ID でグループ化して合計スコアでソートする。
fn fuse_rrf(word_hits: &[Hit], vector_hits: &[Hit]) -> Vec<Hit> {
    use std::collections::HashMap;

    const K: f32 = 60.0;
    let mut fused: HashMap<i64, (f32, Hit)> = HashMap::new();

    // 全文検索側のスコアを計算
    for (rank, hit) in word_hits.iter().enumerate() {
        let rrf_score = 1.0 / (K + rank as f32);
        fused
            .entry(hit.chunk_id)
            .and_modify(|(score, _)| *score += rrf_score)
            .or_insert_with(|| (rrf_score, hit.clone()));
    }

    // ベクトル検索側のスコアを計算
    for (rank, hit) in vector_hits.iter().enumerate() {
        let rrf_score = 1.0 / (K + rank as f32);
        fused
            .entry(hit.chunk_id)
            .and_modify(|(score, _)| *score += rrf_score)
            .or_insert_with(|| (rrf_score, hit.clone()));
    }

    // スコアでソート
    let mut results: Vec<_> = fused.into_values().collect();
    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    results.into_iter().map(|(_, hit)| hit).collect()
}

/// ウェイト合成で 2 つの結果セットを統合する。
///
/// 各側の結果を 0-1 に正規化し、`score = alpha * vector + (1 - alpha) * word` で統合する。
/// 一方の側にしかない結果は、その側のスコアのみ使用する。
fn fuse_weighted(word_hits: &[Hit], vector_hits: &[Hit], alpha: f64) -> Vec<Hit> {
    use std::collections::HashMap;

    let mut fused: HashMap<i64, Hit> = HashMap::new();

    // 全文検索側を追加（正規化用に max_score を取得）
    let word_max_score = word_hits.iter().map(|h| h.score).fold(0.0, f32::max);
    let word_normalized_max = word_max_score.max(1.0);

    for hit in word_hits {
        let normalized_score = hit.score / word_normalized_max;
        let mut fused_hit = hit.clone();
        fused_hit.score = ((1.0 - alpha) as f32 * normalized_score).min(1.0);
        fused.insert(hit.chunk_id, fused_hit);
    }

    // ベクトル検索側を追加/更新
    let vector_max_score = vector_hits.iter().map(|h| h.score).fold(0.0, f32::max);
    let vector_normalized_max = vector_max_score.max(1.0);

    for hit in vector_hits {
        let normalized_score = hit.score / vector_normalized_max;
        let vector_contribution = (alpha as f32 * normalized_score).min(1.0);

        fused
            .entry(hit.chunk_id)
            .and_modify(|fused_hit| {
                // 既存：word 側のスコア
                // 追加：vector 側のスコア
                fused_hit.score = (fused_hit.score + vector_contribution).min(1.0);
            })
            .or_insert_with(|| {
                let mut fused_hit = hit.clone();
                fused_hit.score = vector_contribution;
                fused_hit
            });
    }

    // スコアでソート
    let mut results: Vec<_> = fused.into_values().collect();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results
}

/// 指定論文の断片の埋め込みを平均し、ほかの論文の近い断片を返す。
///
/// # Errors
///
/// データベースエラー。
pub fn neighbors(
    db: &Connection,
    paper_id: i64,
    model_id: i64,
    top_k: i64,
    index: Option<&str>,
) -> Result<Vec<Hit>> {
    let filter = index_where(index);
    // 論文のチャンク埋め込みを取得
    let mut stmt = db.prepare(&format!(
        "SELECT ce.vec FROM chunk_embeddings ce
         JOIN chunks c ON ce.chunk_id = c.id
         WHERE c.paper_id = ?1 AND ce.model_id = ?2 {filter}"
    ))?;
    let embeddings: Vec<Vec<f32>> = stmt
        .query_map([paper_id, model_id], |row| row.get::<_, Vec<u8>>(0))?
        .filter_map(|blob| blob.ok().and_then(|b| embeddings::from_blob(&b).ok()))
        .collect();
    if embeddings.is_empty() {
        return Ok(Vec::new());
    }
    let query_vec = average_embeddings(&embeddings);
    // 当該論文自体は除く
    let ranked = rank_by_vector(
        db,
        &query_vec,
        model_id,
        &format!("AND (c.paper_id IS NULL OR c.paper_id != {paper_id}) {filter}"),
        top_k,
    )?;
    load_hits(db, &ranked)
}

/// CLI の `find` / TUI の `f` / MCP の `find` が受け取る条件。
#[derive(Debug, Clone, Default)]
pub struct FindRequest {
    /// 検索語。
    pub query: String,
    /// word / vector / hybrid（無ければ `rag.default_mode`）。
    pub mode: Option<String>,
    /// 上位 K 件（無ければ `rag.top_k`）。
    pub top_k: Option<i64>,
    /// 1 本の論文から出す最大の件数（無ければ `rag.per_paper`。0 で制限なし）。
    pub per_paper: Option<i64>,
    /// papers / notes / documents / all。
    pub scope: Option<String>,
    /// 論文で絞る。
    pub paper_id: Option<i64>,
    /// 索引の名前（無ければ `rag.default_index`）。
    pub index: Option<String>,
    /// hybrid の重み（無ければ `rag.alpha`）。
    pub alpha: Option<f64>,
    /// weighted / rrf（無ければ `rag.fusion`）。
    pub fusion: Option<String>,
    /// 検索語を FTS5 の式として渡す。
    pub fts_expr: bool,
}

/// 設定を読んだうえで検索する（CLI・TUI・MCP の共通の入口。段階 7）。
///
/// # Errors
///
/// 索引が無い・モードが不明・埋め込みの失敗・DB エラー。
pub async fn find(db: &Connection, config: &crate::Config, req: FindRequest) -> Result<Vec<Hit>> {
    let rag = &config.rag;
    let index = rag.index(req.index.as_deref()).map_err(RetrievalError::Other)?;
    let top_k = req.top_k.unwrap_or(rag.top_k);
    let per_paper = req.per_paper.unwrap_or(rag.per_paper);
    // `paper_id` で 1 本に絞った検索と `per_paper = 0` では制限しない。
    let limited = req.paper_id.is_none() && per_paper > 0;
    // 制限で減る分を見込んで多めに取る（最大 200 件）。
    let fetch_k = if limited {
        top_k.saturating_mul(3).min(200)
    } else {
        top_k
    };
    let index_name = Some(index.name.clone());
    let hits = match req.mode.as_deref().unwrap_or(&rag.default_mode) {
        "word" => word_search(
            db,
            &req.query,
            WordSearchOptions {
                top_k: fetch_k,
                scope: req.scope,
                paper_id: req.paper_id,
                fts_expr: req.fts_expr,
                index: index_name,
            },
        ),
        "vector" => {
            let client = EmbedClient::for_index(&index);
            vector_search(
                db,
                &req.query,
                &client,
                VectorSearchOptions {
                    top_k: fetch_k,
                    model: None,
                    index: index_name,
                    scope: req.scope,
                },
            )
            .await
        }
        "hybrid" => {
            let client = EmbedClient::for_index(&index);
            hybrid_search(
                db,
                &req.query,
                &client,
                HybridSearchOptions {
                    top_k: fetch_k,
                    scope: req.scope,
                    paper_id: req.paper_id,
                    model: None,
                    alpha: req.alpha.unwrap_or(rag.alpha),
                    fusion: req.fusion.unwrap_or_else(|| rag.fusion.clone()),
                    fts_expr: req.fts_expr,
                    index: index_name,
                },
            )
            .await
        }
        other => Err(RetrievalError::Other(format!(
            "unknown search mode: '{other}'. use 'word', 'vector', or 'hybrid'"
        ))),
    }?;
    Ok(if limited {
        limit_per_paper(hits, per_paper as usize, top_k.max(0) as usize)
    } else {
        hits
    })
}

/// 上位から順に、同じ論文から出す件数を `per_paper` までに抑えて `top_k` 件を返す。
///
/// `per_paper` が 0 のときは制限しない。`paper_id` の無い結果は制限しない。
/// `top_k` 件そろったらそこで止める。
#[must_use]
pub fn limit_per_paper(mut hits: Vec<Hit>, per_paper: usize, top_k: usize) -> Vec<Hit> {
    if per_paper == 0 {
        hits.truncate(top_k);
        return hits;
    }
    let mut counts: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    let mut out: Vec<Hit> = Vec::with_capacity(top_k.min(hits.len()));
    for hit in hits.drain(..) {
        if out.len() >= top_k {
            break;
        }
        if let Some(paper_id) = hit.paper_id {
            let count = counts.entry(paper_id).or_insert(0);
            if *count >= per_paper {
                continue;
            }
            *count += 1;
        }
        out.push(hit);
    }
    out
}

/// 設定を読んだうえで近傍の断片を探す（CLI・MCP の共通の入口）。
///
/// # Errors
///
/// 索引が無い・その索引の埋め込みが無い・DB エラー。
pub fn find_neighbors(
    db: &Connection,
    config: &crate::Config,
    paper_id: i64,
    top_k: Option<i64>,
    index: Option<&str>,
) -> Result<Vec<Hit>> {
    let index = config.rag.index(index).map_err(RetrievalError::Other)?;
    let model_id = model_id_of(db, &index.embed_model).map_err(|_| {
        RetrievalError::Other(format!(
            "索引 '{}' の埋め込み（{}）がありません。先に `sai-paper index embed --index {}` を実行してください",
            index.name, index.embed_model, index.name
        ))
    })?;
    neighbors(db, paper_id, model_id, top_k.unwrap_or(20), Some(&index.name))
}

/// そのまま report に貼れる引用の文字列（Library の論文なら `[#37]`）。論文でない結果は None。
///
/// 章があれば `[#37 §<最後の見出し>]` にする（` > ` でつないだものの最後の部分。
/// 40 字を超えたら 40 字で切って `…` を付ける）。
/// 図（`source_kind == "figure"`）は `[#37 Fig.3]` / `[#37 Table 2]` にする。
#[must_use]
pub fn cite_for(hit: &Hit) -> Option<String> {
    let id = hit.paper_id?;
    if hit.source_kind == "figure" {
        return Some(cite_figure(id, hit.section.as_deref()));
    }
    Some(match &hit.section {
        Some(section) => {
            let last = section.rsplit(" > ").next().unwrap_or(section.as_str());
            let mut short: String = last.chars().take(40).collect();
            if last.chars().count() > 40 {
                short.push('…');
            }
            format!("[#{id} §{short}]")
        }
        None => format!("[#{id}]"),
    })
}

/// 図の引用の文字列。`Figure 3` / `Fig. 3` は `[#37 Fig.3]`、`Table 2` は `[#37 Table 2]`。
/// ラベルが読めないときは `[#37]`。
fn cite_figure(id: i64, label: Option<&str>) -> String {
    let Some(label) = label else {
        return format!("[#{id}]");
    };
    let lower = label.to_ascii_lowercase();
    let number = first_number(label);
    if lower.starts_with("table") {
        if let Some(number) = number {
            return format!("[#{id} Table {number}]");
        }
    } else if lower.starts_with("fig") {
        if let Some(number) = number {
            return format!("[#{id} Fig.{number}]");
        }
    }
    format!("[#{id}]")
}

/// 文字列の先頭から最初に現れる数字の並びを返す。
fn first_number(s: &str) -> Option<String> {
    let mut number = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            number.push(ch);
        } else if !number.is_empty() {
            break;
        }
    }
    if number.is_empty() { None } else { Some(number) }
}

/// 検索結果を JSON にし、各要素に `cite` を足す（CLI の --json と MCP で共用）。
pub fn hits_to_json(hits: &[Hit]) -> serde_json::Value {
    let mut value = serde_json::to_value(hits).expect("Hit は必ず JSON にできる");
    if let serde_json::Value::Array(items) = &mut value {
        for (item, hit) in items.iter_mut().zip(hits.iter()) {
            if let serde_json::Value::Object(map) = item {
                let cite = cite_for(hit).map_or(serde_json::Value::Null, serde_json::Value::String);
                map.insert("cite".to_string(), cite);
            }
        }
    }
    value
}

/// コサイン類似度を計算する。
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a * norm_b)
    }
}

/// 複数の埋め込みベクトルを平均化する。
fn average_embeddings(embeddings: &[Vec<f32>]) -> Vec<f32> {
    if embeddings.is_empty() {
        return Vec::new();
    }

    let dim = embeddings[0].len();
    let mut result = vec![0.0; dim];

    for embedding in embeddings {
        for (i, val) in embedding.iter().enumerate() {
            if i < dim {
                result[i] += val;
            }
        }
    }

    let count = embeddings.len() as f32;
    for val in &mut result {
        *val /= count;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6);

        let c = vec![3.0, 4.0];
        let d = vec![3.0, 4.0];
        let sim = cosine_similarity(&c, &d);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_average_embeddings() {
        let embeddings = vec![
            vec![1.0, 2.0, 3.0],
            vec![2.0, 3.0, 4.0],
            vec![3.0, 4.0, 5.0],
        ];
        let avg = average_embeddings(&embeddings);
        assert_eq!(avg.len(), 3);
        assert!((avg[0] - 2.0).abs() < 1e-6);
        assert!((avg[1] - 3.0).abs() < 1e-6);
        assert!((avg[2] - 4.0).abs() < 1e-6);
    }

    #[test]
    fn test_average_embeddings_empty() {
        let embeddings: Vec<Vec<f32>> = vec![];
        let avg = average_embeddings(&embeddings);
        assert_eq!(avg.len(), 0);
    }

    /// Library の論文には `[#番号]` を、論文でない結果には None を返す。
    #[test]
    fn test_cite_for_paper_and_non_paper() {
        let mut hit = Hit {
            score: 0.9,
            chunk_id: 1,
            source_kind: "abstract".to_string(),
            source_ref: "ref1".to_string(),
            paper_id: Some(37),
            title: Some("Title".to_string()),
            authors: None,
            year: None,
            text: "text".to_string(),
            char_start: None,
            char_end: None,
            md_path: None,
            section: None,
        };
        assert_eq!(cite_for(&hit).as_deref(), Some("[#37]"));

        // ノートなど論文でない結果は cite を持たない
        hit.paper_id = None;
        assert_eq!(cite_for(&hit), None);
    }

    /// 図の引用は Figure / Table で形を変える（R05）。
    #[test]
    fn test_cite_for_figure() {
        let mut hit = Hit {
            score: 0.9,
            chunk_id: 1,
            source_kind: "figure".to_string(),
            source_ref: "figure_key1_3".to_string(),
            paper_id: Some(37),
            title: Some("Title".to_string()),
            authors: None,
            year: None,
            text: "text".to_string(),
            char_start: None,
            char_end: None,
            md_path: None,
            section: Some("Figure 3".to_string()),
        };
        assert_eq!(cite_for(&hit).as_deref(), Some("[#37 Fig.3]"));

        // `Fig. 3` のような書き方でも番号を取る。
        hit.section = Some("Fig. 3".to_string());
        assert_eq!(cite_for(&hit).as_deref(), Some("[#37 Fig.3]"));

        // 表は `Table <番号>`。
        hit.section = Some("Table 2".to_string());
        assert_eq!(cite_for(&hit).as_deref(), Some("[#37 Table 2]"));

        // ラベルが読めないときは論文だけ。
        hit.section = None;
        assert_eq!(cite_for(&hit).as_deref(), Some("[#37]"));
    }

    /// 章があれば引用に最後の見出しを入れ、40 字を超えたら切る。
    #[test]
    fn test_cite_for_uses_section() {
        let mut hit = Hit {
            score: 0.9,
            chunk_id: 1,
            source_kind: "body".to_string(),
            source_ref: "extract_1".to_string(),
            paper_id: Some(37),
            title: Some("Title".to_string()),
            authors: None,
            year: None,
            text: "text".to_string(),
            char_start: None,
            char_end: None,
            md_path: None,
            section: Some("3 Methods > 3.2 Preprocessing".to_string()),
        };
        // 章の最後の見出しだけを付ける。
        assert_eq!(cite_for(&hit).as_deref(), Some("[#37 §3.2 Preprocessing]"));

        // 章が無ければ論文だけ。
        hit.section = None;
        assert_eq!(cite_for(&hit).as_deref(), Some("[#37]"));

        // 40 字を超える見出しは 40 字で切って … を付ける。
        hit.section = Some("あ".repeat(41));
        assert_eq!(
            cite_for(&hit),
            Some(format!("[#37 §{}…]", "あ".repeat(40)))
        );
    }

    /// JSON の各要素に `cite` を足す。論文でない結果は null。
    #[test]
    fn test_hits_to_json_adds_cite() {
        let paper_hit = Hit {
            score: 0.9,
            chunk_id: 1,
            source_kind: "abstract".to_string(),
            source_ref: "ref1".to_string(),
            paper_id: Some(37),
            title: Some("Title".to_string()),
            authors: None,
            year: None,
            text: "text1".to_string(),
            char_start: None,
            char_end: None,
            md_path: None,
            section: None,
        };
        let note_hit = Hit {
            paper_id: None,
            chunk_id: 2,
            source_ref: "note1".to_string(),
            source_kind: "note".to_string(),
            text: "text2".to_string(),
            ..paper_hit.clone()
        };

        let value = hits_to_json(&[paper_hit, note_hit]);
        assert_eq!(value[0]["cite"], serde_json::json!("[#37]"));
        assert!(value[1]["cite"].is_null());
    }

    #[test]
    fn test_neighbors_excludes_source_paper() {
        // in-memory DB でテスト
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE papers (
               id INTEGER PRIMARY KEY,
               title TEXT,
               authors TEXT,
               year INTEGER,
               published_at TEXT
             );
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               paper_id INTEGER REFERENCES papers(id),
               text TEXT NOT NULL,
               source_kind TEXT,
               source_ref TEXT,
               ord INTEGER,
               char_start INTEGER,
               char_end INTEGER,
               strategy TEXT,
               created_at TEXT,
               index_name TEXT NOT NULL DEFAULT 'default',
               section TEXT
             );
             CREATE TABLE embedding_models (
               id INTEGER PRIMARY KEY,
               name TEXT NOT NULL UNIQUE,
               endpoint TEXT NOT NULL,
               dim INTEGER NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE chunk_embeddings (
               chunk_id INTEGER NOT NULL REFERENCES chunks(id),
               model_id INTEGER NOT NULL REFERENCES embedding_models(id),
               vec BLOB NOT NULL,
               PRIMARY KEY (chunk_id, model_id)
             );
             CREATE TABLE paper_extracts (
               paper_id TEXT PRIMARY KEY,
               md_path TEXT
             );",
        )
        .unwrap();

        // テストデータを挿入
        conn.execute("INSERT INTO papers (id) VALUES (1)", [])
            .unwrap();
        conn.execute("INSERT INTO papers (id) VALUES (2)", [])
            .unwrap();
        conn.execute("INSERT INTO papers (id) VALUES (3)", [])
            .unwrap();

        // モデルを登録
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO embedding_models (name, endpoint, dim, created_at) VALUES (?, ?, ?, ?)",
            rusqlite::params!["test-model", "http://test", 3, now],
        )
        .unwrap();
        let model_id: i64 = conn.last_insert_rowid();

        // チャンクを作成（paper_id=1 に属する 2 個、paper_id=2,3 に属する 2 個）
        for (i, paper_id) in vec![1, 1, 2, 3].iter().enumerate() {
            conn.execute(
                "INSERT INTO chunks (paper_id, text, source_kind, source_ref, ord, char_start, char_end, strategy, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    paper_id, format!("text{}", i), "body", "ref", i as i64, 0, 10, "sentence",
                    &now
                ],
            ).unwrap();
        }

        // 埋め込みベクトルを挿入
        let vectors = vec![
            vec![1.0, 0.0, 0.0], // chunk 1 (paper 1)
            vec![1.0, 0.0, 0.0], // chunk 2 (paper 1)
            vec![0.0, 1.0, 0.0], // chunk 3 (paper 2)
            vec![0.0, 0.0, 1.0], // chunk 4 (paper 3)
        ];
        for (chunk_id, vec_data) in vectors.iter().enumerate() {
            let blob = embeddings::to_blob(vec_data);
            conn.execute(
                "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?, ?, ?)",
                rusqlite::params![(chunk_id + 1) as i64, model_id, blob],
            )
            .unwrap();
        }

        // neighbors() を呼び出し、paper_id=1 の近傍を探す
        let hits = neighbors(&conn, 1, model_id, 10, None).unwrap();

        // paper_id=1 自体のチャンクは除外されるべき
        assert!(!hits.iter().any(|h| h.paper_id == Some(1)));
        // paper_id=2, 3 のチャンクが含まれるべき
        let paper_ids: Vec<Option<i64>> = hits.iter().map(|h| h.paper_id).collect();
        assert!(paper_ids.contains(&Some(2)) || paper_ids.contains(&Some(3)));
    }

    #[test]
    fn test_incremental_embedding_in_db() {
        // DB setup
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               paper_id INTEGER,
               text TEXT NOT NULL,
               source_kind TEXT,
               source_ref TEXT,
               ord INTEGER,
               char_start INTEGER,
               char_end INTEGER,
               strategy TEXT,
               created_at TEXT,
               section TEXT
             );
             CREATE TABLE embedding_models (
               id INTEGER PRIMARY KEY,
               name TEXT NOT NULL UNIQUE,
               endpoint TEXT NOT NULL,
               dim INTEGER NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE chunk_embeddings (
               chunk_id INTEGER NOT NULL REFERENCES chunks(id),
               model_id INTEGER NOT NULL REFERENCES embedding_models(id),
               vec BLOB NOT NULL,
               PRIMARY KEY (chunk_id, model_id)
             );",
        )
        .unwrap();

        let now = chrono::Utc::now().to_rfc3339();

        // チャンクを作成
        for i in 1..=3 {
            conn.execute(
                "INSERT INTO chunks (id, text, source_kind, source_ref, ord, char_start, char_end, strategy, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    i, format!("text{}", i), "body", "ref", i, 0, 10, "sentence", &now
                ],
            ).unwrap();
        }

        // モデル A を登録
        conn.execute(
            "INSERT INTO embedding_models (name, endpoint, dim, created_at) VALUES (?, ?, ?, ?)",
            rusqlite::params!["model-a", "http://a", 3, &now],
        )
        .unwrap();
        let model_a_id: i64 = conn.last_insert_rowid();

        // モデル A でチャンク 1, 2 を埋め込む
        for chunk_id in 1..=2 {
            let blob = embeddings::to_blob(&vec![0.1; 3]);
            conn.execute(
                "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?, ?, ?)",
                rusqlite::params![chunk_id, model_a_id, blob],
            )
            .unwrap();
        }

        // モデル B を登録
        conn.execute(
            "INSERT INTO embedding_models (name, endpoint, dim, created_at) VALUES (?, ?, ?, ?)",
            rusqlite::params!["model-b", "http://b", 3, &now],
        )
        .unwrap();
        let model_b_id: i64 = conn.last_insert_rowid();

        // モデル B でチャンク 1, 3 を埋め込む
        for chunk_id in [1, 3] {
            let blob = embeddings::to_blob(&vec![0.2; 3]);
            conn.execute(
                "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?, ?, ?)",
                rusqlite::params![chunk_id, model_b_id, blob],
            )
            .unwrap();
        }

        // 確認：モデル A は 2 個、モデル B は 2 個の埋め込みを持つ
        let count_a: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunk_embeddings WHERE model_id = ?1",
                [model_a_id],
                |r| r.get(0),
            )
            .unwrap();
        let count_b: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunk_embeddings WHERE model_id = ?1",
                [model_b_id],
                |r| r.get(0),
            )
            .unwrap();

        assert_eq!(count_a, 2); // 1, 2
        assert_eq!(count_b, 2); // 1, 3

        // モデルが別々に管理されていることを確認（インクリメンタル埋め込みの基盤）
        let model_a_chunks: Vec<i64> = conn
            .prepare("SELECT chunk_id FROM chunk_embeddings WHERE model_id = ?1 ORDER BY chunk_id")
            .unwrap()
            .query_map([model_a_id], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        let model_b_chunks: Vec<i64> = conn
            .prepare("SELECT chunk_id FROM chunk_embeddings WHERE model_id = ?1 ORDER BY chunk_id")
            .unwrap()
            .query_map([model_b_id], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(model_a_chunks, vec![1, 2]);
        assert_eq!(model_b_chunks, vec![1, 3]);
    }

    #[test]
    fn test_word_search_basic() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE papers (
               id INTEGER PRIMARY KEY,
               title TEXT,
               published_at TEXT
             );
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               source_kind TEXT NOT NULL,
               source_ref TEXT NOT NULL,
               paper_id INTEGER REFERENCES papers(id),
               ord INTEGER NOT NULL,
               text TEXT NOT NULL,
               char_start INTEGER,
               char_end INTEGER,
               strategy TEXT NOT NULL,
               created_at TEXT NOT NULL,
               section TEXT
             );
             CREATE VIRTUAL TABLE chunk_fts USING fts5(
               text,
               content='chunks',
               content_rowid='id',
               tokenize='unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
               INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
             END;
             CREATE TABLE paper_extracts (
               paper_id TEXT PRIMARY KEY,
               md_path TEXT
             );",
        )
        .unwrap();

        // テストデータを挿入
        conn.execute("INSERT INTO papers (id, title, published_at) VALUES (1, 'Paper A', '2024-01-15T00:00:00+00:00')", []).unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["abstract", "paper_1", 1, 0, "ferroptosis is a type of cell death", 0, 35, "abstract", &now],
        ).unwrap();

        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["body", "paper_1_1", 1, 1, "ferroptosis mechanism involves iron metabolism", 0, 44, "sentence", &now],
        ).unwrap();

        // 全文検索を実行
        let opts = WordSearchOptions {
            top_k: 10,
            scope: None,
            paper_id: None,
            fts_expr: false,
            index: None,
        };
        let hits = word_search(&conn, "ferroptosis", opts).unwrap();

        // 2 件がヒットするはず
        assert_eq!(hits.len(), 2);
        // スコアは 0-1 の範囲内
        for hit in &hits {
            assert!(hit.score >= 0.0 && hit.score <= 1.0);
        }
    }

    #[test]
    fn test_word_search_with_fts_operator_chars() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE papers (
               id INTEGER PRIMARY KEY,
               title TEXT,
               published_at TEXT
             );
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               source_kind TEXT NOT NULL,
               source_ref TEXT NOT NULL,
               paper_id INTEGER REFERENCES papers(id),
               ord INTEGER NOT NULL,
               text TEXT NOT NULL,
               char_start INTEGER,
               char_end INTEGER,
               strategy TEXT NOT NULL,
               created_at TEXT NOT NULL,
               section TEXT
             );
             CREATE VIRTUAL TABLE chunk_fts USING fts5(
               text,
               content='chunks',
               content_rowid='id',
               tokenize='unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
               INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
             END;
             CREATE TABLE paper_extracts (
               paper_id TEXT PRIMARY KEY,
               md_path TEXT
             );",
        )
        .unwrap();

        conn.execute("INSERT INTO papers (id, title, published_at) VALUES (1, 'Paper A', '2024-01-15T00:00:00+00:00')", []).unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["abstract", "paper_1", 1, 0, "The OR (Operational Risk) framework is important", 0, 50, "abstract", &now],
        ).unwrap();

        // 自然言語モード：FTS5 operator characters が含まれていても動作する
        let opts = WordSearchOptions {
            top_k: 10,
            scope: None,
            paper_id: None,
            fts_expr: false, // 自然言語モード
            index: None,
        };
        // クエリがクォートされるので、エラーなく実行される
        word_search(&conn, "The OR framework", opts).unwrap();
    }

    #[test]
    fn test_word_search_fts_expr_mode() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE papers (
               id INTEGER PRIMARY KEY,
               title TEXT,
               published_at TEXT
             );
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               source_kind TEXT NOT NULL,
               source_ref TEXT NOT NULL,
               paper_id INTEGER REFERENCES papers(id),
               ord INTEGER NOT NULL,
               text TEXT NOT NULL,
               char_start INTEGER,
               char_end INTEGER,
               strategy TEXT NOT NULL,
               created_at TEXT NOT NULL,
               section TEXT
             );
             CREATE VIRTUAL TABLE chunk_fts USING fts5(
               text,
               content='chunks',
               content_rowid='id',
               tokenize='unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
               INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
             END;
             CREATE TABLE paper_extracts (
               paper_id TEXT PRIMARY KEY,
               md_path TEXT
             );",
        )
        .unwrap();

        conn.execute("INSERT INTO papers (id, title, published_at) VALUES (1, 'Paper A', '2024-01-15T00:00:00+00:00')", []).unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["abstract", "paper_1", 1, 0, "ferroptosis or necroptosis", 0, 25, "abstract", &now],
        ).unwrap();

        // FTS5 式モード：OR は OR 演算子として処理される
        let opts = WordSearchOptions {
            top_k: 10,
            scope: None,
            paper_id: None,
            fts_expr: true, // FTS5 式として渡す
            index: None,
        };
        let hits = word_search(&conn, "ferroptosis OR necroptosis", opts).unwrap();
        // 1 件がヒット
        assert!(hits.len() >= 1);
    }

    #[test]
    fn test_word_search_scope_filtering() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE papers (
               id INTEGER PRIMARY KEY,
               title TEXT,
               published_at TEXT
             );
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               source_kind TEXT NOT NULL,
               source_ref TEXT NOT NULL,
               paper_id INTEGER REFERENCES papers(id),
               ord INTEGER NOT NULL,
               text TEXT NOT NULL,
               char_start INTEGER,
               char_end INTEGER,
               strategy TEXT NOT NULL,
               created_at TEXT NOT NULL,
               section TEXT
             );
             CREATE VIRTUAL TABLE chunk_fts USING fts5(
               text,
               content='chunks',
               content_rowid='id',
               tokenize='unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
               INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
             END;
             CREATE TABLE paper_extracts (
               paper_id TEXT PRIMARY KEY,
               md_path TEXT
             );",
        )
        .unwrap();

        conn.execute("INSERT INTO papers (id, title, published_at) VALUES (1, 'Paper A', '2024-01-15T00:00:00+00:00')", []).unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["abstract", "paper_1", 1, 0, "test keyword", 0, 12, "abstract", &now],
        ).unwrap();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["note", "notes/1.md", 1, 0, "test keyword in note", 0, 20, "abstract", &now],
        ).unwrap();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["figure", "figure_key1_1", 1, 1, "test keyword in figure caption", 0, 30, "paragraph", &now],
        ).unwrap();

        // スコープ = papers
        let opts = WordSearchOptions {
            top_k: 10,
            scope: Some("papers".to_string()),
            paper_id: None,
            fts_expr: false,
            index: None,
        };
        let hits = word_search(&conn, "test", opts).unwrap();
        // abstract のみヒット
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_kind, "abstract");

        // スコープ = notes
        let opts = WordSearchOptions {
            top_k: 10,
            scope: Some("notes".to_string()),
            paper_id: None,
            fts_expr: false,
            index: None,
        };
        let hits = word_search(&conn, "test", opts).unwrap();
        // note のみヒット
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_kind, "note");

        // スコープ = figures: 図のみヒット
        let opts = WordSearchOptions {
            top_k: 10,
            scope: Some("figures".to_string()),
            paper_id: None,
            fts_expr: false,
            index: None,
        };
        let hits = word_search(&conn, "test", opts).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_kind, "figure");
    }

    #[test]
    fn test_word_search_paper_id_filtering() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE papers (
               id INTEGER PRIMARY KEY,
               title TEXT,
               published_at TEXT
             );
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               source_kind TEXT NOT NULL,
               source_ref TEXT NOT NULL,
               paper_id INTEGER REFERENCES papers(id),
               ord INTEGER NOT NULL,
               text TEXT NOT NULL,
               char_start INTEGER,
               char_end INTEGER,
               strategy TEXT NOT NULL,
               created_at TEXT NOT NULL,
               section TEXT
             );
             CREATE VIRTUAL TABLE chunk_fts USING fts5(
               text,
               content='chunks',
               content_rowid='id',
               tokenize='unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
               INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
             END;
             CREATE TABLE paper_extracts (
               paper_id TEXT PRIMARY KEY,
               md_path TEXT
             );",
        )
        .unwrap();

        conn.execute("INSERT INTO papers (id, title, published_at) VALUES (1, 'Paper A', '2024-01-15T00:00:00+00:00')", []).unwrap();
        conn.execute("INSERT INTO papers (id, title, published_at) VALUES (2, 'Paper B', '2024-01-15T00:00:00+00:00')", []).unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["abstract", "paper_1", 1, 0, "test keyword", 0, 12, "abstract", &now],
        ).unwrap();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, char_start, char_end, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["abstract", "paper_2", 2, 0, "test keyword", 0, 12, "abstract", &now],
        ).unwrap();

        // paper_id = 1 でフィルタ
        let opts = WordSearchOptions {
            top_k: 10,
            scope: None,
            paper_id: Some(1),
            fts_expr: false,
            index: None,
        };
        let hits = word_search(&conn, "test", opts).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].paper_id, Some(1));
    }

    #[test]
    fn test_fuse_rrf() {
        let hit1 = Hit {
            score: 0.9,
            chunk_id: 1,
            source_kind: "abstract".to_string(),
            source_ref: "ref1".to_string(),
            paper_id: Some(1),
            title: Some("Title".to_string()),
            authors: None,
            year: None,
            text: "text1".to_string(),
            char_start: None,
            char_end: None,
            md_path: None,
            section: None,
        };
        let hit2 = Hit {
            score: 0.8,
            chunk_id: 2,
            source_kind: "abstract".to_string(),
            source_ref: "ref2".to_string(),
            paper_id: Some(1),
            title: Some("Title".to_string()),
            authors: None,
            year: None,
            text: "text2".to_string(),
            char_start: None,
            char_end: None,
            md_path: None,
            section: None,
        };

        // word: [1, 2], vector: [2, 1]
        let word_hits = vec![hit1.clone(), hit2.clone()];
        let vector_hits = vec![hit2.clone(), hit1.clone()];

        let fused = fuse_rrf(&word_hits, &vector_hits);

        // 両方にランクされているので、両方が組み合わさっている
        // スコアは合計なので、hit1 と hit2 の両方が含まれる
        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn test_fuse_weighted() {
        let hit1 = Hit {
            score: 0.9,
            chunk_id: 1,
            source_kind: "abstract".to_string(),
            source_ref: "ref1".to_string(),
            paper_id: Some(1),
            title: Some("Title".to_string()),
            authors: None,
            year: None,
            text: "text1".to_string(),
            char_start: None,
            char_end: None,
            md_path: None,
            section: None,
        };
        let hit2 = Hit {
            score: 0.5,
            chunk_id: 2,
            source_kind: "abstract".to_string(),
            source_ref: "ref2".to_string(),
            paper_id: Some(1),
            title: Some("Title".to_string()),
            authors: None,
            year: None,
            text: "text2".to_string(),
            char_start: None,
            char_end: None,
            md_path: None,
            section: None,
        };

        let word_hits = vec![hit1.clone()];
        let vector_hits = vec![hit2.clone()];

        let fused = fuse_weighted(&word_hits, &vector_hits, 0.7);

        // 異なるチャンクなので両方含まれる
        assert_eq!(fused.len(), 2);
        // スコアは 0-1 の範囲内
        for hit in &fused {
            assert!(hit.score >= 0.0 && hit.score <= 1.0);
        }
    }

    /// ベクトルの近い順に並び、索引で絞れる（1 回の問い合わせで読む形でも順位は変わらない）。
    #[test]
    fn vector_ranking_is_stable_and_filtered_by_index() {
        let db = crate::Database::in_memory().unwrap();
        let conn = db.connection();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'P')", []).unwrap();
        conn.execute(
            "INSERT INTO embedding_models (name, endpoint, dim, created_at) VALUES ('m', 'x', 2, 'now')",
            [],
        )
        .unwrap();
        let model_id = conn.last_insert_rowid();
        // (本文, 索引, ベクトル)
        let rows = [
            ("far", "a", [0.0f32, 1.0]),
            ("near", "a", [1.0, 0.1]),
            ("mid", "a", [1.0, 1.0]),
            ("other-index", "b", [1.0, 0.0]),
        ];
        for (text, index, vec) in rows {
            conn.execute(
                "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, strategy, created_at, index_name)
                 VALUES ('body', 'r', 1, 0, ?1, 'paragraph', 'now', ?2)",
                rusqlite::params![text, index],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?1, ?2, ?3)",
                rusqlite::params![conn.last_insert_rowid(), model_id, embeddings::to_blob(&vec)],
            )
            .unwrap();
        }
        let query = [1.0f32, 0.0];
        let texts = |index: Option<&str>| -> Vec<String> {
            let ranked = rank_by_vector(conn, &query, model_id, &index_where(index), 10).unwrap();
            load_hits(conn, &ranked).unwrap().into_iter().map(|h| h.text).collect()
        };
        assert_eq!(texts(Some("a")), ["near", "mid", "far"]);
        assert_eq!(texts(None), ["other-index", "near", "mid", "far"]);
        assert_eq!(texts(Some("b")), ["other-index"]);
    }

    /// 目安の計測: 約 3 万断片（1024 次元）のベクトル検索が 1 秒以内。
    /// 時間の計測なので普段は走らせない（`cargo test --release -p sai-paper-core -- --ignored vector_search_speed`）。
    #[test]
    #[ignore = "計測用"]
    fn vector_search_speed_30k_chunks() {
        let db = crate::Database::in_memory().unwrap();
        let conn = db.connection();
        conn.execute(
            "INSERT INTO embedding_models (name, endpoint, dim, created_at) VALUES ('m', 'x', 1024, 'now')",
            [],
        )
        .unwrap();
        let model_id = conn.last_insert_rowid();
        conn.execute_batch("BEGIN").unwrap();
        for n in 0..30_000i64 {
            conn.execute(
                "INSERT INTO chunks (source_kind, source_ref, ord, text, strategy, created_at)
                 VALUES ('body', 'r', 0, 'text', 'paragraph', 'now')",
                [],
            )
            .unwrap();
            let vec: Vec<f32> = (0..1024).map(|d| ((n * 31 + d) % 97) as f32).collect();
            conn.execute(
                "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?1, ?2, ?3)",
                rusqlite::params![conn.last_insert_rowid(), model_id, embeddings::to_blob(&vec)],
            )
            .unwrap();
        }
        conn.execute_batch("COMMIT").unwrap();
        let query: Vec<f32> = (0..1024).map(|d| d as f32).collect();
        let started = std::time::Instant::now();
        let ranked = rank_by_vector(conn, &query, model_id, "", 20).unwrap();
        let hits = load_hits(conn, &ranked).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(hits.len(), 20);
        assert!(elapsed.as_secs_f64() < 1.0, "遅い: {elapsed:?}");
    }

    /// `limit_per_paper` のテスト用に検索結果を 1 件作る。
    fn make_hit(paper_id: Option<i64>, chunk_id: i64) -> Hit {
        Hit {
            score: 0.5,
            chunk_id,
            source_kind: "body".to_string(),
            source_ref: format!("ref_{chunk_id}"),
            paper_id,
            title: None,
            authors: None,
            year: None,
            text: format!("needle {chunk_id}"),
            char_start: None,
            char_end: None,
            md_path: None,
            section: None,
        }
    }

    /// 同じ論文の塊が上位を占めても、1 本から `per_paper` 件までに抑える。
    #[test]
    fn limit_per_paper_caps_same_paper() {
        // 論文 1 が 5 件、論文 2 が 2 件、論文 3 が 1 件。
        let hits = vec![
            make_hit(Some(1), 1),
            make_hit(Some(1), 2),
            make_hit(Some(1), 3),
            make_hit(Some(1), 4),
            make_hit(Some(1), 5),
            make_hit(Some(2), 6),
            make_hit(Some(2), 7),
            make_hit(Some(3), 8),
        ];
        let out = limit_per_paper(hits, 2, 5);
        let chunks: Vec<i64> = out.iter().map(|h| h.chunk_id).collect();
        // 論文 1 の 1・2、論文 2 の 6・7、論文 3 の 8 が残る。
        assert_eq!(chunks, vec![1, 2, 6, 7, 8]);
    }

    /// `paper_id` の無い結果（ノートなど）は制限しない。
    #[test]
    fn limit_per_paper_ignores_hits_without_paper() {
        let hits = vec![
            make_hit(None, 1),
            make_hit(None, 2),
            make_hit(Some(1), 3),
            make_hit(Some(1), 4),
            make_hit(Some(1), 5),
        ];
        let out = limit_per_paper(hits, 1, 10);
        let chunks: Vec<i64> = out.iter().map(|h| h.chunk_id).collect();
        // `None` は 2 件とも残り、論文 1 は 1 件だけ。
        assert_eq!(chunks, vec![1, 2, 3]);
    }

    /// `top_k` 件そろったらそこで止める。
    #[test]
    fn limit_per_paper_stops_at_top_k() {
        let hits: Vec<Hit> = (1..=10).map(|i| make_hit(Some(i), i)).collect();
        let out = limit_per_paper(hits, 2, 3);
        let chunks: Vec<i64> = out.iter().map(|h| h.chunk_id).collect();
        assert_eq!(chunks, vec![1, 2, 3]);
    }

    /// `per_paper = 0` は制限なし。
    #[test]
    fn limit_per_paper_zero_means_no_limit() {
        let hits = vec![
            make_hit(Some(1), 1),
            make_hit(Some(1), 2),
            make_hit(Some(1), 3),
        ];
        let out = limit_per_paper(hits, 0, 10);
        assert_eq!(out.len(), 3);
    }

    /// `find` は既定（`per_paper = 2`）で 1 本の論文から 2 件までに抑える。
    #[tokio::test]
    async fn find_limits_per_paper_by_default() {
        let db = crate::Database::in_memory().unwrap();
        let conn = db.connection();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'P')", [])
            .unwrap();
        conn.execute("INSERT INTO papers (id, title) VALUES (2, 'Q')", [])
            .unwrap();
        // 論文 1 の断片を 5 件、論文 2 を 1 件。
        for (paper_id, ord) in (0..5).map(|ord| (1, ord)).chain(std::iter::once((2, 0))) {
            conn.execute(
                "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, strategy, created_at)
                 VALUES ('body', 'r', ?1, ?2, 'needle', 'paragraph', 'now')",
                rusqlite::params![paper_id, ord],
            )
            .unwrap();
        }
        let config = crate::Config::default();
        let hits = find(
            conn,
            &config,
            FindRequest {
                query: "needle".to_string(),
                mode: Some("word".to_string()),
                top_k: Some(4),
                ..FindRequest::default()
            },
        )
        .await
        .unwrap();
        // 論文 1 から 2 件・論文 2 から 1 件の 3 件で止まる（4 件目は無い）。
        assert_eq!(hits.len(), 3);
        assert_eq!(hits.iter().filter(|h| h.paper_id == Some(1)).count(), 2);
        assert_eq!(hits.iter().filter(|h| h.paper_id == Some(2)).count(), 1);
    }

    /// `paper_id` で 1 本に絞った検索では制限しない。
    #[tokio::test]
    async fn find_does_not_limit_when_paper_id_is_set() {
        let db = crate::Database::in_memory().unwrap();
        let conn = db.connection();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'P')", [])
            .unwrap();
        for ord in 0..5 {
            conn.execute(
                "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, strategy, created_at)
                 VALUES ('body', 'r', 1, ?1, 'needle', 'paragraph', 'now')",
                rusqlite::params![ord],
            )
            .unwrap();
        }
        let config = crate::Config::default();
        let hits = find(
            conn,
            &config,
            FindRequest {
                query: "needle".to_string(),
                mode: Some("word".to_string()),
                top_k: Some(10),
                paper_id: Some(1),
                per_paper: Some(1),
                ..FindRequest::default()
            },
        )
        .await
        .unwrap();
        // `per_paper = 1` でも 1 本に絞った検索では 5 件そのまま。
        assert_eq!(hits.len(), 5);
    }

    /// `per_paper = 0` では `find` が件数を制限しない。
    #[tokio::test]
    async fn find_does_not_limit_when_per_paper_is_zero() {
        let db = crate::Database::in_memory().unwrap();
        let conn = db.connection();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'P')", [])
            .unwrap();
        for ord in 0..5 {
            conn.execute(
                "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text, strategy, created_at)
                 VALUES ('body', 'r', 1, ?1, 'needle', 'paragraph', 'now')",
                rusqlite::params![ord],
            )
            .unwrap();
        }
        let config = crate::Config::default();
        let hits = find(
            conn,
            &config,
            FindRequest {
                query: "needle".to_string(),
                mode: Some("word".to_string()),
                top_k: Some(10),
                per_paper: Some(0),
                ..FindRequest::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(hits.len(), 5);
    }
}

//! セッション jsonl の usage 集計ロジック（トークン / コスト）。
//!
//! 実装計画 01 §2.1 により、当初 `agent-link-view/src/cache_stats.rs` にあった
//! **集計ロジックと型だけ** を engine へ移設したものである。
//! 描画部分は TUI の削除に合わせて外し、agent-link-view は
//! `pub use agent_link_engine::usage_metrics::*;` で再公開する。
//!
//! **計算式は移設前と完全に同一**（TUI の Stats 画面と数字を一致させるため）。
//!
//! - pi セッション jsonl: [`collect_metrics_from_file`]
//! - cc（Claude Code）セッション jsonl: [`collect_metrics_from_file_cc`]
//! - cc のコスト概算: [`estimate_cc_cost`]（モデル名 × 公式単価）

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;

// ============================================================================
// 集計ロジック（sai-cache-tui-rs / cache_math.rs の移植）
// ============================================================================

/// コスト内訳（USD）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CostBreakdown {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

/// トークン使用量の合計。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CacheUsageTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    pub assistant_messages: usize,
    pub cost: CostBreakdown,
}

/// キャッシュヒット率（正規定義）。
///
/// `cacheRead / (input + cacheRead + cacheWrite) * 100`。
/// Anthropic は input に cacheWrite を含まず、OpenAI は cacheWrite == 0 のため
/// この式が両プロバイダで互換になる。
pub fn compute_cache_hit_percent(input: u64, cache_read: u64, cache_write: u64) -> f64 {
    let denom = input as f64 + cache_read as f64 + cache_write as f64;
    if denom <= 0.0 {
        0.0
    } else {
        cache_read as f64 / denom * 100.0
    }
}

/// 合計値からヒット率を要約する。
pub fn summarize_hit_percent(t: &CacheUsageTotals) -> f64 {
    compute_cache_hit_percent(t.input, t.cache_read, t.cache_write)
}

/// 1 メッセージ分の使用量を合計に加算する。
fn add_to_totals(totals: &mut CacheUsageTotals, m: &AssistantUsageMetric) {
    totals.input += m.input;
    totals.output += m.output;
    totals.cache_read += m.cache_read;
    totals.cache_write += m.cache_write;
    totals.total_tokens += m.total_tokens;
    totals.assistant_messages += 1;
    totals.cost.input += m.cost.input;
    totals.cost.output += m.cost.output;
    totals.cost.cache_read += m.cost.cache_read;
    totals.cost.cache_write += m.cost.cache_write;
    totals.cost.total += m.cost.total;
}

/// assistant メッセージ 1 件分のトークン使用量メトリクス。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AssistantUsageMetric {
    /// ツリー全体での通し番号（1 始まり）
    pub sequence: usize,
    /// アクティブブランチ内での通し番号（アクティブブランチ上のみ Some）
    pub active_branch_sequence: Option<usize>,
    /// jsonl エントリ ID
    pub entry_id: String,
    /// タイムスタンプ
    pub timestamp: String,
    /// プロバイダ
    pub provider: String,
    /// モデル
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    /// キャッシュヒット率（%）
    pub cache_hit_percent: f64,
    /// アクティブブランチ（最終エントリから parentId を遡った経路）上か
    pub is_on_active_branch: bool,
    /// コスト内訳
    pub cost: CostBreakdown,
}

/// セッション 1 件分の集計結果。
#[derive(Debug, Clone, Default)]
pub struct CacheSessionMetrics {
    /// 全 assistant メッセージ（ツリー全体）
    pub all_messages: Vec<AssistantUsageMetric>,
    /// アクティブブランチ上の assistant メッセージ
    pub active_branch_messages: Vec<AssistantUsageMetric>,
    /// ツリー全体の合計
    pub tree_totals: CacheUsageTotals,
    /// アクティブブランチの合計
    pub active_branch_totals: CacheUsageTotals,
}

/// 累積系列（チャート用）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CumulativeSeries {
    pub cum_input: Vec<f64>,
    pub cum_cache_read: Vec<f64>,
    pub cum_cache_write: Vec<f64>,
    pub cum_hit_percent: Vec<f64>,
}

/// メッセージ列から累積系列を計算する。
pub fn compute_cumulative_series(messages: &[AssistantUsageMetric]) -> CumulativeSeries {
    let mut s = CumulativeSeries::default();
    let (mut sum_in, mut sum_read, mut sum_write) = (0u64, 0u64, 0u64);
    for m in messages {
        sum_in += m.input;
        sum_read += m.cache_read;
        sum_write += m.cache_write;
        s.cum_input.push(sum_in as f64);
        s.cum_cache_read.push(sum_read as f64);
        s.cum_cache_write.push(sum_write as f64);
        s.cum_hit_percent
            .push(compute_cache_hit_percent(sum_in, sum_read, sum_write));
    }
    s
}

// ============================================================================
// jsonl パース（sai-cache-tui-rs / session.rs の移植）
// ============================================================================

/// セッション内容のメッセージ数カウント。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SessionCounts {
    pub user: usize,
    pub assistant: usize,
    pub thinking: usize,
    pub toolcall: usize,
}

/// pi セッション jsonl の最小限の serde モデル。
#[derive(Debug, serde::Deserialize)]
struct RawEntry {
    #[serde(rename = "type")]
    type_: String,
    id: Option<String>,
    #[serde(rename = "parentId")]
    parent_id: Option<String>,
    timestamp: Option<String>,
    message: Option<RawMessage>,
}

#[derive(Debug, serde::Deserialize)]
struct RawMessage {
    role: String,
    provider: Option<String>,
    model: Option<String>,
    usage: Option<RawUsage>,
    content: Option<Vec<RawContent>>,
}

#[derive(Debug, serde::Deserialize)]
struct RawContent {
    #[serde(rename = "type")]
    type_: String,
}

#[derive(Debug, serde::Deserialize)]
struct RawUsage {
    input: Option<u64>,
    output: Option<u64>,
    #[serde(rename = "cacheRead")]
    cache_read: Option<u64>,
    #[serde(rename = "cacheWrite")]
    cache_write: Option<u64>,
    #[serde(rename = "totalTokens")]
    total_tokens: Option<u64>,
    cost: Option<RawCost>,
}

#[derive(Debug, serde::Deserialize)]
struct RawCost {
    input: Option<f64>,
    output: Option<f64>,
    #[serde(rename = "cacheRead")]
    cache_read: Option<f64>,
    #[serde(rename = "cacheWrite")]
    cache_write: Option<f64>,
    total: Option<f64>,
}

/// Claude Code（cc）セッション jsonl の usage モデル（snake_case）。
#[derive(Debug, serde::Deserialize)]
struct RawUsageCc {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    /// TTL 別内訳（5分 / 1時間）。書き込み単価が TTL で異なる（1.25x / 2x）ため必要。
    cache_creation: Option<RawCacheCreationCc>,
}

#[derive(Debug, serde::Deserialize)]
struct RawCacheCreationCc {
    ephemeral_5m_input_tokens: Option<u64>,
    ephemeral_1h_input_tokens: Option<u64>,
}

/// Claude Code（cc）セッション jsonl の最小限の serde モデル。
#[derive(Debug, serde::Deserialize)]
struct RawEntryCc {
    #[serde(rename = "type")]
    type_: String,
    timestamp: Option<String>,
    message: Option<RawMessageCc>,
}

#[derive(Debug, serde::Deserialize)]
struct RawMessageCc {
    role: String,
    model: Option<String>,
    usage: Option<RawUsageCc>,
    // 集計では使わないが、cc の jsonl 形式を正確に写すために保持する
    #[allow(dead_code)]
    content: Option<Vec<RawContent>>,
}

/// cc の usage には金額が含まれないため、モデル名から Anthropic 公式レートで概算する。
/// 単価は $/1M トークン（2026-06 時点、Claude API 公式レート）。
fn claude_pricing_usd_per_mtok(model: &str) -> (f64, f64) {
    let m = model.to_ascii_lowercase();
    if m.contains("fable-5-1") || m.contains("mythos-5-1") || m.contains("fable-5") {
        (10.00, 50.00)
    } else if m.contains("opus") {
        (5.00, 25.00)
    } else if m.contains("sonnet-5") {
        (2.00, 10.00)
    } else if m.contains("sonnet") {
        (3.00, 15.00)
    } else if m.contains("haiku") {
        (1.00, 5.00)
    } else {
        // 未知のモデル名は単価不明のため 0（cost 欄は 0 のまま表示される）。
        (0.0, 0.0)
    }
}

/// キャッシュ読み取り単価は入力単価の 0.1 倍（Fable 5.1 / Mythos 5.1 のみ 0.025 倍）。
fn cache_read_multiplier(model: &str) -> f64 {
    let m = model.to_ascii_lowercase();
    if m.contains("fable-5-1") || m.contains("mythos-5-1") {
        0.025
    } else {
        0.1
    }
}

/// トークン数とモデル名からコスト内訳を概算する。
/// キャッシュ書き込みは TTL 別（5分=1.25x / 1時間=2x）に計算する。
fn estimate_cc_cost(
    model: &str,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_creation: Option<&RawCacheCreationCc>,
    cache_write_total: u64,
) -> CostBreakdown {
    let (price_in, price_out) = claude_pricing_usd_per_mtok(model);
    if price_in == 0.0 && price_out == 0.0 {
        return CostBreakdown::default();
    }
    let (cw_5m, cw_1h) = match cache_creation {
        Some(cc) => (
            cc.ephemeral_5m_input_tokens.unwrap_or(0),
            cc.ephemeral_1h_input_tokens.unwrap_or(0),
        ),
        // 内訳が無ければ既定 TTL（5分）とみなす。
        None => (cache_write_total, 0),
    };
    let cost_input = input as f64 * price_in / 1_000_000.0;
    let cost_output = output as f64 * price_out / 1_000_000.0;
    let cost_cache_read = cache_read as f64 * price_in * cache_read_multiplier(model) / 1_000_000.0;
    let cost_cache_write = (cw_5m as f64 * price_in * 1.25 + cw_1h as f64 * price_in * 2.0) / 1_000_000.0;
    CostBreakdown {
        input: cost_input,
        output: cost_output,
        cache_read: cost_cache_read,
        cache_write: cost_cache_write,
        total: cost_input + cost_output + cost_cache_read + cost_cache_write,
    }
}

/// jsonl ファイルをエントリ列として読み込む（順序保持・不正行はスキップ）。
fn load_entries(path: &Path) -> std::io::Result<Vec<RawEntry>> {
    let f = std::fs::File::open(path)?;
    let r = BufReader::new(f);
    let mut out = Vec::new();
    for line in r.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(e) = serde_json::from_str::<RawEntry>(&line) {
            out.push(e);
        }
    }
    Ok(out)
}

/// アクティブブランチを再構築する: 最終エントリから `parentId` を逆順に辿る。
fn active_branch_ids(entries: &[RawEntry]) -> HashSet<String> {
    let id_map: HashMap<String, &RawEntry> = entries
        .iter()
        .filter_map(|e| e.id.clone().map(|id| (id, e)))
        .collect();
    let mut ids = HashSet::new();
    let mut cur = entries.iter().rev().find_map(|e| e.id.clone());
    while let Some(id) = cur {
        ids.insert(id.clone());
        cur = id_map.get(&id).and_then(|e| e.parent_id.clone());
    }
    ids
}

/// セッション jsonl からメッセージ数カウントを収集する。
pub fn collect_input_counts(path: &Path) -> std::io::Result<SessionCounts> {
    let entries = load_entries(path)?;
    let mut c = SessionCounts::default();
    for e in &entries {
        if e.type_ != "message" {
            continue;
        }
        let Some(msg) = &e.message else { continue };
        match msg.role.as_str() {
            "user" => c.user += 1,
            "assistant" => {
                c.assistant += 1;
                if let Some(content) = &msg.content {
                    for block in content {
                        match block.type_.as_str() {
                            "thinking" => c.thinking += 1,
                            "toolCall" => c.toolcall += 1,
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(c)
}

/// セッション jsonl ファイルからキャッシュ使用量メトリクスを収集する。
pub fn collect_metrics_from_file(path: &Path) -> std::io::Result<CacheSessionMetrics> {
    let entries = load_entries(path)?;
    let branch_ids = active_branch_ids(&entries);

    let mut all = Vec::new();
    let mut active = Vec::new();
    let mut tree_totals = CacheUsageTotals::default();
    let mut branch_totals = CacheUsageTotals::default();
    let (mut seq, mut branch_seq) = (0usize, 0usize);

    for e in &entries {
        if e.type_ != "message" {
            continue;
        }
        let Some(msg) = &e.message else { continue };
        if msg.role != "assistant" {
            continue;
        }
        let Some(usage) = &msg.usage else { continue };

        seq += 1;
        let input = usage.input.unwrap_or(0);
        let output = usage.output.unwrap_or(0);
        let cr = usage.cache_read.unwrap_or(0);
        let cw = usage.cache_write.unwrap_or(0);
        let total = usage.total_tokens.unwrap_or(input + output + cr + cw);
        let c = usage.cost.as_ref();
        let cost = CostBreakdown {
            input: c.and_then(|x| x.input).unwrap_or(0.0),
            output: c.and_then(|x| x.output).unwrap_or(0.0),
            cache_read: c.and_then(|x| x.cache_read).unwrap_or(0.0),
            cache_write: c.and_then(|x| x.cache_write).unwrap_or(0.0),
            total: c.and_then(|x| x.total).unwrap_or(0.0),
        };
        let hit = compute_cache_hit_percent(input, cr, cw);
        let id = e.id.clone().unwrap_or_default();
        let is_branch = branch_ids.contains(&id);
        let mut m = AssistantUsageMetric {
            sequence: seq,
            active_branch_sequence: None,
            entry_id: id,
            timestamp: e.timestamp.clone().unwrap_or_default(),
            provider: msg.provider.clone().unwrap_or_default(),
            model: msg.model.clone().unwrap_or_default(),
            input,
            output,
            cache_read: cr,
            cache_write: cw,
            total_tokens: total,
            cache_hit_percent: hit,
            is_on_active_branch: is_branch,
            cost,
        };
        add_to_totals(&mut tree_totals, &m);
        all.push(m.clone());
        if is_branch {
            branch_seq += 1;
            m.active_branch_sequence = Some(branch_seq);
            add_to_totals(&mut branch_totals, &m);
            if let Some(last) = all.last_mut() {
                last.active_branch_sequence = Some(branch_seq);
            }
            active.push(m);
        }
    }

    Ok(CacheSessionMetrics {
        all_messages: all,
        active_branch_messages: active,
        tree_totals,
        active_branch_totals: branch_totals,
    })
}

/// Claude Code（cc）セッション jsonl ファイルからキャッシュ使用量メトリクスを収集する。
///
/// cc の usage フィールドは snake_case: input_tokens, output_tokens,
/// cache_read_input_tokens, cache_creation_input_tokens
pub fn collect_metrics_from_file_cc(path: &Path) -> std::io::Result<CacheSessionMetrics> {
    let f = std::fs::File::open(path)?;
    let r = BufReader::new(f);
    let mut entries = Vec::new();
    for line in r.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(e) = serde_json::from_str::<RawEntryCc>(&line) {
            entries.push(e);
        }
    }

    let mut all = Vec::new();
    let mut seq = 0usize;
    let mut tree_totals = CacheUsageTotals::default();

    for e in &entries {
        if e.type_ != "user" && e.type_ != "assistant" {
            continue;
        }
        let Some(msg) = &e.message else { continue };
        if msg.role != "assistant" {
            continue;
        }
        let Some(usage) = &msg.usage else { continue };

        seq += 1;
        let input = usage.input_tokens.unwrap_or(0);
        let output = usage.output_tokens.unwrap_or(0);
        let cr = usage.cache_read_input_tokens.unwrap_or(0);
        let cw = usage.cache_creation_input_tokens.unwrap_or(0);
        let total = input + output + cr + cw;
        let model = msg.model.clone().unwrap_or_default();

        // cc の jsonl には金額が入っていないため、モデル名 + 公式レートから概算する。
        let cost = estimate_cc_cost(&model, input, output, cr, usage.cache_creation.as_ref(), cw);
        let hit = compute_cache_hit_percent(input, cr, cw);

        let m = AssistantUsageMetric {
            sequence: seq,
            active_branch_sequence: None,
            entry_id: String::new(),
            timestamp: e.timestamp.clone().unwrap_or_default(),
            provider: "claude".to_string(),
            model,
            input,
            output,
            cache_read: cr,
            cache_write: cw,
            total_tokens: total,
            cache_hit_percent: hit,
            is_on_active_branch: true,
            cost,
        };
        add_to_totals(&mut tree_totals, &m);
        all.push(m);
    }

    Ok(CacheSessionMetrics {
        all_messages: all,
        active_branch_messages: Vec::new(),
        tree_totals,
        active_branch_totals: CacheUsageTotals::default(),
    })
}

// ============================================================================
// テスト（cache_stats.rs から移設。計算式が変わっていないことを確認する）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// ヒット率の正規定義どおりに計算されること。
    #[test]
    fn cache_hit_percent_formula() {
        assert_eq!(compute_cache_hit_percent(0, 0, 0), 0.0);
        // input=100, read=300, write=100 → 300/500 = 60%
        assert!((compute_cache_hit_percent(100, 300, 100) - 60.0).abs() < 1e-9);
        // OpenAI 互換（write=0）: read / (input + read)
        assert!((compute_cache_hit_percent(100, 100, 0) - 50.0).abs() < 1e-9);
    }

    /// テスト用の pi セッション jsonl を作成する。
    fn write_sample_jsonl(dir: &Path) -> PathBuf {
        let path = dir.join("2026-08-29T10-00-00-main_test.jsonl");
        let lines = [
            r#"{"type":"message","id":"e1","parentId":null,"timestamp":"2026-08-29T10:00:00Z","message":{"role":"user","content":[{"type":"text"}]}}"#,
            r#"{"type":"message","id":"e2","parentId":"e1","timestamp":"2026-08-29T10:00:01Z","message":{"role":"assistant","provider":"anthropic","model":"claude","usage":{"input":100,"output":50,"cacheRead":300,"cacheWrite":100,"totalTokens":550,"cost":{"input":0.001,"output":0.002,"cacheRead":0.0003,"cacheWrite":0.001,"total":0.0043}}}}"#,
            r#"{"type":"message","id":"e3","parentId":"e2","timestamp":"2026-08-29T10:00:02Z","message":{"role":"user","content":[{"type":"text"}]}}"#,
            r#"{"type":"message","id":"e4","parentId":"e3","timestamp":"2026-08-29T10:00:03Z","message":{"role":"assistant","provider":"anthropic","model":"claude","usage":{"input":200,"output":80,"cacheRead":400,"cacheWrite":0,"totalTokens":680,"cost":{"input":0.002,"output":0.003,"cacheRead":0.0004,"cacheWrite":0.0,"total":0.0054}}}}"#,
        ];
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    /// cc（Claude Code）セッション jsonl から usage を集計し、モデル別レートで
    /// cost を概算できること（実データの snake_case / cache_creation 内訳に対応）。
    #[test]
    fn collect_metrics_from_file_cc_estimates_cost() {
        let dir = std::env::temp_dir().join(format!("sai_cache_cc_t{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cc_test.jsonl");
        let line = r#"{"type":"assistant","timestamp":"2026-09-22T10:00:00Z","message":{"role":"assistant","model":"claude-sonnet-5","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":2000,"cache_creation_input_tokens":3000,"cache_creation":{"ephemeral_5m_input_tokens":1000,"ephemeral_1h_input_tokens":2000}}}}"#;
        std::fs::write(&path, format!("{}\n", line)).unwrap();

        let m = collect_metrics_from_file_cc(&path).unwrap();
        assert_eq!(m.all_messages.len(), 1);
        let msg = &m.all_messages[0];
        assert_eq!(msg.model, "claude-sonnet-5");
        assert_eq!(msg.input, 1000);
        assert_eq!(msg.cache_read, 2000);
        assert_eq!(msg.cache_write, 3000);

        // sonnet-5: input $2/MTok, output $10/MTok, cache_read 0.1x, cache_write 5m=1.25x/1h=2x
        let expect_input = 1000.0 * 2.00 / 1_000_000.0;
        let expect_output = 500.0 * 10.00 / 1_000_000.0;
        let expect_cache_read = 2000.0 * 2.00 * 0.1 / 1_000_000.0;
        let expect_cache_write = (1000.0 * 2.00 * 1.25 + 2000.0 * 2.00 * 2.0) / 1_000_000.0;
        assert!((msg.cost.input - expect_input).abs() < 1e-9);
        assert!((msg.cost.output - expect_output).abs() < 1e-9);
        assert!((msg.cost.cache_read - expect_cache_read).abs() < 1e-9);
        assert!((msg.cost.cache_write - expect_cache_write).abs() < 1e-9);
    }

    /// jsonl からメトリクス・累積系列・カウントが収集できること。
    #[test]
    fn collect_metrics_parses_jsonl() {
        let dir = std::env::temp_dir().join(format!("sai_cache_t{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_sample_jsonl(&dir);

        let m = collect_metrics_from_file(&path).unwrap();
        // assistant 2 件
        assert_eq!(m.all_messages.len(), 2);
        // 全エントリが分岐なし → アクティブブランチ上
        assert_eq!(m.active_branch_messages.len(), 2);
        // usage の合計
        assert_eq!(m.tree_totals.input, 300);
        assert_eq!(m.tree_totals.cache_read, 700);
        assert_eq!(m.tree_totals.cache_write, 100);
        // 累積系列
        let cum = compute_cumulative_series(&m.all_messages);
        assert_eq!(cum.cum_input, vec![100.0, 300.0]);
        assert_eq!(cum.cum_hit_percent.len(), 2);
        // カウント
        let c = collect_input_counts(&path).unwrap();
        assert_eq!(c.user, 2);
        assert_eq!(c.assistant, 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}

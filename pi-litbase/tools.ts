// ============================================================================
// tools.ts — pi-litbase が登録するツール定義
// ============================================================================
//
// すべて `sai-paper litbase ...`（または `sai-paper get-pdf` 等）を呼ぶ薄い
// ラッパ。LLM 推論は行わず、決定論的な取得・保存・監査のみを提供する。

import { errorResult, jsonResult, runSaiPaper } from "./sai-paper-cli.ts";

type ToolHandler = (params: any) => unknown | Promise<unknown>;

interface LitbaseTool {
  name: string;
  label: string;
  description: string;
  parameters: Record<string, unknown>;
  execute: (
    toolCallId: string,
    params: any,
    signal?: AbortSignal,
    onUpdate?: (update: any) => void,
    ctx?: any,
  ) => Promise<ReturnType<typeof jsonResult>>;
}

/** ツール定義を組み立てるヘルパー（エラー処理を共通化）。 */
function tool(
  name: string,
  label: string,
  description: string,
  parameters: Record<string, unknown>,
  handler: ToolHandler,
): LitbaseTool {
  return {
    name,
    label,
    description,
    parameters,
    execute: async (_toolCallId, params) => {
      try {
        return jsonResult(await handler(params ?? {}));
      } catch (error) {
        return errorResult(
          error instanceof Error ? error.message : String(error),
        );
      }
    },
  };
}

const str = (description: string) => ({ type: "string", description });
const num = (description: string) => ({ type: "number", description });
const bool = (description: string) => ({ type: "boolean", description });

/** 実行番号を出す。`run_id` が無ければ環境変数 `SAI_PARENT_RUN_ID`。両方とも無ければ null。 */
function resolveRunId(params: { run_id?: unknown }): number | null {
  if (params.run_id !== undefined && params.run_id !== null) {
    return Number(params.run_id);
  }
  const fromEnv = process.env.SAI_PARENT_RUN_ID;
  if (!fromEnv) {
    return null;
  }
  const parsed = Number(fromEnv);
  return Number.isFinite(parsed) ? parsed : null;
}

/** 論文解決（Library / DOI / arXiv / タイトル）。 */
export const resolveTool = tool(
  "sai_paper_resolve",
  "Resolve Paper",
  "paper_id / DOI / arXiv ID / タイトルから sai-paper Library の論文を解決する。まずこれで対象論文を特定する。",
  {
    type: "object",
    properties: { query: str("paper_id / DOI / arXiv ID / タイトル") },
    required: ["query"],
  },
  (p) => runSaiPaper(["litbase", "resolve", String(p.query)]),
);

/** Library の最近の論文一覧。 */
export const recentTool = tool(
  "sai_paper_recent",
  "Recent Papers",
  "sai-paper Library の論文を最近順に一覧する。ユーザーに対象論文を選ばせたいときに使う。",
  {
    type: "object",
    properties: { limit: num("取得件数（既定 20）") },
  },
  (p) =>
    runSaiPaper([
      "litbase",
      "recent",
      "--limit",
      String(p.limit ?? 20),
    ]),
);

/** S2 メタデータ取得（Paper Weight 用）。 */
export const lookupTool = tool(
  "sai_paper_lookup",
  "Lookup Metadata",
  "Semantic Scholar から書誌メタデータを取得する。Paper Weight（ジャーナル・被引用数・著者）は必ずこの結果のみを使い、捏造してはならない。",
  {
    type: "object",
    properties: {
      doi: str("DOI"),
      title: str("タイトル（DOI が無い場合）"),
    },
  },
  (p) => {
    const args = ["litbase", "lookup"];
    if (p.doi) args.push("--doi", String(p.doi));
    if (p.title) args.push("--title", String(p.title));
    return runSaiPaper(args);
  },
);

/** PDF 確保。 */
export const ensurePdfTool = tool(
  "sai_paper_ensure_pdf",
  "Ensure PDF",
  "論文の PDF を確保する。DB 登録済み → arXiv → S2 OA → Europe PMC → Unpaywall の順に合法的な OA のみ試す。paywall の場合は status=manual_required を返すので、ユーザーに PDF の手動保存を依頼し sai_paper_import_pdf で取り込む。",
  {
    type: "object",
    properties: { query: str("paper_id / DOI / arXiv ID / タイトル") },
    required: ["query"],
  },
  (p) =>
    runSaiPaper(["litbase", "ensure-pdf", String(p.query)], {
      timeoutMs: 300_000,
    }),
);

/** 本文 Markdown 取得。 */
export const textTool = tool(
  "sai_paper_text",
  "Extract Text",
  "docling で PDF を抽出した本文 Markdown を返す。論文の精読・ノート生成にはこの本文を使う。初回は数分かかる場合がある。",
  {
    type: "object",
    properties: { query: str("paper_id / DOI / arXiv ID / タイトル") },
    required: ["query"],
  },
  (p) =>
    runSaiPaper(["litbase", "text", String(p.query)], {
      timeoutMs: 600_000,
    }),
);

/** 手動 PDF 取込。 */
export const importPdfTool = tool(
  "sai_paper_import_pdf",
  "Import PDF",
  "ユーザーが手動で保存した PDF を sai-paper に取り込む。検証・保存・論文への紐付けを内部で行う。pi agent 自身が PDF を書き込んではならない。",
  {
    type: "object",
    properties: {
      query: str("paper_id / DOI / arXiv ID / タイトル"),
      pdf: str("PDF の絶対パス"),
    },
    required: ["query", "pdf"],
  },
  (p) =>
    runSaiPaper([
      "litbase",
      "import-pdf",
      String(p.query),
      "--pdf",
      String(p.pdf),
    ]),
);

/** 直近ダウンロード候補。 */
export const recentDownloadsTool = tool(
  "sai_paper_recent_downloads",
  "Recent Downloads",
  "ユーザーのダウンロードフォルダにある直近の PDF 候補を列挙する。手動取込の候補提示に使う。",
  {
    type: "object",
    properties: { limit: num("取得件数（既定 20）") },
  },
  (p) =>
    runSaiPaper([
      "litbase",
      "recent-downloads",
      "--limit",
      String(p.limit ?? 20),
    ]),
);

/** 検索セット: データベース横断検索。 */
export const searchDbTool = tool(
  "sai_paper_search_db",
  "Search Databases",
  "複数のデータベース（PubMed, bioRxiv, Semantic Scholar, arXiv, OpenAlex）を横断検索する。重複除外済みでSEARCH_SET を作成することもできる。研究方針の木の NODE で探すときは node を付ける（検索の記録が残る）。",
  {
    type: "object",
    properties: {
      query: str("検索クエリ（自然言語）"),
      databases: {
        type: "array",
        items: { type: "string" },
        description: "DB 名（pubmed, biorxiv, medrxiv, s2, arxiv, openalex）— 複数指定可、省略時は config の default",
      },
      limit: num("取得件数（既定 50）"),
      year_from: num("開始年"),
      year_to: num("終了年"),
      raw_query: str("DB 固有構文（PubMed Boolean など）"),
      save_as: str("セット名（指定すると検索セットを作成）"),
      node: str("研究方針の Node（Q12・O5 など）。記録に残り、前の検索を見られる。find ではその Node の下の F の論文に絞る"),
      purpose: str("何のための検索か"),
      extract: str("読み、そこから抜き出す項目"),
      widen: str("範囲を広げた理由"),
    },
    required: ["query"],
  },
  (p) => {
    const args = ["search", String(p.query)];
    (p.databases ?? []).forEach((db: unknown) => {
      args.push("--db", String(db));
    });
    if (p.limit) args.push("--limit", String(p.limit));
    if (p.year_from) args.push("--year-from", String(p.year_from));
    if (p.year_to) args.push("--year-to", String(p.year_to));
    if (p.raw_query) args.push("--raw-query", String(p.raw_query));
    if (p.save_as) args.push("--save-as", String(p.save_as));
    if (p.node) args.push("--node", String(p.node));
    if (p.purpose) args.push("--purpose", String(p.purpose));
    if (p.extract) args.push("--extract", String(p.extract));
    if (p.widen) args.push("--widen", String(p.widen));
    args.push("--json");
    return runSaiPaper(args);
  },
);

/** 検索セット: 一覧取得。 */
export const setListTool = tool(
  "sai_paper_set_list",
  "List Search Sets",
  "作成済みの検索セット一覧を取得する。",
  { type: "object", properties: {} },
  () => runSaiPaper(["set", "list", "--json"]),
);

/** 検索セット: 内容表示。 */
export const setShowTool = tool(
  "sai_paper_set_show",
  "Show Search Set",
  "検索セットの内容を表示する。decision フィルタで pending / keep / drop / downloaded に絞り込める。",
  {
    type: "object",
    properties: {
      name: str("セット名または ID"),
      decision: str("絞り込み：pending / keep / drop / downloaded"),
    },
    required: ["name"],
  },
  (p) => {
    const args = ["set", "show", String(p.name)];
    if (p.decision) args.push("--decision", String(p.decision));
    args.push("--json");
    return runSaiPaper(args);
  },
);

/** 検索セット: アイテムをマーク。 */
export const setMarkTool = tool(
  "sai_paper_set_mark",
  "Mark Items",
  "検索セット内の1件以上のアイテムに decision (keep/drop/pending) と理由をマークする。スクリーニング作業に使う。",
  {
    type: "object",
    properties: {
      name: str("セット名または ID"),
      positions: {
        type: "array",
        items: { type: "number" },
        description: "マーク対象の position（1から開始）",
      },
      decision: str("判定：keep / drop / pending"),
      reason: str("判定理由（任意）"),
    },
    required: ["name", "positions", "decision"],
  },
  (p) => {
    const args = ["set", "mark", String(p.name)];
    (p.positions ?? []).forEach((pos: unknown) => {
      args.push("--position", String(pos));
    });
    args.push("--decision", String(p.decision));
    if (p.reason) args.push("--reason", String(p.reason));
    args.push("--json");
    return runSaiPaper(args);
  },
);

/** 検索セット: マージ。 */
export const setMergeTool = tool(
  "sai_paper_set_merge",
  "Merge Search Sets",
  "複数の検索セットを1つに統合する。重複除外済み。",
  {
    type: "object",
    properties: {
      sources: {
        type: "array",
        items: { type: "string" },
        description: "統合元のセット名/ID（複数）",
      },
      as: str("統合後のセット名"),
    },
    required: ["sources", "as"],
  },
  (p) => {
    const args = ["set", "merge"];
    (p.sources ?? []).forEach((src: unknown) => {
      args.push(String(src));
    });
    args.push("--as", String(p.as));
    args.push("--json");
    return runSaiPaper(args);
  },
);

/** RAG ハイブリッド検索（word/vector/hybrid）。 */
export const findTool = tool(
  "sai_paper_find",
  "Find Chunks",
  "Library、ノート内でハイブリッド検索を実行する。埋め込みサーバが動いていないときは --mode word を使うこと（日本語は当たらないため英語のみ）。研究方針の木の NODE で探すときは node を付ける（検索の記録が残る）。",
  {
    type: "object",
    properties: {
      query: str("検索クエリ（自然言語）"),
      mode: str("検索モード：word / vector / hybrid（既定: config）"),
      top_k: num("上位 K 件（既定: config）"),
      scope: str("インデックス対象：papers / notes / all（既定: papers）"),
      paper: num("論文 ID（指定時はその論文に絞り込む、任意）"),
      index: str("索引の名前（既定: settings.json の rag.default_index。埋め込みモデルも索引で決まる）"),
      alpha: num("ハイブリッド時の vector 側重み 0.0-1.0（既定: config）"),
      fusion: str("融合方法：weighted / rrf（既定: config）"),
      papers: {
        type: "array",
        items: { type: "number" },
        description: "論文の集合で絞る。`paper` と同じ扱い",
      },
      node: str("研究方針の Node（Q12・O5 など）。記録に残り、前の検索を見られる。find ではその Node の下の F の論文に絞る"),
      subtree: bool("`node` の子孫の NODE も含める"),
      each: num("SCOPE の論文ごとに上位 m 件を返す（`papers` か `node` と一緒に使う）"),
      widen: str("範囲を広げた理由"),
    },
    required: ["query"],
  },
  (p) => {
    const args = ["find", String(p.query)];
    if (p.mode) args.push("--mode", String(p.mode));
    if (p.top_k) args.push("--top-k", String(p.top_k));
    if (p.scope) args.push("--scope", String(p.scope));
    if (p.paper) args.push("--paper", String(p.paper));
    if (p.index) args.push("--index", String(p.index));
    if (p.alpha) args.push("--alpha", String(p.alpha));
    if (p.fusion) args.push("--fusion", String(p.fusion));
    // 空の配列を渡されたときは `--papers` を付けない（無い値に空文字列を送ると CLI が怒る）。
    const papers = (p.papers ?? []) as unknown[];
    if (papers.length > 0) args.push("--papers", papers.map(String).join(","));
    if (p.node) args.push("--node", String(p.node));
    if (p.subtree) args.push("--subtree");
    if (p.each) args.push("--each", String(p.each));
    if (p.widen) args.push("--widen", String(p.widen));
    args.push("--json");
    return runSaiPaper(args);
  },
);

/** 論文の近傍チャンク検索。 */
export const neighborsTool = tool(
  "sai_paper_neighbors",
  "Find Similar Chunks",
  "指定した論文に意味的に近いチャンク（別の論文のセクション、ノートなど）をベクトル検索で返す。引用をたどる補助に使う。",
  {
    type: "object",
    properties: {
      paper_id: num("論文 ID"),
      top_k: num("上位 K 件（既定: 20）"),
      index: str("索引の名前（既定: settings.json の rag.default_index）"),
    },
    required: ["paper_id"],
  },
  (p) => {
    const args = ["neighbors", String(p.paper_id)];
    if (p.top_k) args.push("--top-k", String(p.top_k));
    if (p.index) args.push("--index", String(p.index));
    args.push("--json");
    return runSaiPaper(args);
  },
);

/** インデックス状態表示。 */
export const indexStatusTool = tool(
  "sai_paper_index_status",
  "Index Status",
  "チャンク化と埋め込みのインデックス状態を表示する。チャンク数、モデル、未計算分などを確認できる。",
  { type: "object", properties: {} },
  () => runSaiPaper(["index", "status"]),
);

/** 実行番号が無いときのエラーの文。 */
const NO_RUN_ID = "実行番号がありません（run_id か環境変数 SAI_PARENT_RUN_ID）";

/** 研究方針の木: 今の実行の SCOPE。 */
export const sciContextTool = tool(
  "sai_paper_sci_context",
  "Sci Context",
  "今の実行の文脈（研究方針の木から範囲を切り出したもの・出せる PROPOSAL の op の説明）を返す",
  {
    type: "object",
    properties: {
      run_id: num("省略時は環境変数 SAI_PARENT_RUN_ID"),
      scope: str("SCOPE：node / path / subtree / siblings / summary / blind"),
    },
  },
  (p) => {
    // context の --run は省略できない。番号が無いなら CLI を呼ばずに落とす。
    const runId = resolveRunId(p);
    if (runId === null) {
      throw new Error(NO_RUN_ID);
    }
    const args = ["sci", "context", "--run", String(runId)];
    if (p.scope) args.push("--scope", String(p.scope));
    return runSaiPaper(args);
  },
);

/** 研究方針の木: 全体の現況。 */
export const sciStatusTool = tool(
  "sai_paper_sci_status",
  "Sci Status",
  "研究方針の木の全体（Q・O・F・E、Objectivity・Logicality、進み具合、SEED、Next Action、DEBT）。brief を true にすると進み具合・Next Action・DEBT の数・注意だけになる",
  {
    type: "object",
    properties: {
      brief: bool("true のとき進み具合・Next Action・DEBT の数・注意だけ返す"),
    },
  },
  (p) => runSaiPaper(["sci", "status", p.brief ? "--brief" : "--json"]),
);

/** 研究方針の木: NODE 1 つ。 */
export const sciNodeTool = tool(
  "sai_paper_sci_node",
  "Sci Node",
  "Node 1 つの中身（親までの道・子・ガイド・Logicality の記録・決定の記録・Action の履歴・仕分け待ち・Next Action・検索の記録）",
  {
    type: "object",
    properties: { node: str("Q12・O5・F3・E2 の形") },
    required: ["node"],
  },
  (p) => runSaiPaper(["sci", "node", String(p.node), "--json"]),
);

/** 研究方針の木: 変更の提案。 */
export const sciProposeTool = tool(
  "sai_paper_sci_propose",
  "Sci Propose",
  "研究方針の木への変更を PROPOSAL として出す。O・F・E・Logicality はすぐ入り、Q を足す・写す・✕ は利用者が仕分けてから入る。★ などは出せない。1 件でも違反があれば何も入らない",
  {
    type: "object",
    properties: {
      proposals: {
        type: "array",
        items: { type: "object" },
        description: "op の JSON",
      },
      run_id: num("省略時は環境変数 SAI_PARENT_RUN_ID"),
    },
    required: ["proposals"],
  },
  (p) => {
    // 番号が無ければ --run を付けない。run 不明として CLI が扱う。
    const args = ["sci", "propose"];
    const runId = resolveRunId(p);
    if (runId !== null) {
      args.push("--run", String(runId));
    }
    return runSaiPaper(args, {
      input: JSON.stringify({ proposals: p.proposals }),
    });
  },
);

/** 登録する全ツール。 */
export const litbaseTools: LitbaseTool[] = [
  resolveTool,
  recentTool,
  lookupTool,
  ensurePdfTool,
  textTool,
  importPdfTool,
  recentDownloadsTool,
  searchDbTool,
  setListTool,
  setShowTool,
  setMarkTool,
  setMergeTool,
  findTool,
  neighborsTool,
  indexStatusTool,
  sciContextTool,
  sciStatusTool,
  sciNodeTool,
  sciProposeTool,
];

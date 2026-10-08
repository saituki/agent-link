/**
 * ask_user.ts — pi 用の質問ツール（sai-web 連携付き）.
 *
 * 元ファイル: ~/workdir/Pi/my-extension/ask_user_question/index.ts（152行）
 * （元ファイルは編集せず，ここにコピーして拡張した）
 *
 * cc の AskUserQuestion と同じ書式で質問を受け付け，
 * 端末に選択画面を出すと同時に sai-web サーバーにも登録する．
 * iPhone と端末のどちらで答えても同じ結果になる．
 * 先に答えた方が勝ち，もう片方の画面は自動で消える．
 *
 * 変更点:
 * - 質問は最大4問（元は1問のみ）
 * - 選択肢は label と description を持つ物の配列（元は文字列の配列）
 * - header（短い見出し）と multiSelect（複数選択）に対応
 * - sai-web への登録と，外から閉じる仕組み（AbortSignal）を追加
 * - iPhone 側は全問まとめて1回で答えを返してくる（1画面に全問並べて送信）．
 *   pollAnswer はその全問分の配列を受け取り，古い1問分の形（後方互換）も読む．
 * - 複数選択（choice_indices）の回答も読む（05 §1 の2番）．
 * - 質問中は herdr へ `herdr:blocked` を出し，終わった時はどの場合も外す
 *   （05 §3.1。出さないとサーバーが登録直後に「解決済み」にしてしまう）．
 * - PC で答えたと伝えるのは専用 API `POST /api/questions/{id}/resolved`
 *   （05 §3.1。以前の __resolved_on_terminal__ はサーバーが無視せず
 *   端末へ打ち込んでしまう不具合があった）．
 *
 * iPhone 側の回答の待ち方はポーリング（1秒おきにサーバーへ聞きに行く）．
 * WebSocket は使わない．拡張機能の中で接続を維持する作りは，
 * pi の再読み込みやセッション終了との兼ね合いで壊れやすいため．
 * Web 側の答えを取りこぼさないため，ポーリングの回数はサーバーの期限
 * （`questions.py` の `_PI_EXPIRE_SEC`）に合わせてある（B6）．
 */

// 子の run の質問を親（起動者）に知らせるために sai-link を呼ぶ．
// 答えの承認を記録するために sai-sci を呼ぶ（U04・同期で待つ。記録の後に返すため）。
import { execFile, spawnSync } from "node:child_process";

// ---------------------------------------------------------------------------
// 型定義（外部依存なし．pi 本体の型に合わせる）
// ---------------------------------------------------------------------------

interface QuestionOption {
  label: string;
  description?: string;
}

interface PhoneQuestion {
  question: string;
  header: string;
  multiSelect: boolean;
  options: QuestionOption[];
}

interface ToolContext {
  hasUI?: boolean;
  mode?: string;
  cwd?: string;
  signal?: AbortSignal;
  sessionManager?: {
    getSessionFile?: () => string | undefined;
    getSessionId?: () => string | undefined;
  };
  ui?: {
    select: (title: string, options: string[], opts?: { signal?: AbortSignal }) => Promise<string | undefined>;
    input: (title: string, placeholder?: string, opts?: { signal?: AbortSignal }) => Promise<string | undefined>;
    notify: (msg: string, level?: string) => void;
  };
}

// process.env を読むための小さな宣言（Node の型定義なしで動かすため）
declare const process: { env: Record<string, string | undefined> } | undefined;

// 環境変数（process.env と同じ形。テストから差し替えられるように引数で受ける）
type PhoneEnv = Record<string, string | undefined>;

// サーバーが返す質問 1 件の読み取り結果（`GET /api/questions/{qid}`）
type ServerQuestionPayload = {
  state?: string;
  answer?: {
    answers?: Array<{
      choice_index?: number;
      choice_indices?: number[];
      text?: string;
    }>;
    choice_index?: number;
    text?: string;
  };
  questions?: PhoneQuestion[];
  answered_by?: string;
};

// 質問 → 答え（質問文の鍵）
type Answers = Record<string, string>;

// 保存された答えと、答えた人の名前（承認の記録に使う）
type PolledAnswer = {
  answers: Answers;
  answeredBy: string;
};

// 外から呼ぶコマンドの呼び方（親の通知 `task question`・承認の記録 `approval record`）
type ParentNotifyCommand = {
  bin: string;
  args: string[];
};

interface ToolInput {
  name: string;
  label?: string;
  description: string;
  parameters: unknown;
  execute: (
    toolCallId: string,
    params: Record<string, unknown>,
    signal?: AbortSignal,
    onUpdate?: (update: { content: Array<{ type: string; text?: string }> }) => void,
    ctx?: ToolContext,
  ) => Promise<{ content: Array<{ type: string; text?: string }>; isError?: boolean; details?: Record<string, unknown> }>;
}

interface ExtensionAPI {
  registerTool: (tool: ToolInput) => void;
  on: (event: string, handler: (event: Record<string, unknown>, ctx: ToolContext) => void | Promise<void>) => void;
  // herdr-agent-state.ts が聞いているイベント（質問待ちの合図・05 §3.1）
  events?: {
    emit: (event: string, data: Record<string, unknown>) => void;
  };
}

// ---------------------------------------------------------------------------
// 設定
// ---------------------------------------------------------------------------

// sai-web サーバーの場所．家の中だけの接続（Tailscale 前提）．
// 置き場は環境変数 `SAI_WEB_URL` で差し替えられる（U17 の共有契約）。
const SAI_WEB_URL = (typeof process !== "undefined" && process.env.SAI_WEB_URL) || "http://127.0.0.1:8200";
// 内部用の合言葉．あればすべての要求に Authorization: Bearer を付ける（共有契約 C3）．
const SAI_WEB_INTERNAL_TOKEN = (typeof process !== "undefined" && process.env.SAI_WEB_INTERNAL_TOKEN) || "";
// iPhone の回答を待つ間隔（ミリ秒）と上限回数（10分）
const POLL_INTERVAL_MS = 1000;
// 上限回数はサーバーの期限（`apps/web/src/sai_web/questions.py` の
// `_PI_EXPIRE_SEC` = 600 秒）とそろえる。ここが短いと、サーバーは答えを
// 受け付けたまま拡張は打ち切るという状態になり，答えが pi に届かない（B6）。
const POLL_MAX_COUNT = 600;
// headless（画面が無い）ときの上限回数．24 時間（cc の hooks の timeout 86400 秒とそろえる）．
const POLL_MAX_COUNT_HEADLESS = 86400;

const CUSTOM_LABEL = "Type something.";

// ---------------------------------------------------------------------------
// 戻り値の作り方
// ---------------------------------------------------------------------------

function result(text: string, details: Record<string, unknown> = {}) {
  return { content: [{ type: "text" as const, text }], details };
}

function errorResult(text: string, details: Record<string, unknown> = {}) {
  return { content: [{ type: "text" as const, text }], isError: true, details };
}

/** cc の返し方に寄せる（同じ書式の指示が両方で通るようにするため） */
function answeredText(answers: Record<string, string>): string {
  const parts = Object.entries(answers).map(([q, a]) => `"${q}"="${a}"`);
  return `Your questions have been answered: ${parts.join(", ")}.`;
}

// ---------------------------------------------------------------------------
// sai-web サーバーとのやり取り（失敗しても端末だけで完結できるようにする）
// ---------------------------------------------------------------------------

/** `SAI_WEB_INTERNAL_TOKEN` があれば Authorization を足したヘッダを返す（共有契約 C3）． */
function authHeaders(): Record<string, string> {
  const headers: Record<string, string> = {};
  if (SAI_WEB_INTERNAL_TOKEN) headers["Authorization"] = `Bearer ${SAI_WEB_INTERNAL_TOKEN}`;
  return headers;
}

async function registerToServer(payload: Record<string, unknown>): Promise<string | null> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 3000);
    const res = await fetch(`${SAI_WEB_URL}/api/questions`, {
      method: "POST",
      headers: { "Content-Type": "application/json", ...authHeaders() },
      body: JSON.stringify(payload),
      signal: controller.signal,
    });
    clearTimeout(timer);
    if (!res.ok) return null;
    const data = (await res.json()) as { id?: string };
    return data.id ?? null;
  } catch {
    // サーバー停止中でも端末の質問は普通に使える
    return null;
  }
}

async function tellResolved(questionId: string): Promise<void> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 3000);
    // PC で答えたことを記録させる専用 API（05 §3.1）。
    // 以前は {text:"__resolved_on_terminal__"} を回答 API に送っていたが，
    // サーバーはそれを特別扱いせず端末に打ち込んでいた（不具合）。
    await fetch(`${SAI_WEB_URL}/api/questions/${questionId}/resolved`, {
      method: "POST",
      headers: { "Content-Type": "application/json", ...authHeaders() },
      body: JSON.stringify({}),
      signal: controller.signal,
    });
    clearTimeout(timer);
  } catch {
    // 伝えられなくても pi の動作には影響させない
  }
}

// ---------------------------------------------------------------------------
// 子の run の質問を親（起動者）に知らせる（共有契約 C6）
//
// 利用者の決まりは「研究者の質問には、そのカードの起動者（executer）が答える」．
// 親の run から起こされたカードで，子のエージェントが
//  ask_user で聞いたときはその親に知らせる．
// 返事（`task_output` の `status: "stalled"`）は親が出すので、ここでは
// 通知だけを送り、待つことはしない（失敗しても質問は Web に出ている）．
// ---------------------------------------------------------------------------

function shouldNotifyParent(env: PhoneEnv | undefined): boolean {
  /** 子の run（`SAI_IS_SUBTASK=1`）かどうか。親のない run では呼ばない。 */
  return !!env && env.SAI_IS_SUBTASK === "1";
}

function parentNotifyArgs(serverId: string, questions: PhoneQuestion[], env: PhoneEnv | undefined): ParentNotifyCommand {
  /** 親に知らせるコマンド（通知を待たないため，引数は手で組み立てる）。 */
  const bin = (env && env.SAI_LINK_BIN) || "sai-link";
  // 見出しは 1 行に畳んで 400 文字で切る（サーバーが 500 文字で切るのより短くする）
  const head = questions.map((q) => q.question).join(" / ").replace(/\s+/g, " ").slice(0, 400);
  return { bin, args: ["task", "question", "--qid", serverId, "--text", head] };
}

function notifyParent(serverId: string, questions: PhoneQuestion[], env: PhoneEnv | undefined): void {
  /** 親へ通知する（待たない。失敗しても質問は続ける）。 */
  if (!shouldNotifyParent(env)) return;
  const cmd = parentNotifyArgs(serverId, questions, env);
  // 呼び出しは待たない（質問の待ち時間を増やさないため）
  void execFile(cmd.bin, cmd.args, { timeout: 10000 }, () => {});
}

// ---------------------------------------------------------------------------
// 答えを承認として記録する（共有契約 C8）
// ---------------------------------------------------------------------------

function approvalRecordArgs(
  serverId: string,
  questions: PhoneQuestion[],
  answers: Answers,
  answeredBy: string,
  env: PhoneEnv | undefined,
): ParentNotifyCommand | null {
  /** `sai-sci approval record` の呼び方を組み立てる。親の run（SAI_PARENT_RUN_ID）が無ければ null。 */
  if (!env || !env.SAI_PARENT_RUN_ID) return null;
  const texts = questions.map((q) => q.question);
  return {
    bin: env.SAI_SCI_BIN || "sai-sci",
    args: [
      "approval",
      "record",
      "--qid",
      serverId,
      "--question",
      texts.join("\n"),
      "--answer",
      texts.map((t) => answers[t] ?? "").join("\n"),
      "--answered-by",
      answeredBy,
    ],
  };
}

function recordApproval(
  serverId: string,
  questions: PhoneQuestion[],
  answers: Answers,
  answeredBy: string,
  env: PhoneEnv | undefined,
): void {
  /** 答えを記録する。承認は後の `sai-sci op` より先に入っている必要があるので待つ（10 秒の上限）。失敗は無視する。 */
  const cmd = approvalRecordArgs(serverId, questions, answers, answeredBy, env);
  if (!cmd) return;
  try {
    const res = spawnSync(cmd.bin, cmd.args, { timeout: 10000 });
    if (res.status !== 0) {
      console.warn(`[ask_user] approval record が失敗しました（終了コード ${res.status}）`);
    }
  } catch {
    // 記録できなくても質問の答えは返す（後の op が許可なしで止まって気づける）
  }
}

// ---------------------------------------------------------------------------
// サーバーが保存した答えを読む
// ---------------------------------------------------------------------------

function answersFromState(data: ServerQuestionPayload): Record<string, string> | null {
  /** サーバーが保存した答えを pi が使える形（問い → 答え）に変える。

   * 答えが無いとき（`state` が `answered` 以外）は `null` を返す。
   * ポーリングの変換（`pollAnswer`）と 1 回だけ読む処理（`answeredOnServer`）で共有する。
   */
  if (data.state !== "answered" || !data.answer) return null;
  const qs = data.questions ?? [];
  // 新形式（全問分の配列）を優先し，無ければ古い1問分の形として扱う（後方互換）
  const items = data.answer.answers ?? [
    { choice_index: data.answer.choice_index, text: data.answer.text },
  ];
  const answers: Record<string, string> = {};
  items.forEach((item, i) => {
    const q = qs[i];
    if (!q) return;
    if (item?.text) {
      answers[q.question] = item.text;
    } else if (Array.isArray(item?.choice_indices) && item.choice_indices.length) {
      // 複数選択: 選んだ番号を全部ラベルにして渡す（05 §1 の2番）
      answers[q.question] = item.choice_indices
        .map((n) => q.options[n]?.label ?? "")
        .filter(Boolean)
        .join(", ");
    } else if (typeof item?.choice_index === "number") {
      const opt = q.options[item.choice_index];
      answers[q.question] = opt?.label ?? "";
    }
  });
  return answers;
}

function answeredByOf(data: ServerQuestionPayload): string {
  /** 答えた人の名前（サーバーの `answered_by`。無ければ user）。 */
  return data.answered_by || "user";
}

async function answeredOnServer(questionId: string, url: string = SAI_WEB_URL): Promise<PolledAnswer | null> {
  /** サーバー側に保存済みの答えと答えた人を 1 回だけ読む（取れなければ `null`）。

   * 端末（herdr）で何かが選ばれたとき、サーバーに既に答えがあっても
   * 端末の選択を採ると Web の答えが捨てられる（B6）。そこで 1 回だけ読んで
   * 読み込み済みならそちらを採る。取りに行けないときは取り損ねず
   * 端末の選択を採る（今までと同じ動き）。
   */
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 3000);
    const res = await fetch(`${url}/api/questions/${questionId}`, {
      headers: authHeaders(),
      signal: controller.signal,
    });
    clearTimeout(timer);
    if (!res.ok) return null;
    const data = await res.json();
    const answers = answersFromState(data);
    return answers ? { answers, answeredBy: answeredByOf(data) } : null;
  } catch {
    // 通信できないときは端末の選択を採る（今までと同じ）
    return null;
  }
}

// ---------------------------------------------------------------------------
// 質問の答えを待つ
// ---------------------------------------------------------------------------

async function pollAnswer(
  questionId: string,
  maxCount: number = POLL_MAX_COUNT,
): Promise<PolledAnswer | null> {
  for (let i = 0; i < maxCount; i++) {
    await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
    try {
      const res = await fetch(`${SAI_WEB_URL}/api/questions/${questionId}`, {
        headers: authHeaders(),
      });
      if (!res.ok) continue;
      const data = (await res.json()) as ServerQuestionPayload;
      // 答えに変換する処理は `answeredOnServer` と共有する（変換が二重にならないように）
      const answers = answersFromState(data);
      if (answers) return { answers, answeredBy: answeredByOf(data) };
      if (data.state && data.state !== "open") return null;
    } catch {
      // 通信が切れている間は待ち続ける
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// 質問の実行
// ---------------------------------------------------------------------------

function normalizeQuestions(params: Record<string, unknown>): PhoneQuestion[] {
  const raw = Array.isArray(params.questions) ? params.questions : [];
  return raw.slice(0, 4).map((q, i) => {
    const obj = (q ?? {}) as Record<string, unknown>;
    const opts = Array.isArray(obj.options) ? obj.options : [];
    return {
      question: String(obj.question ?? ""),
      header: String(obj.header ?? `質問${i + 1}`),
      multiSelect: obj.multiSelect === true,
      options: opts.slice(0, 4).map((o) => {
        const oo = (o ?? {}) as Record<string, unknown>;
        // 文字列だけの古い形も受け付ける
        if (typeof o === "string") return { label: o };
        return { label: String(oo.label ?? ""), description: String(oo.description ?? "") };
      }),
    };
  });
}

async function handleAskUserPhone(
  _toolCallId: string,
  params: Record<string, unknown>,
  _signal?: AbortSignal,
  _onUpdate?: (update: { content: Array<{ type: string; text?: string }> }) => void,
  ctx?: ToolContext,
): Promise<{ content: Array<{ type: string; text?: string }>; isError?: boolean; details?: Record<string, unknown> }> {
  const questions = normalizeQuestions(params);
  if (questions.length === 0) {
    return errorResult("質問がありません", { answers: null });
  }
  const env: PhoneEnv | undefined = typeof process !== "undefined" ? process.env : undefined;

  // 質問ID：サーバー登録後に差し替える（登録前から一意にしておく）
  const localId = `local-${Date.now()}`;
  const paneId = typeof process !== "undefined" ? (process.env.HERDR_PANE_ID ?? "") : "";
  // セッション ID は API から取る（00 §3.7 の対応づけのため）。
  // 取れない時だけファイルパスにフォールバックするが，その場合
  // 実行と質問が対応づけられないためログに出す。
  const sessionId = ctx?.sessionManager?.getSessionId?.() ?? "";
  const sessionFile = ctx?.sessionManager?.getSessionFile?.() ?? "";
  if (!sessionId && sessionFile) {
    console.warn(
      "[ask_user] getSessionId() を取得できないためファイルパスで登録します（実行と質問の対応づけはできません）:",
      sessionFile,
    );
  }

  // headless（画面が無い）: 端末の選択画面を出さず，Web の答えだけを待つ（共有契約 C1・C3）。
  const headless =
    ctx?.mode === "headless" ||
    (typeof process !== "undefined" && process.env.SAI_AGENT_TRANSPORT === "headless");
  if (!ctx?.hasUI || !ctx?.ui || headless) {
    const serverId = await registerToServer({
      agent: "pi",
      session_id: sessionId || sessionFile,
      pane_id: paneId,
      tool_use_id: localId,
      questions,
    });
    if (!serverId) {
      return errorResult("UI is unavailable in this mode.", { answers: null });
    }
    const got = await pollAnswer(serverId, POLL_MAX_COUNT_HEADLESS);
    if (got) {
      recordApproval(serverId, questions, got.answers, got.answeredBy, env);
      return result(answeredText(got.answers), { answers: got.answers });
    }
    return result("User cancelled.", { answers: null });
  }
  const ui = ctx.ui;

  // sai-web に登録する（失敗しても端末だけで続ける）
  const serverId = await registerToServer({
    agent: "pi",
    session_id: sessionId || sessionFile,
    pane_id: paneId,
    tool_use_id: localId,
    questions,
  });

  // 子の run なら，質問を親（カードの起動者）に知らせる（共有契約 C6）。
  // 待たない。通知に失敗しても質問は Web に出ている。
  if (serverId) notifyParent(serverId, questions, env);

  // この段階では1問ずつ順に聞く．複数問・複数選択の同時操作は
  // 端末画面の仕様に依存し壊れやすいため，順番に1問ずつ出す割り切り．
  // iPhone 側は全問まとめて1回で答えてくるため，届いた時点で全問揃っていれば
  // 端末側の残りの質問は出さずに打ち切る．
  const answers: Answers = {};
  // 答えた人の名前（承認の記録に使う）。端末で選んだときは terminal。
  let answeredBy = "";
  for (const q of questions) {
    if (questions.every((qq) => Object.prototype.hasOwnProperty.call(answers, qq.question))) {
      break;
    }
    const controller = new AbortController();
    const labels = q.options.map((o) =>
      o.description && o.description !== o.label ? `${o.label} — ${o.description}` : o.label
    );
    const selectable = [...labels, CUSTOM_LABEL];
    const title = q.header ? `${q.header}: ${q.question}` : q.question;

    // iPhone の回答を並行して待つ（サーバー登録できた場合のみ）
    let phoneAnswer: Promise<PolledAnswer | null> | null = null;
    if (serverId) {
      phoneAnswer = pollAnswer(serverId).then((a) => {
        if (a) controller.abort();
        return a;
      });
    }

    const selected = await ui.select(title, selectable, { signal: controller.signal });

    if (selected) {
      // 端末で選ばれても、サーバーに保存済みの答えがあればそちらを採る（B6）。
      // herdr で送った down・enter が端末の選択になり、Web の答えが
      // 捨てられていた取りこぼしを塞ぐ。
      const fromWeb = serverId ? await answeredOnServer(serverId) : null;
      if (fromWeb && Object.keys(fromWeb.answers).length) {
        for (const [k, v] of Object.entries(fromWeb.answers)) answers[k] = v;
        answeredBy = fromWeb.answeredBy;
        continue;
      }
      // 端末で選ばれた → サーバーに伝えて iPhone のカードを「PC で回答済み」に
      answeredBy = "terminal";
      if (serverId) void tellResolved(serverId);
      if (selected === CUSTOM_LABEL) {
        const custom = await ui.input("Your answer");
        if (!custom) return result("User cancelled.", { answers: null });
        answers[q.question] = custom;
      } else {
        const index = labels.indexOf(selected);
        answers[q.question] = q.options[index]?.label ?? selected;
      }
    } else if (controller.signal.aborted) {
      // 外から閉じた ＝ iPhone で答えられた
      const a = serverId ? await phoneAnswer : null;
      if (a) {
        for (const [k, v] of Object.entries(a.answers)) answers[k] = v;
        answeredBy = a.answeredBy;
      } else {
        return result("User cancelled.", { answers: null });
      }
    } else {
      // 利用者が Esc で取り消した
      return result("User cancelled.", { answers: null });
    }
  }

  // 答えを承認として記録してから返す（後の `sai-sci op` が許可を見つけられるように）
  if (serverId) recordApproval(serverId, questions, answers, answeredBy, env);
  return result(answeredText(answers), { answers });
}

// ---------------------------------------------------------------------------
// ツール定義（typebox 形式．Google のモデルとの互換性のため StringEnum 相当の
//  文字列 enum を使う．ここでは外部依存なしの素の定義で同じ形にする）
// ---------------------------------------------------------------------------

const askUserPhoneTool: ToolInput = {
  name: "ask_user",
  label: "Ask User",
  description:
    "Ask the user a question with predefined choices. " +
    "Use ask_user when you need clarification, a decision, or additional input. " +
    "The question also appears on the user's iPhone; answering from either side works.",
  parameters: {
    type: "object",
    properties: {
      questions: {
        type: "array",
        maxItems: 4,
        description: "Questions to ask (max 4)",
        items: {
          type: "object",
          properties: {
            question: { type: "string", description: "Question shown to the user" },
            header: { type: "string", description: "Short header (about 12 chars)" },
            multiSelect: { type: "boolean", description: "Whether multiple selection is allowed" },
            options: {
              type: "array",
              minItems: 2,
              maxItems: 4,
              description: "Selectable options (2-4)",
              items: {
                type: "object",
                properties: {
                  label: { type: "string", description: "Short text shown on the button" },
                  description: { type: "string", description: "Supplementary explanation (1-2 sentences)" },
                },
                required: ["label"],
              },
            },
          },
          required: ["question", "header", "options"],
        },
      },
    },
    required: ["questions"],
  },
  execute: handleAskUserPhone,
};

// ---------------------------------------------------------------------------
// 拡張機能の入口
// ---------------------------------------------------------------------------

export default function (pi: ExtensionAPI): void {
  pi.registerTool({ ...askUserPhoneTool, execute: (id, params, signal, onUpdate, ctx) =>
    handleAskUserPhone(id, params, signal, onUpdate, ctx) });
  // セッション開始時に登録する（01 の対応づけ用．失敗しても動く）
  pi.on("session_start", (_event, ctx) => {
    try {
      const file = ctx.sessionManager?.getSessionFile?.() ?? "";
      const sid = ctx.sessionManager?.getSessionId?.() ?? file;
      const pane = typeof process !== "undefined" ? (process.env.HERDR_PANE_ID ?? "") : "";
      if (!sid || !pane) return;
      void fetch(`${SAI_WEB_URL}/api/register`, {
        method: "POST",
        headers: { "Content-Type": "application/json", ...authHeaders() },
        body: JSON.stringify({
          agent: "pi",
          session_id: sid,
          transcript_path: file,
          cwd: ctx.cwd ?? "",
          pane_id: pane,
        }),
      }).catch(() => {});
    } catch {
      // 登録できなくても pi は普通に動く
    }
  });
  console.log("[ask_user] Registered tool: ask_user");
}

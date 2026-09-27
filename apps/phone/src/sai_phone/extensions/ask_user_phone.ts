/**
 * ask_user_phone.ts — pi 用の質問ツール（sai-phone 連携付き）.
 *
 * 元ファイル: ~/workdir/Pi/my-extension/ask_user_question/index.ts（152行）
 * （元ファイルは編集せず，ここにコピーして拡張した）
 *
 * cc の AskUserQuestion と同じ書式で質問を受け付け，
 * 端末に選択画面を出すと同時に sai-phone サーバーにも登録する．
 * iPhone と端末のどちらで答えても同じ結果になる．
 * 先に答えた方が勝ち，もう片方の画面は自動で消える．
 *
 * 変更点:
 * - 質問は最大4問（元は1問のみ）
 * - 選択肢は label と description を持つ物の配列（元は文字列の配列）
 * - header（短い見出し）と multiSelect（複数選択）に対応
 * - sai-phone への登録と，外から閉じる仕組み（AbortSignal）を追加
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
 */

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

// sai-phone サーバーの場所．家の中だけの接続（Tailscale 前提）．
const SAI_PHONE_URL = "http://127.0.0.1:8200";
// iPhone の回答を待つ間隔（ミリ秒）と上限回数（約5分）
const POLL_INTERVAL_MS = 1000;
const POLL_MAX_COUNT = 300;

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
// sai-phone サーバーとのやり取り（失敗しても端末だけで完結できるようにする）
// ---------------------------------------------------------------------------

async function registerToServer(payload: Record<string, unknown>): Promise<string | null> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 3000);
    const res = await fetch(`${SAI_PHONE_URL}/api/questions`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
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
    await fetch(`${SAI_PHONE_URL}/api/questions/${questionId}/resolved`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
      signal: controller.signal,
    });
    clearTimeout(timer);
  } catch {
    // 伝えられなくても pi の動作には影響させない
  }
}

// ---------------------------------------------------------------------------
// herdr への「質問待ち」の合図（05 §3.1）
//
// pi の質問中は herdr の一覧・プッシュ通知で「質問待ち（blocked）」が分かる
// ようにする。herdr-agent-state.ts は pi.events.emit("herdr:blocked") を
// 聞いているだけ（カウント方式なので1回 emit し，どの場合も必ず解除する）。
// ---------------------------------------------------------------------------

function setBlocked(pi: ExtensionAPI | undefined, active: boolean, label = "質問"): void {
  try {
    pi?.events?.emit("herdr:blocked", active ? { active: true, label } : { active: false });
  } catch {
    // 合図に失敗しても質問自体は続けさせる
  }
}

async function pollAnswer(questionId: string): Promise<Record<string, string> | null> {
  for (let i = 0; i < POLL_MAX_COUNT; i++) {
    await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
    try {
      const res = await fetch(`${SAI_PHONE_URL}/api/questions/${questionId}`);
      if (!res.ok) continue;
      const data = (await res.json()) as {
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
      };
      if (data.state === "answered" && data.answer) {
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
  pi?: ExtensionAPI,
): Promise<{ content: Array<{ type: string; text?: string }>; isError?: boolean; details?: Record<string, unknown> }> {
  const questions = normalizeQuestions(params);
  if (questions.length === 0) {
    return errorResult("質問がありません", { answers: null });
  }
  if (!ctx?.hasUI || !ctx?.ui || ctx.mode === "headless") {
    return errorResult("UI is unavailable in this mode.", { answers: null });
  }
  const ui = ctx.ui;

  // 質問ID：サーバー登録後に差し替える（登録前から一意にしておく）
  const localId = `local-${Date.now()}`;
  const paneId = typeof process !== "undefined" ? (process.env.HERDR_PANE_ID ?? "") : "";
  // セッション ID は API から取る（00 §3.7 の対応づけのため）。
  // 取れない時だけファイルパスにフォールバックするが，その場合
  // 実行と質問が対応づけられないためログに出す。
  const sessionId = ctx.sessionManager?.getSessionId?.() ?? "";
  const sessionFile = ctx.sessionManager?.getSessionFile?.() ?? "";
  if (!sessionId && sessionFile) {
    console.warn(
      "[ask_user_phone] getSessionId() を取得できないためファイルパスで登録します（実行と質問の対応づけはできません）:",
      sessionFile,
    );
  }

  // 質問中は herdr で「質問待ち」と分かるようにする（05 §3.1）。
  // 答えた・取り消した・エラーのどの場合でも必ず解除する。
  setBlocked(pi, true);
  try {
    // sai-phone に登録する（失敗しても端末だけで続ける）
    const serverId = await registerToServer({
      agent: "pi",
      session_id: sessionId || sessionFile,
      pane_id: paneId,
      tool_use_id: localId,
      questions,
    });

    // この段階では1問ずつ順に聞く．複数問・複数選択の同時操作は
    // 端末画面の仕様に依存し壊れやすいため，順番に1問ずつ出す割り切り．
    // iPhone 側は全問まとめて1回で答えてくるため，届いた時点で全問揃っていれば
    // 端末側の残りの質問は出さずに打ち切る．
    const answers: Record<string, string> = {};
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
      let phoneAnswer: Promise<Record<string, string> | null> | null = null;
      if (serverId) {
        phoneAnswer = pollAnswer(serverId).then((a) => {
          if (a) controller.abort();
          return a;
        });
      }

      const selected = await ui.select(title, selectable, { signal: controller.signal });

      if (selected) {
        // 端末で選ばれた → サーバーに伝えて iPhone のカードを「PC で回答済み」に
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
          for (const [k, v] of Object.entries(a)) answers[k] = v;
        } else {
          return result("User cancelled.", { answers: null });
        }
      } else {
        // 利用者が Esc で取り消した
        return result("User cancelled.", { answers: null });
      }
    }

    return result(answeredText(answers), { answers });
  } finally {
    // 答えた・取り消した・エラーのどの場合でも「質問待ち」を解除する（05 §3.1）
    setBlocked(pi, false);
  }
}

// ---------------------------------------------------------------------------
// ツール定義（typebox 形式．Google のモデルとの互換性のため StringEnum 相当の
//  文字列 enum を使う．ここでは外部依存なしの素の定義で同じ形にする）
// ---------------------------------------------------------------------------

const askUserPhoneTool: ToolInput = {
  name: "ask_user_phone",
  label: "Ask User Phone",
  description:
    "Ask the user a question with predefined choices. " +
    "Use ask_user_phone when you need clarification, a decision, or additional input. " +
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
    handleAskUserPhone(id, params, signal, onUpdate, ctx, pi) });
  // セッション開始時に登録する（01 の対応づけ用．失敗しても動く）
  pi.on("session_start", (_event, ctx) => {
    try {
      const file = ctx.sessionManager?.getSessionFile?.() ?? "";
      const sid = ctx.sessionManager?.getSessionId?.() ?? file;
      const pane = typeof process !== "undefined" ? (process.env.HERDR_PANE_ID ?? "") : "";
      if (!sid || !pane) return;
      void fetch(`${SAI_PHONE_URL}/api/register`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
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
  console.log("[ask_user_phone] Registered tool: ask_user_phone");
}

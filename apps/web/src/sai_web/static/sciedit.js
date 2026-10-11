"use strict";
// research-tree の操作の入力欄・カードのフォーム（実装計画 U19・C 契約の U06 版）。
//
// 板の釦は scitree.js から次の形で呼ばれる（この関数が無ければその釦は出さない）。
//   seOpenAction(op, nodeId, detail)   入力欄のある op。detail は node の JSON
//   seQuick(op, payload)               ★・stance は入力欄を出さずに送る
//   seOpenCard(nodeId, preset)         preset は C-CARD。値の入ったフォームを開く
//
// 中の処理は `POST …/sci/apply` / `…/sci/cards` だけを使う。
// この画面はファイルのパスを受け取らない。送る前に画面でも必須の項目と字数と出典を確かめる。
// 通らなければ送らず、小窓の中に理由を日本語で出す。最後の検査はサーバとコアが行う。
//
// DOM を使わない純関数（seBuildOp / seBuildCard / seCardValues / seChildOptions など）は
// 末尾で node に渡す（tests/test_sciedit.js）。
// 一番外側の名前は se で始める（tests/test_globals.js）。

// --- 定数 ---

// op の表示名（表に無い op はそのまま出す）
const SE_OPS = {
  add_q: "Add Q", add_a: "Add A", add_f: "Add F", add_e: "Add E",
  add_seed: "seed（D）を作る", add_sprout: "Create SPROUT", copy: "Copy A", remove: "Delete",
  star: "★ を付ける", unstar: "★ を外す", set_stance: "Stance を変える",
  edit: "Edit", insert_q: "Q を間に入れる", move_a: "A を移す", set_mlflow: "MLflow を変える", set_dataset: "データセットを変える",
  move_node: "A を 1 件移す", regroup: "まとめる（中間の A と細分の Q）",
  add_seed_q: "問いを足す", edit_seed_q: "問いを直す", remove_seed_q: "問いを消す",
  add_answer: "回答を足す", edit_answer: "回答を直す", remove_answer: "回答を消す",
};
// 反映したときに知らせる言葉（op ごとに 1 つ）
const SE_DONE = {
  add_q: "Q を足しました", add_a: "A を足しました", add_f: "F を足しました", add_e: "E を足しました",
  add_seed: "seed（D）を作りました", add_sprout: "Created SPROUT", copy: "A を写しました", remove: "消しました",
  star: "★ を付けました", unstar: "★ を外しました", set_stance: "Stance を変えました",
  edit: "文を直しました", insert_q: "Q を間に入れました", move_a: "A を移しました", set_mlflow: "MLflow を変えました", set_dataset: "データセットを変えました",
  move_node: "A を 1 件移しました", regroup: "まとめました",
  add_seed_q: "問いを足しました", edit_seed_q: "問いを直しました", remove_seed_q: "問いを消しました",
  add_answer: "回答を足しました", edit_answer: "回答を直しました", remove_answer: "回答を消しました",
};
// 入力欄を出さずに送る op
const SE_QUICK = ["star", "unstar", "set_stance"];
// 読む深さ
const SE_DEPTHS = [["abstract", "abstract"], ["figures", "figures"], ["full", "full"]];
// 実行するもの（user は READ_CARD・pi は廃止）
const SE_RUNNERS = [["cc", "cc"], ["user", "自分で読む（READ_CARD）"]];
// CARD の model・thinking（空は block の既定。値は API の CARD_MODELS・CARD_THINKING）
const SE_CARD_MODELS = [["", "既定（block の値）"], ["opus", "opus"], ["sonnet", "sonnet"], ["haiku", "haiku"]];
const SE_CARD_THINKING = [["", "既定（block の値）"], ["none", "none"], ["low", "low"], ["medium", "medium"], ["high", "high"], ["xhigh", "xhigh"], ["max", "max"]];
// READ_CARD の型
const SE_MODES = [["purpose_first", "purpose_first"], ["blind_first", "blind_first"]];
// F の stance（S・D・I）
// 値は API の STANCES（support・deny・issue）。表示は略号つき。
const SE_STANCES = [["support", "S 支持"], ["deny", "D 反対"], ["issue", "I 問い"]];
// カードの子の型（対象の種類ごとに出す。空は「なし」）
const SE_CHILD_KINDS = { Q: [["Q", "Q"], ["A", "A"]], A: [["Q", "Q"], ["F", "F"]] };
// カードで選べるブロック（Next Action に出るもの）
const SE_BLOCKS = [
  ["sci-start", "研究の入口: 最初の Q を立てる"],
  ["sci-restore", "戻す: 消した Node を探して戻す案を出す"],
  ["sci-cite-check", "出典を確かめる: F の出典を照らす"],
  ["sci-digest", "まとめる: 今の状態をまとめる（op は出さない）"],
  ["sci-coach", "理解の確認: 問いかけて確かめる"],
  ["sci-typify", "型に当てる: 型を当てはめる"],
  ["sci-exp-sprout", "実験を始める: SPROUT から実験仕様書と最初の BRANCH を作る"],
  ["sci-exp-branch", "実験を分ける: BRANCH する"],
  ["sci-exp-review", "REVIEW: 実験の結果を点検する"],
  ["sci-result-compare", "結果を比べる: 実験の結果を比べる"],
  ["sci-data-explore", "データを調べる: データ表・EDA・前処理"],
  ["sci-data-test", "データで確かめる: 計画どおりに検定して報告"],
  ["lab-read", "1 本を読む（論文あり）: 論文から F を抜き出す"],
  ["lab-search", "探して読む（検索）: 論文を探し、候補から取り込む"],
  ["lab-grow", "木を育てる（論文あり）: A・F・Q を足し、Q を間に入れる"],
  ["lab-survey", "外を調べる（検索）: 論文を探して木の案を出す"],
  ["lab-place", "置き場所を決める（論文だけ）: 木を探して読む CARD を作る"],
  ["lab-map", "root を組む（review 論文）: 分野の軸ごとに Q・A・F の骨組みを作る"],
  ["lab-sprout", "SPROUT を作る: seed（D）と path を選び、制約と先行研究との違いを確かめて S を作る"],
  ["lab-progress", "進み具合の確認: 数と穴を報告し、次の CARD を作る（op は出さない）"],
  ["sci-verify", "検証: target_node と子の関係を評価し、報告・op・次の CARD を作る"],
];
// 出典の形（C-CITE。`#37`・`#37.3.2`・`#37.3.2-4 注`）
const SE_CITE_RE = /^#([1-9][0-9]{0,8})((?:\.[1-9][0-9]*){1,2}(?:-[1-9][0-9]*)?)?( .{1,80})?$/;
// ID の形（Q・A・F・E・S・D）
const SE_ID_RE = /^[QAFESD][1-9][0-9]{0,8}$/;
// seed の問い（SQ）・回答（SA）の ID（U01）
const SE_SQ_RE = /^SQ[1-9][0-9]{0,8}$/;
const SE_SA_RE = /^SA[1-9][0-9]{0,8}$/;
// seed の問いの軸（並びは画面の選びの順。空は「なし」）
const SE_SEED_AXES = [
  ["task", "解く問題（task）"],
  ["target", "対象（target）"],
  ["data", "データ（data）"],
  ["method", "手法（method）"],
  ["output", "出力（output）"],
  ["metric", "評価指標（metric）"],
];
// 文字数の上限
const SE_MAX = { qtext: 300, atext: 120, ftext: 1200, etext: 120, body: 2000, mlflow: 200, dataset: 200, title: 120, purpose: 300, expect: 300 };

// ============================================================================
// DOM を使わない純関数（tests/test_sciedit.js）
// ============================================================================

/** 前後の空白を落とした文字（null は空文字）。 */
function seTrim(value) {
  return value == null ? "" : String(value).trim();
}

/** op の表示名。表に無い op はそのまま返す。 */
function seOpLabel(op) {
  const name = seTrim(op);
  return SE_OPS[name] || name;
}

/** 出典の形か。 */
function seCiteOk(cite) {
  return SE_CITE_RE.test(seTrim(cite));
}

/** ID の形か。prefix を渡すと、その頭の文字（"QO" のように複数可）だけ受け付ける。 */
function seRefOk(value, prefix) {
  const v = seTrim(value);
  return SE_ID_RE.test(v) && (!prefix || prefix.includes(v[0]));
}

/** 問い（SQ）・回答（SA）の ID の形か。re は SE_SQ_RE か SE_SA_RE。 */
function seIdOk(value, re) {
  return re.test(seTrim(value));
}

/** 文の数を数える（`0.85` や `§3.2` の「.」は数えない。0 なら 1 文）。 */
function seCountSentences(text) {
  const s = seTrim(text);
  if (!s) return 0;
  const jp = (s.match(/[。？！]/g) || []).length;
  const en = (s.match(/[.?!](?=\s|$)/g) || []).length;
  return jp + en || 1;
}

/** 数字を含み出典も報告書も無いとき true（C3: 数字の文は cites か report が要る）。 */
function seNeedsCite(text, cites, report) {
  const list = (Array.isArray(cites) ? cites : []).filter((c) => seTrim(c) !== "");
  if (list.length || seTrim(report)) return false;
  return /[0-9]/.test(seTrim(text));
}

/** 編集の字数と本文の可否（C3）。Q は 300 字・本文なし、F は 1200 字・本文なし、A・E・S は 120 字・本文あり。 */
function seEditRules(nodeId) {
  const k = seTrim(nodeId).charAt(0);
  if (k === "Q") return { text: SE_MAX.qtext, body: false };
  if (k === "F") return { text: SE_MAX.ftext, body: false };
  return { text: SE_MAX.atext, body: true };
}

/** 文字の配列（空を落とす）。 */
function seList(value) {
  return (Array.isArray(value) ? value : []).map(seTrim).filter(Boolean);
}

/**
 * 小窓の値（values）から op の JSON を作る。
 *   → { ok: true, op: {...}, json: {...}, error: null } または { ok: false, op: null, json: null, error: "理由" }
 * 通らないものは日本語で理由を返す（画面では送らない）。
 */
function seBuildOp(kind, values) {
  const v = values || {};
  const name = seTrim(kind);
  const bad = (error) => ({ ok: false, op: null, json: null, error });
  const good = (op) => ({ ok: true, op, json: op, error: null });
  // 必須の ID を確かめる。prefix は許す頭の文字
  const id = (key, prefix, label) => (seRefOk(v[key], prefix) ? "" : `${label}を選んでください`);
  // 字数を確かめる。min が 1 なら必須
  const len = (key, label, min, max) => {
    const n = seTrim(v[key]).length;
    if (n < min) return `${label}を入力してください`;
    if (n > max) return `${label}は ${max} 字までです`;
    return "";
  };
  const first = (...msgs) => msgs.find(Boolean) || "";

  switch (name) {
    case "add_q": {
      // 軸の Q は sai-sci init が作るので、Q を足せるのは Q・A の下だけ
      if (seTrim(v.parent) === "SEED") return bad("軸の Q は sai-sci init が作ります");
      const msg = first(seRefOk(v.parent, "QA") ? "" : "親（Q・A）を選んでください", len("text", "文", 1, SE_MAX.qtext));
      if (msg) return bad(msg);
      return good({ op: name, parent: seTrim(v.parent), text: seTrim(v.text) });
    }
    case "add_a": {
      const msg = first(id("q", "Q", "Q"), len("text", "文", 1, SE_MAX.atext), len("body", "本文", 0, SE_MAX.body));
      if (msg) return bad(msg);
      const op = { op: name, q: seTrim(v.q), text: seTrim(v.text) };
      if (seTrim(v.body)) op.body = seTrim(v.body);
      return good(op);
    }
    case "add_f": {
      // F を付けられるのは A だけ（E の下に F は置かない。結果は E の mlflow で結ぶ）
      const msg = first(id("parent", "A", "親（A）"), len("text", "文", 1, SE_MAX.ftext));
      if (msg) return bad(msg);
      const stance = seTrim(v.stance);
      if (!SE_STANCES.some((x) => x[0] === stance)) return bad("stance（S・D・I）を選んでください");
      const text = seTrim(v.text);
      if (seCountSentences(text) > 3) return bad("文は 1〜3 文にしてください");
      const cites = seList(v.cites);
      if (cites.length > 10) return bad("出典は 10 個までです");
      for (const c of cites) if (!seCiteOk(c)) return bad(`出典「${c}」の形が正しくありません（#37 §3.2）`);
      const report = seTrim(v.report);
      if (report.length > 100) return bad("報告書は 100 字までです");
      if (seNeedsCite(text, cites, report)) return bad("数字を含む F には出典か報告書が要ります");
      const op = { op: name, parent: seTrim(v.parent), text, stance };
      if (cites.length) op.cites = cites;
      const paper = seTrim(v.paper);
      if (paper) {
        if (!/^[1-9][0-9]{0,8}$/.test(paper)) return bad("論文番号は数字で入れてください");
        op.paper = Number(paper);
      }
      if (report) op.report = report;
      return good(op);
    }
    case "add_e": {
      const msg = first(id("parent", "SE", "親（S か E）"), len("text", "文", 1, SE_MAX.etext), len("body", "本文", 0, SE_MAX.body), len("mlflow", "MLflow の名前", 0, SE_MAX.mlflow), len("dataset", "データセット", 0, SE_MAX.dataset));
      if (msg) return bad(msg);
      const op = { op: name, parent: seTrim(v.parent), text: seTrim(v.text) };
      if (seTrim(v.body)) op.body = seTrim(v.body);
      if (seTrim(v.mlflow)) op.mlflow = seTrim(v.mlflow);
      if (seTrim(v.dataset)) op.dataset = seTrim(v.dataset);
      return good(op);
    }
    case "add_seed": {
      // 題と本文だけ（問いは雛形の 7 問が入る）。字数は送る前に確かめる
      const msg = len("title", "題", 1, SE_MAX.title);
      if (msg) return bad(msg);
      const body = seTrim(v.body);
      if (body.length > SE_MAX.body) return bad(`本文が ${SE_MAX.body} 字を超えています（今 ${body.length} 字）`);
      const op = { op: name, title: seTrim(v.title) };
      if (body) op.body = body;
      return good(op);
    }
    case "add_sprout": {
      // 1 つの SPROUT は 1 つの seed（D）の下に作る。回答は作った後に S の板で足す（paths は無い）
      if (!seRefOk(v.seed, "D")) return bad("seed（D）を選んでください");
      const msg = first(len("title", "題", 1, SE_MAX.title), len("body", "本文", 0, SE_MAX.body));
      if (msg) return bad(msg);
      const op = { op: name, seed: seTrim(v.seed), title: seTrim(v.title) };
      if (seTrim(v.body)) op.body = seTrim(v.body);
      return good(op);
    }
    case "add_seed_q": {
      // 問いは seed（D）に足す。軸は空なら付けない
      if (!seRefOk(v.seed, "D")) return bad("seed（D）を選んでください");
      const msg = len("text", "問い", 1, SE_MAX.qtext);
      if (msg) return bad(msg);
      const axis = seTrim(v.axis);
      if (axis && !SE_SEED_AXES.some((x) => x[0] === axis)) return bad("軸が正しくありません");
      const op = { op: name, seed: seTrim(v.seed), text: seTrim(v.text) };
      if (axis) op.axis = axis;
      return good(op);
    }
    case "edit_seed_q": {
      // 軸を「なし」にするときは axis が空文字で送る（サーバが外す）
      if (!seIdOk(v.q, SE_SQ_RE)) return bad("問いを選んでください");
      const msg = len("text", "問い", 1, SE_MAX.qtext);
      if (msg) return bad(msg);
      const axis = seTrim(v.axis);
      if (axis && !SE_SEED_AXES.some((x) => x[0] === axis)) return bad("軸が正しくありません");
      return good({ op: name, q: seTrim(v.q), text: seTrim(v.text), axis });
    }
    case "remove_seed_q": {
      return seIdOk(v.q, SE_SQ_RE) ? good({ op: name, q: seTrim(v.q) }) : bad("問いを選んでください");
    }
    case "add_answer": {
      // 回答は持ち主（D か S）の問いに、A（path）かテキストのどちらか一方
      if (!seRefOk(v.to, "DS")) return bad("回答の持ち主（D か S）を選んでください");
      if (!seIdOk(v.q, SE_SQ_RE)) return bad("問いを選んでください");
      const hasA = seTrim(v.a) !== "";
      const hasText = seTrim(v.text) !== "";
      if (hasA === hasText) return bad("A を選ぶか、文を入力するかのどちらか一方にしてください");
      const op = { op: name, to: seTrim(v.to), q: seTrim(v.q) };
      if (hasA) {
        if (!seRefOk(v.a, "A")) return bad(`A の形が正しくありません: ${seTrim(v.a)}`);
        op.a = seTrim(v.a);
      } else {
        const msg = len("text", "回答の文", 1, SE_MAX.qtext);
        if (msg) return bad(msg);
        op.text = seTrim(v.text);
      }
      return good(op);
    }
    case "edit_answer": {
      if (!seIdOk(v.answer, SE_SA_RE)) return bad("回答を選んでください");
      const msg = len("text", "回答の文", 1, SE_MAX.qtext);
      return msg ? bad(msg) : good({ op: name, answer: seTrim(v.answer), text: seTrim(v.text) });
    }
    case "remove_answer": {
      return seIdOk(v.answer, SE_SA_RE) ? good({ op: name, answer: seTrim(v.answer) }) : bad("回答を選んでください");
    }
    case "copy": {
      const msg = first(id("node", "A", "写す A"), id("to", "Q", "写す先の Q"));
      return msg ? bad(msg) : good({ op: name, node: seTrim(v.node), to: seTrim(v.to) });
    }
    case "remove": {
      const msg = id("node", "QAFESD", "消す Node");
      return msg ? bad(msg) : good({ op: name, node: seTrim(v.node) });
    }
    case "star":
    case "unstar": {
      const msg = id("node", "QAFESD", "Node");
      return msg ? bad(msg) : good({ op: name, node: seTrim(v.node) });
    }
    case "set_stance": {
      const msg = id("node", "F", "F");
      if (msg) return bad(msg);
      const stance = seTrim(v.stance);
      if (!SE_STANCES.some((x) => x[0] === stance)) return bad("stance は S・D・I のどれかです");
      return good({ op: name, node: seTrim(v.node), stance });
    }
    case "insert_q": {
      const msg = first(id("q", "Q", "Q"), len("text", "文", 1, SE_MAX.qtext));
      return msg ? bad(msg) : good({ op: name, q: seTrim(v.q), text: seTrim(v.text) });
    }
    case "move_a": {
      // from の A の子を全部 to へ移す（C3）。to は生きた child_Q を持たない Q
      const msg = first(id("from", "Q", "移す A を持つ Q"), id("to", "Q", "移す先の Q"));
      if (msg) return bad(msg);
      if (seTrim(v.from) === seTrim(v.to)) return bad("同じ Q へは移せません");
      return good({ op: name, from: seTrim(v.from), to: seTrim(v.to) });
    }
    case "move_node": {
      // A を 1 件、その下の F・child_Q ごと、ID を保って別の Q の下へ移す（C-OP-2）。移す先は child_Q を持たない Q
      const msg = first(id("node", "A", "移す A"), id("to", "Q", "移す先の Q"));
      return msg ? bad(msg) : good({ op: name, node: seTrim(v.node), to: seTrim(v.to) });
    }
    case "regroup": {
      // Q の直下の A（members）を、新しい中間の A の下の細分の Q へまとめる（C-OP-3。ID は保つ）
      const msg = first(
        id("q", "Q", "まとめる Q"),
        len("text", "中間の A の文", 1, 120),
        len("body", "中間の A の本文", 0, SE_MAX.body),
        len("child_q", "細分の Q の文", 1, SE_MAX.qtext),
        len("child_q_body", "細分の Q の本文", 0, SE_MAX.body),
      );
      if (msg) return bad(msg);
      const members = seList(v.members);
      if (members.length < 1) return bad("まとめる A を 1 つ以上選んでください");
      if (members.length > 20) return bad("まとめる A は 20 個までです");
      for (const a of members) if (!seRefOk(a, "A")) return bad(`A の形が正しくありません: ${a}`);
      if (new Set(members).size !== members.length) return bad("同じ A を 2 回選んでいます");
      const op = { op: name, q: seTrim(v.q), text: seTrim(v.text), child_q: seTrim(v.child_q), members };
      if (seTrim(v.body)) op.body = seTrim(v.body);
      if (seTrim(v.child_q_body)) op.child_q_body = seTrim(v.child_q_body);
      return good(op);
    }
    case "set_mlflow": {
      // 空文字は外す（API の ztext）
      const msg = first(id("node", "E", "E"), len("mlflow", "MLflow の名前", 0, SE_MAX.mlflow));
      if (msg) return bad(msg);
      return good({ op: name, node: seTrim(v.node), mlflow: seTrim(v.mlflow) });
    }
    case "edit": {
      const rules = seEditRules(v.node);
      const msg = first(
        id("node", "QAFESD", "直す Node"),
        len("text", "文", 0, rules.text),
        rules.body ? len("body", "本文", 0, SE_MAX.body) : "",
      );
      if (msg) return bad(msg);
      if (!rules.body && seTrim(v.body)) return bad("本文は A・E・S だけに付けられます");
      if (!seTrim(v.text) && !seTrim(v.body)) return bad("文か本文のどちらかを入力してください");
      if (seTrim(v.node).charAt(0) === "F" && seCountSentences(v.text) > 3) return bad("文は 1〜3 文にしてください");
      const op = { op: name, node: seTrim(v.node) };
      if (seTrim(v.text)) op.text = seTrim(v.text);
      if (seTrim(v.body)) op.body = seTrim(v.body);
      return good(op);
    }
    default:
      return bad("Unknown operation");
  }
}

/** 対象の種類で選べる子の型（[値, 表示]）。Q は Q か A、A は Q か F、それ以外は無い。 */
function seChildOptions(nodeId) {
  return SE_CHILD_KINDS[seTrim(nodeId).charAt(0)] || [];
}

/** 足す子の型が対象の Node に合うか（空は未指定なので合う）。 */
function seChildOk(nodeId, child) {
  const c = seTrim(child);
  if (!c) return true;
  return seChildOptions(nodeId).some((x) => x[0] === c);
}

/**
 * カードの対象の Node。空は木の全体（CLI に --node を渡さない）。
 * 旧い画面が木の全体を表す印（SEED）で渡すときも空として扱う（古い住所の互換）。
 */
function seCardNode(value) {
  const v = seTrim(value);
  return v === "SEED" ? "" : v;
}

/**
 * カードの初めの値（C-CARD の preset から）。無い欄は空。
 * runner が user でブロックが空なら、ブロックの既定は lab-read。
 */
function seCardValues(preset, nodeId) {
  const p = preset || {};
  const runner = SE_RUNNERS.some((r) => r[0] === p.runner) ? p.runner : "cc";
  let block = SE_BLOCKS.some((b) => b[0] === p.block) ? p.block : "";
  if (runner === "user" && !block) block = "lab-read";
  const node = seCardNode(nodeId) || seCardNode(p.node);
  const child = seTrim(p.child);
  return {
    node,
    block,
    purpose: seTrim(p.purpose),
    expect: seTrim(p.expect),
    runner,
    mode: SE_MODES.some((m) => m[0] === p.mode) ? p.mode : "purpose_first",
    model: SE_CARD_MODELS.some((m) => m[0] === p.model) ? p.model : "",
    thinking: SE_CARD_THINKING.some((t) => t[0] === p.thinking) ? p.thinking : "",
    papers: (Array.isArray(p.papers) ? p.papers : []).map(Number).filter((n) => Number.isInteger(n) && n > 0),
    depth: SE_DEPTHS.some((d) => d[0] === p.depth) ? p.depth : "full",
    child: seChildOk(node, child) ? child : "",
  };
}

/**
 * カードのフォームの値を確かめて、送る JSON にする。
 *   → { ok: true, card: {...}, error: null } または { ok: false, card: null, error: "理由" }
 * papers は番号の配列か、番号を書いた文字の配列（打ち込み）。
 */
function seBuildCard(values) {
  const v = values || {};
  const bad = (error) => ({ ok: false, card: null, error });
  // 空は木の全体（node を送らない）
  const node = seCardNode(v.node);
  if (node && !seRefOk(node)) return bad("Node を選んでください");
  if (!SE_BLOCKS.some((b) => b[0] === v.block)) return bad("ブロックを選んでください");
  const purpose = seTrim(v.purpose);
  if (!purpose) return bad("目的を入力してください（空の目的では作りません）");
  if (purpose.length > SE_MAX.purpose) return bad("目的は 300 字までです");
  const expect = seTrim(v.expect);
  if (expect.length > SE_MAX.expect) return bad("期待する成果は 300 字までです");
  const runner = SE_RUNNERS.some((r) => r[0] === v.runner) ? v.runner : "cc";
  const papers = [];
  for (const raw of Array.isArray(v.papers) ? v.papers : []) {
    const s = seTrim(raw).replace(/^#/, "");
    if (!s) continue;
    if (!/^[1-9][0-9]{0,8}$/.test(s)) return bad(`論文の番号「${seTrim(raw)}」が正しくありません`);
    if (!papers.includes(Number(s))) papers.push(Number(s));
  }
  if (papers.length > 20) return bad("論文は 20 本までです");
  const child = seTrim(v.child);
  if (!seChildOk(node, child)) return bad("足す子の型は、対象が Q なら Q か A、A なら Q か F です");
  const card = { block: v.block, purpose, runner, papers };
  if (node) card.node = node;
  if (runner === "user") {
    if (!/^[QA]/.test(node)) return bad("READ_CARD の対象は Q か A にしてください");
    if (papers.length !== 1) return bad("READ_CARD の論文はちょうど 1 本にしてください");
    card.depth = SE_DEPTHS.some((d) => d[0] === v.depth) ? v.depth : "full";
  }
  if (runner === "cc") {
    if (SE_CARD_MODELS.some((m) => m[0] === v.model && m[0])) card.model = v.model;
    if (SE_CARD_THINKING.some((t) => t[0] === v.thinking && t[0])) card.thinking = v.thinking;
  }
  if (expect) card.expect = expect;
  if (child) card.child = child;
  if (v.block === "lab-read" && seTrim(v.mode)) card.mode = seTrim(v.mode);
  return { ok: true, card, error: null };
}

/** カードを作った応答から run の番号を読む。 */
function seCardRunId(res) {
  if (!res || typeof res !== "object") return null;
  if (res.run_id != null) return res.run_id;
  if (res.id != null) return res.id;
  if (res.run && res.run.id != null) return res.run.id;
  if (res.run && res.run.run_id != null) return res.run.run_id;
  return null;
}

// ============================================================================
// 小さい DOM を作る手（ui.js の saiEl と同じ形）
// ============================================================================

function seEl(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function seBtn(label, onClick, cls) {
  const b = seEl("button", `sai-btn ${cls || ""}`.trim(), label);
  b.type = "button";
  if (typeof onClick === "function") b.addEventListener("click", onClick);
  else b.disabled = true;
  return b;
}

function seChip(text, tone) {
  return seEl("span", `sai-chip ${tone || "muted"}`, text);
}

function seLine(cls) {
  return seEl("div", `sci-edit-line ${cls || ""}`.trim());
}

function seNote(text) {
  return seEl("p", "sci-edit-note", text || "");
}

/** Node の題を status から引く。 */
// ============================================================================
// 入力欄の部品（1 種類ごとに 1 つの関数）
// ============================================================================

/** 1 行の入力。 */
function seFieldText(key, label, init, hint, placeholder) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const input = document.createElement("input");
  input.type = "text";
  input.className = "sai-input sci-edit-input";
  input.value = init == null ? "" : String(init);
  if (placeholder) input.placeholder = placeholder;
  el.appendChild(input);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => input.value, set: (x) => { input.value = x; } };
}

/** 大きな入力。 */
function seFieldArea(key, label, init, rows, hint, placeholder) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const area = document.createElement("textarea");
  area.className = "sai-input sci-edit-area";
  area.rows = rows || 4;
  area.value = init == null ? "" : String(init);
  if (placeholder) area.placeholder = placeholder;
  el.appendChild(area);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => area.value, set: (x) => { area.value = x; } };
}

/** 1 つ選ぶ（allowEmpty で「なし」を付ける）。 */
function seFieldSelect(key, label, options, init, hint, allowEmpty) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const sel = document.createElement("select");
  sel.className = "sai-input sci-edit-select";
  if (allowEmpty) {
    const none = document.createElement("option");
    none.value = "";
    none.textContent = "なし";
    sel.appendChild(none);
  }
  const list = Array.isArray(options) ? options : [];
  for (const [value, text] of list) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    sel.appendChild(o);
  }
  const want = seTrim(init);
  sel.value = want;
  if (sel.value !== want) sel.value = allowEmpty || !list.length ? "" : String(list[0][0]);
  el.appendChild(sel);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => sel.value, set: (x) => { sel.value = x; } };
}

/** 1 つだけ選ぶ（押して切り替える）。 */
function seFieldPick(key, label, options, init) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const box = seEl("div", "sci-edit-picks");
  const buttons = [];
  for (const [value, text] of options || []) {
    const b = seEl("button", `sci-edit-pick${seTrim(init) === value ? " active" : ""}`, text);
    b.type = "button";
    b.dataset.value = value;
    b.addEventListener("click", () => {
      for (const x of buttons) x.el.classList.toggle("active", x.value === value);
      // 1 つ選んだことが分かるように change を起こす（ほかの欄の表示を切り替えるため）
      b.dispatchEvent(new Event("change", { bubbles: true }));
    });
    buttons.push({ value, el: b });
    box.appendChild(b);
  }
  el.appendChild(box);
  const read = () => (buttons.find((x) => x.el.classList.contains("active")) || {}).value || "";
  return { key, el, read, refresh: () => el.classList.toggle("sci-edit-picked", !!read()) };
}

/** 複数選ぶ。 */
function seFieldChecks(key, label, options, init, hint) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const box = seEl("div", "sci-edit-checks");
  const chosen = new Set((Array.isArray(init) ? init : []).map(String));
  const list = Array.isArray(options) ? options : [];
  for (const [value, text] of list) {
    const row = seEl("label", "sci-edit-check");
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = chosen.has(String(value));
    row.appendChild(input);
    row.appendChild(seEl("span", null, text));
    box.appendChild(row);
  }
  el.appendChild(box);
  if (hint) el.appendChild(seNote(hint));
  return {
    key,
    el,
    read: () => [...box.querySelectorAll("input")].flatMap((c, i) => (c.checked ? [String(list[i][0])] : [])),
  };
}

/** 1 行ずつ何個も足す欄（出典・仮置き）。 */
function seFieldLines(key, label, init, max, hint, placeholder) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const box = seEl("div", "sci-edit-lines");
  const inputs = [];
  const push = (value) => {
    const input = document.createElement("input");
    input.type = "text";
    input.className = "sai-input sci-edit-input";
    input.value = value == null ? "" : String(value);
    if (placeholder) input.placeholder = placeholder;
    box.appendChild(input);
    inputs.push(input);
  };
  const start = (Array.isArray(init) ? init : []).filter((x) => seTrim(x) !== "");
  if (start.length) for (const s of start) push(s);
  else push("");
  const add = seBtn("Add a line", () => {
    if (inputs.length >= max) return;
    push("");
    add.disabled = inputs.length >= max;
  }, "ghost small");
  el.appendChild(box);
  el.appendChild(add);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => inputs.map((i) => i.value) };
}

/**
 * 足す子の型（CARD の child）。対象の Node の種類で選べる物が変わるので、setNode で選びを作り直す。
 * 選べる物が無ければ欄ごと隠す（読むと空になり、送る JSON に child は入らない）。
 */
function seFieldChild(key) {
  const f = seFieldSelect(key, "足す子の型", [], "", "対象が Q なら Q か A、A なら Q か F", true);
  const sel = f.el.querySelector("select");
  f.setNode = (nodeKey, want) => {
    const opts = seChildOptions(nodeKey);
    sel.innerHTML = "";
    const none = document.createElement("option");
    none.value = "";
    none.textContent = "なし";
    sel.appendChild(none);
    for (const [value, text] of opts) {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = text;
      sel.appendChild(o);
    }
    sel.value = opts.some((x) => x[0] === seTrim(want)) ? seTrim(want) : "";
    f.el.hidden = !opts.length;
  };
  return f;
}

/** 入力欄をまとめて小窓の入れ物にする（説明の文のように DOM のままの物も置ける）。 */
function seForm(fields) {
  const el = seEl("div", "sci-edit-form");
  const made = [];
  for (const f of Array.isArray(fields) ? fields : []) {
    if (!f) continue;
    if (f.el) {
      el.appendChild(f.el);
      made.push(f);
    } else if (f.nodeType === 1) {
      el.appendChild(f); // 説明の文のように、そのまま置く要素
    }
  }
  const error = seEl("p", "sci-edit-error");
  error.hidden = true;
  error.setAttribute("role", "alert");
  el.appendChild(error);
  const actions = seEl("div", "sci-edit-actions");
  el.appendChild(actions);
  const form = {
    el,
    actions,
    read() {
      const out = {};
      for (const f of made) {
        out[f.key] = f.read();
        for (const s of f.subs || []) out[s.key] = s.read();
      }
      return out;
    },
    say(text) {
      error.textContent = text || "";
      error.hidden = !text;
    },
    refresh() {
      const cur = form.read();
      for (const f of made) {
        if (typeof f.show === "function") f.el.hidden = !f.show(cur);
        if (typeof f.refresh === "function") f.refresh();
      }
    },
  };
  form.refresh();
  el.addEventListener("change", () => form.refresh());
  return form;
}

/**
 * seed（D）の入力欄（題と本文の 2 つ）。作成の板と Edit の小窓で同じ物を使う。
 * 問い（SQ）は seed の板で足す・直す・消す（seFieldsFor の add_seed_q・edit_seed_q）。
 */
function seSeedFields(init) {
  const v = init || {};
  return [
    seFieldText("title", "題（必須）", v.title, "必須・120 字まで"),
    seFieldArea("body", "本文（任意）", v.body, 4, "背景と制約など。自由な文"),
  ];
}

// ============================================================================
// 通信
// ============================================================================

/** 人の操作を送って描き直す。送れなかったときの理由は小窓の中に出す。 */
async function seApply(op, form) {
  try {
    const res = await paperApi(stApi("/apply"), { method: "POST", body: op });
    if (form && form.host && typeof form.host.close === "function") form.host.close();
    saiToast(SE_DONE[op.op] || "反映しました", "ok");
    if (typeof stReload === "function") await stReload();
    return res;
  } catch (e) {
    const why = (e && e.message) || "反映できませんでした";
    if (form && typeof form.say === "function") form.say(why);
    else saiToast(why, "error");
  }
  return null;
}

// ============================================================================
// Status から引く手（status.nodes は [{id, title, parent, star, ...}]）
// ============================================================================

function seStatusNodes() {
  const st = typeof stStatus === "function" ? stStatus() : null;
  return st && Array.isArray(st.nodes) ? st.nodes : [];
}

const seNodeOption = (n) => [n.id, `${n.id} ${n.title || n.text || ""}`.trim()];

/** 道の上（nodeId から根まで）の ID の集まり。 */
function seAncestors(nodes, nodeId) {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const out = new Set();
  let cur = byId.get(nodeId);
  while (cur && !out.has(cur.id)) {
    out.add(cur.id);
    cur = byId.get(cur.parent);
  }
  return out;
}

/** bottom_Q（bottom_Q。node の `bottom_q` を見る）。 */
function seBottomQs(nodes) {
  return (Array.isArray(nodes) ? nodes : []).filter((n) => n && seRefOk(n.id, "Q") && n.bottom_q);
}

/** detail.subtree の F（[{id, text, paper}]）。 */
function seSubtreeFs(detail) {
  const list = detail && Array.isArray(detail.subtree) ? detail.subtree : [];
  return list.filter((n) => n && seRefOk(n.id, "F"));
}

// ============================================================================
// 操作の入力欄（seOpenAction）
// ============================================================================

/** op ごとの入力欄（{title, values, fields}）。★・stance は seQuick で送るので無い。 */
function seFieldsFor(kind, nodeId, detail) {
  const d = detail || {};
  const node = d.node || {};
  const id = seTrim(nodeId) || seTrim(node.id);
  const nodes = seStatusNodes();

  switch (kind) {
    case "add_q": {
      const cands = (d.guide && Array.isArray(d.guide.child_candidates) ? d.guide.child_candidates : [])
        .map((c) => seTrim(c && typeof c === "object" ? c.text : c)).filter(Boolean);
      const text = seFieldArea("text", "文", "", 3, "必須・300 字まで");
      const fields = [];
      if (cands.length) {
        const guide = seFieldSelect("_guide", "ガイドの候補", cands.map((c) => [c, c]), "", "選ぶと文に入ります", true);
        let last = "";
        guide.refresh = () => {
          const v = guide.read();
          if (v && v !== last) text.set(v);
          last = v;
        };
        fields.push(guide);
      }
      fields.push(text);
      // 親が SEED（旧い印）のときは seBuildOp が「軸の Q は sai-sci init が作ります」で止める
      return { title: `Q を足す（${id}）`, values: { parent: id }, fields };
    }
    case "add_a":
      return {
        title: `A を足す（${id}）`,
        values: { q: id },
        fields: [
          seFieldText("text", "文", "", "必須・120 字まで"),
          seFieldArea("body", "本文", "", 4, "任意・2000 字まで"),
        ],
      };
    case "add_f":
      return {
        title: `F を足す（${id}）`,
        values: { parent: id },
        fields: [
          seFieldPick("stance", "stance（必須）", SE_STANCES, ""),
          seFieldArea("text", "文", "", 3, "必須・1〜3 文・1200 字まで"),
          seFieldLines("cites", "出典", [], 10, "1 行 1 つ。`#37 §3.2` の形（数字を書くときは出典か報告書が要る）", "#37 §3.2"),
          seFieldText("paper", "論文番号", "", "任意（37 のような数字）"),
          seFieldText("report", "報告書", "", "任意・100 字まで（数字の文は出典か報告書のどちらかが要る）"),
        ],
      };
    case "add_e":
      return {
        title: `E を足す（${id}）`,
        values: { parent: id },
        fields: [
          seFieldText("text", "文", "", "必須・120 字まで"),
          seFieldArea("body", "本文", "", 4, "任意・2000 字まで"),
          seFieldText("mlflow", "MLflow の名前", "", "任意・200 字まで"),
          seFieldText("dataset", "データセット", "", "任意・200 字まで（sai-ts のスナップショットは ts:<名前>@<sha256 の先頭 12 桁> の形）"),
        ],
      };
    case "insert_q":
      return {
        title: `Q を間に入れる（${id}）`,
        values: { q: id },
        fields: [seFieldArea("text", "文", "", 3, "必須・300 字まで。今の子の A は新しい Q の下へ入ります")],
      };
    case "move_a":
      return {
        title: `A を移す（${id}）`,
        values: { from: id },
        fields: [seFieldSelect("to", "移す先の Q", seBottomQs(nodes).filter((n) => n.id !== id).map(seNodeOption), "", "この Q の子の A を全部移します。移す先は bottom_Q", false)],
      };
    case "move_node": {
      // A を 1 件、その下の F・child_Q ごと移す。移す先は bottom_Q で、今の親は外す（ID は変わらない）
      const parent = (nodes.find((n) => n && n.id === id) || {}).parent;
      return {
        title: `A を 1 件移す（${id}）`,
        values: { node: id },
        fields: [seFieldSelect("to", "移す先の Q", seBottomQs(nodes).filter((n) => n.id !== parent).map(seNodeOption), "", "A の下の F・child_Q ごと移します。ID は変わりません", false)],
      };
    }
    case "regroup": {
      // Q の直下の A を、新しい中間の A の下の細分の Q へまとめる。細分の Q の文は、空なら「〈中間の A の文〉には、どんな種類があるか。」
      const members = nodes.filter((n) => n && seRefOk(n.id, "A") && n.parent === id).map(seNodeOption);
      const childQ = seFieldText("child_q", "細分の Q の文", "", "必須・300 字まで。空なら「〈中間の A の文〉には、どんな種類があるか。」", "〈中間の A の文〉には、どんな種類があるか。");
      return {
        title: `まとめる（${id}）`,
        values: { q: id },
        fields: [
          seFieldText("text", "中間の A の文", "", "必須・120 字まで。まとめた A の答え"),
          seFieldArea("body", "中間の A の本文", "", 3, "任意・2000 字まで"),
          childQ,
          seFieldArea("child_q_body", "細分の Q の本文", "分け方の基準: ", 3, "任意・2000 字まで。分け方の基準を書きます"),
          seFieldChecks("members", "まとめる A（Q の直下の A）", members, [], "1〜20 個。選んだ A は、細分の Q の下へ移ります（ID は変わりません）"),
        ],
        // 細分の Q の文は、空なら中間の A の文から作る（既定）
        build: (read) => Object.assign({}, read, {
          q: id,
          child_q: seTrim(read.child_q) || `${seTrim(read.text)}には、どんな種類があるか。`,
        }),
      };
    }
    case "set_mlflow":
      return {
        title: `MLflow を変える（${id}）`,
        values: { node: id },
        fields: [seFieldText("mlflow", "MLflow の名前", node.mlflow || "", "任意・200 字まで。空にすると外す")],
      };
    case "copy":
      return {
        title: `A を写す（${id}）`,
        values: { node: id },
        fields: [seFieldSelect("to", "写す先の Q", seBottomQs(nodes).map(seNodeOption), "", "bottom_Q", false)],
      };
    case "remove":
      return {
        title: `消す（${id}）`,
        values: { node: id },
        fields: [],
      };
    case "add_seed_q":
      // id は seed（D）。軸は「なし」が既定（空なら送らない）
      return {
        title: `問いを足す（${id}）`,
        values: { seed: id },
        fields: [
          seFieldArea("text", "問い", "", 3, "必須・300 字まで"),
          seFieldSelect("axis", "軸", SE_SEED_AXES, "", "軸は無くても足せます", true),
        ],
      };
    case "edit_seed_q":
      // id は問い（SQ）。今の文と軸は detail.node から入れる
      return {
        title: `問いを直す（${id}）`,
        values: { q: id },
        fields: [
          seFieldArea("text", "問い", node.text || "", 3, "必須・300 字まで"),
          seFieldSelect("axis", "軸", SE_SEED_AXES, node.axis || "", "「なし」にすると軸を外します", true),
        ],
      };
    case "add_answer":
      // id は持ち主（D か S）。問いは detail.q。A で答えるときは板の Add root path を使う
      return {
        title: `回答を足す（${id}・${seTrim(d.q)}）`,
        values: { to: id, q: seTrim(d.q) },
        fields: [seFieldArea("text", "回答の文", "", 3, "必須・300 字まで")],
      };
    case "edit_answer":
      // id は回答（SA）。今の文は detail.node から入れる
      return {
        title: `回答を直す（${id}）`,
        values: { answer: id },
        fields: [seFieldArea("text", "回答の文", node.text || "", 3, "必須・300 字まで")],
      };
    case "edit": {
      // seed（D）は題と本文で直す（空の題は送らない）
      if (seRefOk(id, "D")) {
        return {
          title: `seed（D）を直す（${id}）`,
          values: { node: id },
          fields: seSeedFields({ title: node.text, body: node.body }),
          build: (read) => ({ node: id, text: seTrim(read.title), body: seTrim(read.body) }),
          check: (read) => (seTrim(read.title) ? "" : "題を入力してください"),
        };
      }
      const rules = seEditRules(id);
      const fields = [seFieldArea("text", "文", node.text || "", 3, rules.body ? "文か本文のどちらかは必須・120 字まで" : (id.charAt(0) === "F" ? "必須・1〜3 文・1200 字まで" : "必須・300 字まで"))];
      if (rules.body) fields.push(seFieldArea("body", "本文", node.body || "", 5, "任意・2000 字まで"));
      return { title: `文を直す（${id}）`, values: { node: id }, fields };
    }
    default:
      return { title: seOpLabel(kind), values: {}, fields: [] };
  }
}

/** 入力欄を出さずに送る（★・stance）。 */
async function seQuick(op, payload) {
  const built = seBuildOp(op, Object.assign({}, payload));
  if (!built.ok) { saiToast(built.error, "error"); return null; }
  return seApply(built.op, null);
}

/** 問い・回答を消す。確かめてから送る（小窓は出さない）。 */
async function seRemoveQuick(op, payload, title, body) {
  const built = seBuildOp(op, Object.assign({}, payload));
  if (!built.ok) { saiToast(built.error, "error"); return null; }
  const ok = await saiConfirm({ title, body, ok: "消す", danger: true });
  if (!ok) return null;
  return seApply(built.op, null);
}

/**
 * op の小窓を出す。SE_QUICK の op は seQuick に回す。remove は確認を必ず出す。
 * 送る前に画面でも必須の項目と字数と出典を確かめる。通らなければ送らない。
 */
function seOpenAction(op, nodeId, detail) {
  if (typeof document === "undefined" || typeof saiSheet !== "function") return;
  const name = seTrim(op);
  // seed（D）・SPROUT は小窓ではなく、作成のパネルで作る（作成中も図と板を操作できる）
  if (name === "add_seed") {
    if (typeof stStartSeed === "function") stStartSeed();
    else saiToast("seed（D）の作成画面がまだありません", "error");
    return;
  }
  if (name === "add_sprout") {
    if (typeof stStartSprout === "function") stStartSprout(seRefOk(nodeId, "D") ? seTrim(nodeId) : "");
    else saiToast("SPROUT の作成画面がまだありません", "error");
    return;
  }
  if (SE_QUICK.includes(name)) {
    void seQuick(name, { node: seTrim(nodeId) });
    return;
  }
  // E は SPROUT（S）か E の下にしか作れない。A から開いたときは断る。
  if (name === "add_e" && !/^[SE]/.test(seTrim(nodeId))) {
    saiToast("親（S か E）になる Node から開いてください", "error");
    return;
  }
  const spec = seFieldsFor(name, nodeId, detail);
  const form = seForm(spec.fields);
  const body = seEl("div", "sci-edit-wrap");
  body.appendChild(form.el);
  const host = saiSheet({ title: spec.title, body });
  form.host = host;
  form.actions.appendChild(seBtn("やめる", () => host.close(), "ghost"));

  const send = async () => {
    const read = form.read();
    const stop = spec.check ? spec.check(read) : "";
    if (stop) { form.say(stop); return; }
    const v = spec.build ? spec.build(read) : Object.assign({}, spec.values, read);
    const built = seBuildOp(name, v);
    if (!built.ok) { form.say(built.error); return; }
    if (name === "remove") {
      const ok = await saiConfirm({
        title: "消しますか",
        body: `${built.op.node} とその下の Node をすべて消します。元に戻せません。`,
        ok: "消す",
        danger: true,
      });
      if (!ok) return;
    }
    await seApply(built.op, form);
  };
  form.actions.appendChild(seBtn(name === "remove" ? "消す" : "送る", send, name === "remove" ? "danger" : "primary"));
}

// ============================================================================
// カードのフォーム（seOpenCard）
// ============================================================================

/**
 * カードを作る小窓。preset（C-CARD）があれば各欄の初めの値にする。
 * runner に user を選ぶと READ_CARD になる（論文は 1 本・ブロックの既定は lab-read）。
 */
async function seOpenCard(nodeId, preset) {
  if (typeof document === "undefined" || typeof saiSheet !== "function") return;
  const init = seCardValues(preset, nodeId);
  const wantId = seCardNode(nodeId);
  let detail = null;
  if (wantId) {
    try {
      detail = await paperApi(stApi(`/nodes/${encodeURIComponent(wantId)}`));
    } catch (e) {
      saiToast((e && e.message) || "Node を読み込めませんでした", "error");
      return;
    }
  }
  const candidates = seSubtreeFs(detail).filter((f) => f.paper != null).map((f) => String(f.paper));
  const uniq = [...new Set(candidates)];
  const known = new Set(uniq);
  const typed = init.papers.filter((n) => !known.has(String(n))).map(String);

  const fields = [];
  let nodeField = null;
  if (!wantId) {
    const picks = [["", "木の全体（Node を選ばない）"], ...seStatusNodes().filter((n) => n && seRefOk(n.id)).map(seNodeOption)];
    nodeField = seFieldSelect("node", "Node", picks, init.node, "CARD を向ける Node。木の全体は Node を送りません", false);
    fields.push(nodeField);
  }
  const block = seFieldSelect("block", "ブロック", SE_BLOCKS, init.block, "必須", false);
  const purpose = seFieldArea("purpose", "目的", init.purpose, 3, "必須・300 字まで。何を達成するか");
  const mode = seFieldSelect("mode", "型", SE_MODES, init.mode, "lab-read のときだけ使う", false);
  mode.show = (v) => v.block === "lab-read";
  const depth = seFieldSelect("depth", "深さ", SE_DEPTHS, init.depth, "READ_CARD のときだけ使う", false);
  depth.show = (v) => v.runner === "user";
  const runner = seFieldSelect("runner", "実行するもの", SE_RUNNERS, init.runner, "user は自分で読む（READ_CARD）", false);
  const model = seFieldSelect("model", "モデル", SE_CARD_MODELS, init.model, "cc のときだけ使う", false);
  model.show = (v) => v.runner === "cc";
  const thinking = seFieldSelect("thinking", "thinking", SE_CARD_THINKING, init.thinking, "cc のときだけ使う", false);
  thinking.show = (v) => v.runner === "cc";
  const child = seFieldChild("child");
  fields.push(
    block, purpose,
    seFieldArea("expect", "期待する成果", init.expect, 2, "300 字まで（任意）"),
    runner,
    model,
    thinking,
    mode,
    child,
  );
  if (uniq.length) {
    fields.push(seFieldChecks("papers", "論文", uniq.map((p) => [p, `#${p}`]), init.papers.map(String), "この Node の下の F の論文から選ぶ"));
  }
  fields.push(seFieldLines("papers_typed", "論文の番号", typed, 20, "番号を打って足せます（20 本まで。READ_CARD は 1 本）", "37"), depth);

  const form = seForm(fields);
  // runner を user にしたとき、ブロックが空なら lab-read にする
  let lastRunner = init.runner;
  runner.refresh = () => {
    const r = runner.read();
    if (r === "user" && lastRunner !== "user" && !block.read()) block.set("lab-read");
    lastRunner = r;
  };
  // 足す子の型は対象の種類で選べる物が変わる。Node を選び替えたときだけ作り直す
  let lastNode = nodeField ? nodeField.read() : (wantId || init.node);
  child.setNode(lastNode, init.child);
  if (nodeField) {
    nodeField.refresh = () => {
      const n = nodeField.read();
      if (n !== lastNode) { lastNode = n; child.setNode(n, ""); }
    };
  }
  form.refresh();

  const host = saiSheet({ title: "CARD を作る", body: form.el });
  form.host = host;
  form.actions.appendChild(seBtn("やめる", () => host.close(), "ghost"));
  form.actions.appendChild(seBtn("作る", async () => {
    const v = form.read();
    const built = seBuildCard({
      node: nodeField ? v.node : wantId,
      block: v.block, purpose: v.purpose, expect: v.expect, runner: v.runner,
      model: v.model, thinking: v.thinking,
      mode: v.mode, depth: v.depth, child: v.child,
      papers: [...(Array.isArray(v.papers) ? v.papers : []), ...(Array.isArray(v.papers_typed) ? v.papers_typed : [])],
    });
    if (!built.ok) { form.say(built.error); return; }
    const json = built.card;
    if (preset && preset.origin_run != null) json.origin_run = preset.origin_run;
    let res = null;
    try {
      res = await paperApi(stApi("/cards"), { method: "POST", body: json });
    } catch (e) {
      form.say((e && e.message) || "CARD を作れませんでした");
      return;
    }
    const runId = seCardRunId(res);
    saiToast(runId ? `run ${runId} の CARD を作りました` : "CARD を作りました", "ok");
    form.el.innerHTML = "";
    const done = seEl("div", "sci-edit-done");
    done.appendChild(seEl("p", "sci-edit-done-text", runId ? `run ${runId} の CARD を作りました（${json.block}）` : "CARD を作りました"));
    const row = seLine("sci-edit-done-row");
    if (runId) {
      const toRead = json.runner === "user";
      row.appendChild(seBtn(toRead ? "READ_CARD を開く" : `run ${runId} を開く`, () => {
        host.close();
        location.hash = toRead
          ? `#/research/read/${encodeURIComponent(runId)}`
          : `#/run/${encodeURIComponent(selectedPaperProject())}/${encodeURIComponent(runId)}`;
      }, "primary"));
    }
    row.appendChild(seBtn("閉じる", () => host.close()));
    done.appendChild(row);
    form.el.appendChild(done);
    if (typeof stReload === "function") await stReload();
  }, "primary"));
}

// --- node からの読み込み（tests/test_sciedit.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    seTrim, seOpLabel, seCiteOk, seRefOk, seIdOk, seCountSentences, seNeedsCite, seEditRules, seBuildOp,
    seCardValues, seCardNode, seBuildCard, seCardRunId, seChildOptions, seChildOk,
    seBottomQs,
    SE_OPS, SE_DONE, SE_QUICK, SE_BLOCKS, SE_CITE_RE, SE_ID_RE, SE_SQ_RE, SE_SA_RE, SE_STANCES, SE_SEED_AXES,
  };
}

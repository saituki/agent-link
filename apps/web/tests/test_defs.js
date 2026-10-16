"use strict";
// 設定画面（static/defs.js・U5）の DOM を使わない部分のテスト（assert だけ）。
// 確かめること:
//   - 住所の往復（一覧と編集の 2 つ）
//   - 定義名の検査（英数字・-・_ の 64 文字まで / settings は settings だけ）
//   - block と template の空のひな形（sai-base が無ければ templates: の行を入れない）
//   - 一覧の組み分け（種類 × 置き場。settings の 2 つは必ず出て、並びは固定）
//   - 1 行の札（共通 / shadowed / 読めません / 使っている block がある）
// 実行:
//     node tests/test_defs.js

const assert = require("assert");
const path = require("path");

const df = require(path.join(__dirname, "..", "src", "agent_link_web", "static", "defs.js"));

// C-A1 の list の形そのままの偽データ
const ITEMS = [
  { kind: "block", scope: "project", name: "kantoku", title: "🎬 kantoku", description: "実装ガイドを読み…", exists: true, shadowed: false, error: null, used_by: [] },
  { kind: "block", scope: "project", name: "sub", title: "sub", description: "", exists: true, shadowed: false, error: null, used_by: ["kantoku"] },
  { kind: "block", scope: "global", name: "sub", title: "sub（共通）", description: "", exists: true, shadowed: true, error: null, used_by: [] },
  { kind: "block", scope: "project", name: "bad", title: "bad", description: "", exists: true, shadowed: false, error: "frontmatter が読めません", used_by: [] },
  { kind: "template", scope: "project", name: "sai-base", title: "sai-base", description: "", exists: true, shadowed: false, error: null, used_by: ["plan"] },
  { kind: "template", scope: "global", name: "sai-base", title: "sai-base（共通）", description: "", exists: true, shadowed: false, error: null, used_by: [] },
  { kind: "settings", scope: "project", name: "settings", title: "settings.json", description: "", exists: true, shadowed: false, error: null, used_by: [] },
  { kind: "settings", scope: "global", name: "settings", title: "settings.json（共通）", description: "", exists: false, shadowed: false, error: null, used_by: [] },
];

function testHash() {
  assert.strictEqual(df.dfListHash("agent-link-web"), "#/project/agent-link-web/defs");
  assert.strictEqual(
    df.dfHash("agent-link-web", "block", "project", "kantoku"),
    "#/project/agent-link-web/defs/block/project/kantoku"
  );
  // 編集の 1 つが無いときは一覧の住所
  assert.strictEqual(df.dfHash("p", null, "project", "x"), "#/project/p/defs");
  assert.strictEqual(df.dfHash("p", "block", null, "x"), "#/project/p/defs");
  assert.strictEqual(df.dfHash("p", "block", "project", null), "#/project/p/defs");
}

function testParseHash() {
  // 一覧
  assert.deepStrictEqual(df.dfParseHash("#/project/p/defs"), {
    name: "p", kind: null, scope: null, defName: null,
  });
  // 編集
  assert.deepStrictEqual(df.dfParseHash("#/project/p/defs/block/project/kantoku"), {
    name: "p", kind: "block", scope: "project", defName: "kantoku",
  });
  assert.deepStrictEqual(df.dfParseHash("#/project/p/defs/settings/global/settings"), {
    name: "p", kind: "settings", scope: "global", defName: "settings",
  });
  // 往復（日本語と空白を含む名前でも元に戻ること）
  const name = "/fonts/本 置き場";
  assert.strictEqual(df.dfParseHash(df.dfHash(name, "block", "global", name)).defName, name);
  // 知らない種類・置き場は一覧として扱う（編集画面を開かない）
  assert.deepStrictEqual(df.dfParseHash("#/project/p/defs/oreason/project/x"), {
    name: "p", kind: null, scope: null, defName: null,
  });
  // 設定のアドレス以外は null
  assert.strictEqual(df.dfParseHash("#/project/p/list"), null);
  assert.strictEqual(df.dfParseHash("#/project/p"), null);
  assert.strictEqual(df.dfParseHash(""), null);
}

function testValidName() {
  assert.strictEqual(df.dfValidName("block", "kantoku").ok, true);
  assert.strictEqual(df.dfValidName("block", "plan-kantoku").ok, true);
  assert.strictEqual(df.dfValidName("block", "a_b-9").ok, true);
  assert.strictEqual(df.dfValidName("block", "0start").ok, true);
  assert.strictEqual(df.dfValidName("block", "a".repeat(64)).ok, true);
  // 規則違反
  assert.strictEqual(df.dfValidName("block", "").ok, false);
  assert.strictEqual(df.dfValidName("block", "-kantoku").ok, false); // 英数字ではじまらない
  assert.strictEqual(df.dfValidName("block", "kan toku").ok, false);
  assert.strictEqual(df.dfValidName("block", "kan.toku").ok, false);
  assert.strictEqual(df.dfValidName("block", "a".repeat(65)).ok, false);
  // 漢字は使えない
  assert.strictEqual(df.dfValidName("template", "本").ok, false);
  // 理由の文が付く
  assert.ok(df.dfValidName("block", "-x").message.length > 0);
  // settings は settings だけ
  assert.strictEqual(df.dfValidName("settings", "settings").ok, true);
  assert.strictEqual(df.dfValidName("settings", "other").ok, false);
  // 同じ検査が agent-link と同じ形（C-A1）
  assert.strictEqual(df.DF_NAME_RE.source, "^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$");
}

function testTemplate() {
  const withBase = df.dfTemplate("block", "u5-check", true);
  assert.ok(withBase.startsWith("---\n"));
  assert.ok(withBase.includes("\nname: u5-check\n"));
  assert.ok(withBase.includes("\nid: u5-check\n"));
  assert.ok(withBase.includes("\ntemplates: sai-base\n"));
  assert.ok(withBase.includes("\n---\n"));
  // 依頼は最初のメッセージの「# task_text」に入るので、古い記法は入れない
  assert.ok(!withBase.includes("{{input.task_text}}"));
  assert.ok(withBase.includes("\n# Instructions\n"));
  assert.ok(withBase.includes("Write the instructions for this block's agent here."));
  // sai-base が無ければ templates: の行を入れない
  const noBase = df.dfTemplate("block", "u5-check", false);
  assert.ok(!noBase.includes("templates:"));
  assert.ok(!noBase.includes("{{input.task_text}}"));
  assert.ok(noBase.includes("\n# Instructions\n"));
  // template は 1 行
  assert.strictEqual(df.dfTemplate("template", "u5-check", true), "# u5-check\n\n");
  // settings は JSON
  assert.strictEqual(df.dfTemplate("settings", "settings", true), "{}\n");
}

function testGroups() {
  const groups = df.dfGroups(ITEMS);
  // 並びは block → template → settings
  assert.deepStrictEqual(groups.map((g) => g.kind), ["block", "template", "settings"]);
  // 並びは project → global
  for (const g of groups) {
    assert.deepStrictEqual(g.scopes.map((s) => s.scope), ["project", "global"]);
  }
  // block は 4 件（プロジェクト 3・共通 1）
  const blocks = groups[0];
  assert.strictEqual(blocks.scopes[0].items.length, 3);
  assert.strictEqual(blocks.scopes[1].items.length, 1);
  // 見出しは日本語
  assert.strictEqual(blocks.kindLabel, "block");
  assert.strictEqual(blocks.scopes[0].scopeLabel, "This project");
  assert.strictEqual(blocks.scopes[1].scopeLabel, "All projects (~/.agent-link)");
  // 0 件の組は入れない（template に global が無くても settings は 2 つ出る）
  const onlyProject = df.dfGroups(ITEMS.filter((i) => i.scope === "project"));
  assert.deepStrictEqual(onlyProject.map((g) => g.kind), ["block", "template", "settings"]);
  assert.strictEqual(onlyProject[1].scopes.length, 1);
  assert.strictEqual(onlyProject[2].scopes.length, 1);
  // 壊れた応答でも止めない
  assert.deepStrictEqual(df.dfGroups(null), []);
  assert.deepStrictEqual(df.dfGroups([null, { kind: "oreason" }]), []);
  // 並びは名前順
  const names = groups[0].scopes[0].items.map((i) => i.name);
  assert.deepStrictEqual(names, ["bad", "kantoku", "sub"]);
}

function testRowChips() {
  // 共通
  const common = df.dfRowChips(ITEMS[2]);
  assert.deepStrictEqual(common.filter((c) => c.cls === "ok").map((c) => c.text), ["Shared"]);
  // プロジェクト側が使われます
  assert.ok(df.dfRowChips(ITEMS[2]).some((c) => c.text === "The project copy is used"));
  // 読めません
  const bad = df.dfRowChips(ITEMS[3]);
  assert.ok(bad.some((c) => c.cls === "danger" && c.text.includes("Cannot read")));
  // 使っている block がある
  assert.ok(df.dfRowChips(ITEMS[1]).some((c) => c.text === "Used by 1 block"));
  // settings が無い
  assert.ok(df.dfRowChips(ITEMS[7]).some((c) => c.text.includes("you can create it")));
  // 普通のものは何も出さない
  assert.deepStrictEqual(df.dfRowChips(ITEMS[0]), []);
  assert.deepStrictEqual(df.dfRowChips(null), []);
}

function testUsedBy() {
  assert.strictEqual(df.dfUsedByText(["kantoku", "plan-kantoku"]), "Used by: kantoku, plan-kantoku");
  assert.strictEqual(df.dfUsedByText([]), "");
  assert.strictEqual(df.dfUsedByText(null), "");
  assert.strictEqual(df.dfUsedByText(["", null]), "");
}

function testTitle() {
  assert.strictEqual(df.dfTitle(ITEMS[0]), "🎬 kantoku");
  // 題が無いときは名前
  assert.strictEqual(df.dfTitle({ name: "x" }), "x");
  assert.strictEqual(df.dfTitle(null), "");
}

function testConstants() {
  assert.deepStrictEqual(df.DF_KINDS, ["block", "template", "settings"]);
  assert.deepStrictEqual(df.DF_SCOPES, ["project", "global"]);
  assert.strictEqual(df.DF_SETTINGS_NAME, "settings");
}

function main() {
  testHash();
  testParseHash();
  testValidName();
  testTemplate();
  testGroups();
  testRowChips();
  testUsedBy();
  testTitle();
  testConstants();
  console.log("test_defs: すべて通りました");
}

main();

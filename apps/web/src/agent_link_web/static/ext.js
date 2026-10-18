// 拡張の画面の枠（U07）。拡張の JS は AgentLinkExt.register(name, {mount}) で画面を登録する。
// 登録すると、サイドバーの拡張のタブから開ける（住所は #/ext/<name>[/<project>[/<path>]]）。
// 約束の正は crates/agent-link-engine/assets/template/ext-contract.md（共有契約 C6）。

// 拡張の名前の規則（共有契約 C1 の extension.json の name と同じ）
const EXT_NAME_RE = /^[a-z][a-z0-9-]{0,31}$/;
// 同じ名前の二重登録を防ぐ（ページを読み直すまで覚える）
const extNames = new Set();
// 最後に選んだプロジェクトを覚える鍵（拡張ごと）
const EXT_LAST_KEY = "agent_link_ext_last_";
// 拡張の画面の枠（index.html の <section id="view-ext">）
const EXT_VIEW_ID = "view-ext";

// 住所から拡張の名前とプロジェクト・後ろの道を取り出す（DOM を使わない）。
// 合わなければ null。#/extra のような名前の続きは別の画面として扱う。
function extParseHash(hash, name) {
  const prefix = "#/ext/" + name;
  if (hash === prefix) return { ext: name, project: null, path: "" };
  if (!hash.startsWith(prefix + "/")) return null;
  const rest = hash.slice(prefix.length + 1);
  const i = rest.indexOf("/");
  const projectPart = i < 0 ? rest : rest.slice(0, i);
  if (projectPart === "") return null;
  let project;
  try {
    project = decodeURIComponent(projectPart);
  } catch {
    return null; // 壊れた %xx は合わない住所
  }
  return { ext: name, project, path: i < 0 ? "" : rest.slice(i + 1) };
}

// 開くプロジェクトを決める。最後に選んだ物が使えればそれ、無ければ先頭、無ければ null
function extPickProject(enabledIn, last) {
  if (!Array.isArray(enabledIn) || enabledIn.length === 0) return null;
  return enabledIn.includes(last) ? last : enabledIn[0];
}

function extLastGet(name) {
  try { return sessionStorage.getItem(EXT_LAST_KEY + name); }
  catch { return null; } // プライベートブラウズでも動くように
}

function extLastSet(name, project) {
  try { sessionStorage.setItem(EXT_LAST_KEY + name, project); }
  catch { /* 保存できなくても動く */ }
}

// 共有契約 C6 の ctx（拡張の mount と poll に渡す）
function extCtx(name, params) {
  const project = params.project;
  return {
    ext: name,
    project,
    path: params.path || "",
    go(path) {
      location.hash = `#/ext/${encodeURIComponent(name)}/${encodeURIComponent(project)}/${path}`;
    },
    cli(args) {
      return saiApi(`/api/projects/${encodeURIComponent(project)}/ext/${name}/cli`, {
        method: "POST",
        body: { args },
      });
    },
    svc(path, { method = "GET", query, body } = {}) {
      let url = `/api/ext/${name}/svc/${path}`;
      if (query && typeof query === "object") {
        const qs = new URLSearchParams(query).toString();
        if (qs) url += `?${qs}`;
      }
      return saiApi(url, { method, body });
    },
    url(file) {
      return `/ext/${name}/${file}`;
    },
  };
}

// 画面の枠に空の案内を出す（拡張が使えないときなど）
function extShowEmpty(view, opts) {
  view.textContent = "";
  view.append(saiEmpty(opts));
}

// 拡張の画面を開く。開くたびに /api/ext を取り直し、新しい空の div に mount させる
async function extOpen(name, def, params) {
  const view = document.getElementById(EXT_VIEW_ID);
  try {
    const list = await saiApi("/api/ext");
    const ext = (list.extensions || []).find((e) => e.name === name);
    const enabledIn = ext ? ext.enabled_in || [] : [];
    if (!ext || !ext.ui) {
      extShowEmpty(view, {
        title: "This extension is not available",
        body: `Check that ${name} is installed with: agent-link ext list`,
      });
      return;
    }
    if (!params.project) {
      const pick = extPickProject(enabledIn, extLastGet(name));
      if (pick === null) {
        extShowEmpty(view, {
          title: "No project uses this extension",
          body: `Run: agent-link ext enable ${name} in a project.`,
        });
        return;
      }
      // 住所を選んだプロジェクトに置き換える（hashchange で route が直し直す）
      location.replace(`#/ext/${encodeURIComponent(name)}/${encodeURIComponent(pick)}`);
      return;
    }
    if (!enabledIn.includes(params.project)) {
      extShowEmpty(view, {
        title: `This extension is not enabled in ${params.project}`,
        body: `Run: agent-link ext enable ${name} in ${params.project}.`,
      });
      return;
    }
    extLastSet(name, params.project);

    // 見出し（拡張の title）と、プロジェクトを選ぶ欄
    const head = saiEl("header", "appbar ext-head");
    head.append(saiEl("h1", "", ext.title));
    const select = document.createElement("select");
    select.className = "ext-project";
    select.setAttribute("aria-label", "Project");
    for (const p of enabledIn) {
      const opt = saiEl("option", "", p);
      opt.value = p;
      select.append(opt);
    }
    select.value = params.project;
    select.addEventListener("change", () => {
      location.hash = `#/ext/${encodeURIComponent(name)}/${encodeURIComponent(select.value)}`;
    });
    head.append(select);

    // 拡張の中身は毎回新しい空の div に描かせる
    const body = saiEl("div", "ext-body");
    view.textContent = "";
    view.append(head, body);
    try {
      await def.mount(body, extCtx(name, params));
    } catch (err) {
      console.error(err);
      body.textContent = "";
      body.append(saiEmpty({ title: "This extension failed to open", body: err.message }));
    }
  } catch (err) {
    console.error(err);
    extShowEmpty(view, { title: "This extension failed to open", body: err.message });
  }
}

// 拡張の画面を SaiRoutes に登録する。name の規則や二重登録は warn して無視する
function extRegister(name, def) {
  if (!EXT_NAME_RE.test(name) || !def || typeof def.mount !== "function") {
    console.warn(`AgentLinkExt: invalid registration for "${name}" (ignored).`);
    return;
  }
  if (extNames.has(name)) {
    console.warn(`AgentLinkExt: "${name}" is already registered (ignored).`);
    return;
  }
  extNames.add(name);
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return extParseHash(hash, name);
    },
    view: EXT_VIEW_ID,
    tab: "ext-" + name,
    interval: def.interval || 0,
    async open(params) {
      await extOpen(name, def, params);
    },
    async poll(params) {
      // プロジェクトが決まる前（住所の置き換え前）は呼ばない
      if (def.poll && params.project) await def.poll(extCtx(name, params));
    },
  });
}

// --- ブラウザだけで動く部分（node には window が無い） ---
if (typeof window !== "undefined") {
  window.AgentLinkExt = { register: extRegister };
}

// --- node からの読み込み（tests/test_ext.js。DOM を使わない部分だけ） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { extParseHash, extPickProject };
}

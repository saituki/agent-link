/* sai-phone の Service Worker．
   静的ファイル（画面の部品）の取得のみ助ける．
   会話データはキャッシュしない（古い会話が残ると状況を誤解して危険なため）．

   画面ファイル（index.html / app.js / style.css など）は
   **ネットワーク優先・失敗したらキャッシュ**（02 §3.3）。
   以前はキャッシュ優先でキャッシュ名が固定のため，サーバーを直しても
   iPhone に古い app.js が残っていた。キャッシュ名はバージョンを上げ，
   activate 時に古いバージョンを消す（04 で /stats.js，05 で v4，06 で /project.js と v5，
   07 で /papers.js と v6，08 で v7＝Discover と過去の検索結果，09 で /radio.js と v8＝Radio）。
   09 からダウンロードした音声は別々の倉庫（sai-phone-radio-audio）に保存し，
   再生用に Range の部分取り（206）もここで返す（09 §4.9）。 */

// バージョンを上げたら ASSETS も見直す（09 で radio.js を追加，10 で pcm-worklet.js を追加し v9，
// 10 の文字の質問の直し（radio.js の ID 判定）で v10 に。古い radio.js が端末に残らないように。
// CP3 の直し（会話の並び順・吹き出しの色分けの chatorder.js 追加）で v11 に）
// 13 の md 整形表示（markdown.js 追加と app.js の miniMarkdown 削除）で v12 に）
// 18 の Papers プロジェクト切り替え（papers.js / radio.js / index.html / style.css の更新）で v13 に）
// 14 の Project に List 画面（linkview.js 追加と index.html / style.css / project.js の更新）で v14 に）
// 15 の task.md 作成フォーム（cardform.js 追加）で v15 に）
// Papers の Dashboard・ノート編集・図と List の task / review 編集（papers.js / radio.js / linkview.js 等）で v16 に）
// papers.js の recentRow が project.js の同名関数を上書きしていた不具合の直しで v17 に）
// 通知の登録直後にテスト通知の結果を出す（app.js）で v18 に）
// 通知を押したときにメッセージ画面へ移るよう直して v19 に）
// 裏で開いていたアプリが前に戻ったときも飛び先を読むよう直して v20 に）
// 通知の飛び先の受け取り開始（startMessages）・読み直しと，調べもの用の記録を足して v21 に）
// Agent の ⓘ で版の確認と入れ替えをできるようにして v22 に（index.html の ?v= も 22）
// 調べもの用の記録（通知の飛び先）を消して v23 に
// Usage タブ（usage.js 追加と index.html / style.css / app.js の更新）で v24，見た目の手直しで v25 に
// ■ 終了（U02）・List の session と 📊 の行き先（U11）・MCP の道具の色（U12）で v26 に（index.html の ?v= も 26）
// 質問の読み上げ（U01・voiceask.js）と図の配置の純関数（U05・canvaslayout.js）で v27 に（index.html の ?v= も 27）
// U4（画面の土台・md の色・ホーム画面）：ui.js と markdown.css / home.js / home.css を追加し、
// U5・U6 用の空のファイル（workflow.js / defs.js / paperimport.js とそれぞれの CSS）も先に登録して v28 に
// （index.html の ?v= も 28）
// U5 の引用一覧の画面（cite.js 追加と index.html / sw.js / papers.js / paperimport.js の更新）で v30 に
// （index.html の ?v= も 30。ASSETS にも /cite.js を足した）
// U19 の研究方針の木（scitree.js とその CSS。U20・U21 用の空のファイル sciedit.js / sciread.js /
// scigraph.js と sciedit.css / scigraph.css も先に登録）で v32 に
// （index.html の ?v= も 32。ASSETS にも 7 つを足した）
// コードレビュー R1 の指摘 1〜6 の直し（papers.js・scitree.js・scitree.css）で v34 に
// （index.html の ?v= も 34）
// 回答済みの表示を問ごとの縦並びにした（app.js・style.css）で v38 に（index.html の ?v= も 38）
const CACHE = "sai-phone-v38";
const ASSETS = ["/", "/index.html", "/style.css", "/markdown.css", "/home.css", "/workflow.css", "/defs.css", "/papers.css", "/markdown.js", "/ui.js", "/app.js", "/chatorder.js", "/stats.js", "/project.js", "/linkview.js", "/cardform.js", "/papers.js", "/radio.js", "/usage.js", "/voiceask.js", "/canvaslayout.js", "/pcm-worklet.js", "/home.js", "/workflow.js", "/defs.js", "/paperimport.js", "/cite.js", "/scitree.js", "/sciedit.js", "/sciread.js", "/scigraph.js", "/scitree.css", "/sciedit.css", "/scigraph.css", "/manifest.json"];

// オフライン保存した音声の倉庫（09 §4.9。sai-phone-v… とは別なので activate で消さない）
const RADIO_CACHE = "sai-phone-radio-audio";
// 音声の住所（合言葉 ?token= は付けない，本体だけ比べる・09 §4.9）
function isAudioUrl(pathname) {
  return /^\/api\/radio\/papers\/[^/]+\/audio$/.test(pathname);
}

// 倉庫の音声から「指定された範囲だけ」を切り出して 206 で返す。
// Safari は再生の途中から Range（部分指定）で取りに来るため，倉庫に
// 全体しか無いときはここでハサミで切って返す必要がある（移植元から・09 §4.9）。
async function rangeResponse(cached, rangeHeader) {
  const total = (await cached.clone().arrayBuffer()).byteLength;
  const m = /^bytes=(\d*)-(\d*)$/.exec(rangeHeader.trim());
  if (!m) return cached;
  const startText = m[1];
  const endText = m[2];
  let start;
  let end;
  if (startText === "" && endText === "") return cached;
  if (startText === "") {
    // 例: bytes=-500 → 最後の500バイトが欲しい
    const suffix = Number(endText);
    if (!Number.isFinite(suffix) || suffix <= 0) return cached;
    start = Math.max(0, total - suffix);
    end = total - 1;
  } else {
    start = Number(startText);
    end = endText === "" ? total - 1 : Number(endText);
  }
  if (!Number.isFinite(start) || !Number.isFinite(end)) return cached;
  if (start >= total || start > end) return cached;
  end = Math.min(end, total - 1);
  const buf = await cached.arrayBuffer();
  const sliced = buf.slice(start, end + 1);
  const contentType = cached.headers.get("Content-Type") || "audio/mp4";
  return new Response(sliced, {
    status: 206,
    headers: {
      "Content-Type": contentType,
      "Content-Range": `bytes ${start}-${end}/${total}`,
      "Accept-Ranges": "bytes",
      "Content-Length": String(end - start + 1),
    },
  });
}

// 音声の GET: 倉庫（ダウンロード済み）にあればそこから，無ければ通常どおり通信
// （保存はしない。保存は再生画面のダウンロードボタンだけ・09 §4.9）。
async function radioAudioResponse(request) {
  try {
    const cache = await caches.open(RADIO_CACHE);
    const cached = await cache.match(request, { ignoreSearch: true });
    if (cached) {
      const rangeHeader = request.headers.get("Range");
      if (rangeHeader) return rangeResponse(cached, rangeHeader);
      return cached;
    }
  } catch {
    // 倉庫が使えないときは通信するだけ（ここでは止めない）
  }
  return fetch(request);
}

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(ASSETS)).then(() => self.skipWaiting()));
});

// 古いバージョンのキャッシュ（sai-phone-v3 など）だけをここで消す。
// sai-phone-v で始まらないキャッシュ（09 で作る音声用 sai-phone-radio-audio 等）は
// 消さないようにする（05 §6）。
self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(
        keys.filter((k) => k.startsWith("sai-phone-v") && k !== CACHE).map((k) => caches.delete(k))
      )
    ).then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
  // ダウンロードした音声は倉庫から（Range にも対応・09 §4.9）。
  // この1本だけ先に判定し，他の /api/ は今までどおり素通しにする。
  if (e.request.method === "GET" && isAudioUrl(url.pathname)) {
    e.respondWith(radioAudioResponse(e.request));
    return;
  }
  // 会話データと API はそのまま通す（保存しない）
  if (url.pathname.startsWith("/api/")) return;
  // ネットワーク優先 → 失敗したらキャッシュ → それも無ければトップ画面
  e.respondWith(
    fetch(e.request)
      .then((res) => {
        if (res.ok) {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(e.request, copy));
        }
        return res;
      })
      .catch(async () => {
        const hit = await caches.match(e.request);
        if (hit) return hit;
        // 離線中に画面を開き直したときの保険
        if (e.request.mode === "navigate") {
          const shell = await caches.match("/index.html");
          if (shell) return shell;
        }
        return Response.error();
      })
  );
});

// 通知の受け取りと表示．タップされたら該当の実行画面を開く．
self.addEventListener("push", (e) => {
  let title = "sai-phone";
  let body = "";
  let url = "/";
  try {
    const data = e.data ? e.data.json() : {};
    if (data.title) title = data.title;
    if (data.body) body = data.body;
    if (data.url) url = data.url;
  } catch { /* 壊れた通知でも落とさない */ }
  e.waitUntil(self.registration.showNotification(title, { body, data: { url } }));
});

// 通知を押したときの飛び先を覚えておく倉庫（アプリが閉じていて開き直すときに使う）
const OPEN_CACHE = "sai-phone-open";
const OPEN_KEY = "/__notification_open";

self.addEventListener("notificationclick", (e) => {
  e.notification.close();
  const url = (e.notification.data && e.notification.data.url) || "/";
  e.waitUntil((async () => {
    // iOS は開き直すと start_url（/）で起動することがあるため，飛び先を倉庫にも残す
    // （アプリ側が起動時に読んで消す・app.js の openFromNotification）
    try {
      const cache = await caches.open(OPEN_CACHE);
      await cache.put(OPEN_KEY, new Response(JSON.stringify({ url, at: Date.now() })));
    } catch { /* 残せなくても下の方法で開く */ }
    const clients = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    for (const c of clients) {
      // iOS の PWA では WindowClient.navigate が効かず前の画面のままになるため，
      // アプリに飛び先を伝えて自分で移ってもらう（届かないときは倉庫の飛び先で移る）
      c.postMessage({ type: "open", url });
      if ("focus" in c) await c.focus().catch(() => {});
      return undefined;
    }
    await self.clients.openWindow(url).catch(() => {});
    return undefined;
  })());
});

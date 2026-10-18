/* agent-link-web の Service Worker．
   静的ファイル（画面の部品）の取得のみ助ける．
   会話データはキャッシュしない（古い会話が残ると状況を誤解して危険なため）．

   画面ファイル（index.html / app.js / style.css など）は
   **ネットワーク優先・失敗したらキャッシュ**。
   以前はキャッシュ優先でキャッシュ名が固定のため，サーバーを直しても
   iPhone に古い app.js が残っていた。キャッシュ名はバージョンを上げ，
   activate 時に古いバージョンを消す（04 で /stats.js，05 で v4，06 で /project.js と v5，
   07 で /papers.js と v6，08 で v7＝Discover と過去の検索結果，09 で /radio.js と v8＝Radio）。
   09 からダウンロードした音声は別々の倉庫（agent-link-web-radio-audio）に保存し，
   再生用に Range の部分取り（206）もここで返す。 */

// 版の履歴は git log を見る
const CACHE = "agent-link-web-v140";
const ASSETS = ["/", "/index.html", "/style.css", "/markdown.css", "/home.css", "/workflow.css", "/defs.css", "/markdown.js", "/ui.js", "/app.js", "/chatorder.js", "/newchat.js", "/stats.js", "/project.js", "/linkview.js", "/cardform.js", "/sessionsearch.js", "/voiceask.js", "/canvaslayout.js", "/home.js", "/workflow.js", "/defs.js", "/manifest.json", "/icons/icon-192.png", "/icons/icon-512.png"];

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(ASSETS)).then(() => self.skipWaiting()));
});

// 古い倉庫かどうかを 1 か所で決める。
// sai-moon の倉庫（`sai-phone-` で始まる名前と `sai-web-v` で始まる名前）はすべて古い。
// `agent-link-web-v` で始まり今の版でない倉庫も古い。通知の飛び先（agent-link-web-open）は版の名前ではないので消さない。
function swIsOldCache(name, current) {
  if (name.startsWith("sai-phone-") || name.startsWith("sai-web-v")) return true;
  return name.startsWith("agent-link-web-v") && name !== current;
}

// 古い倉庫だけをここで消す。
self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(
        keys.filter((k) => swIsOldCache(k, CACHE)).map((k) => caches.delete(k))
      )
    ).then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
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
        const hit = await caches.match(e.request, { ignoreSearch: true });
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
  let title = "agent-link";
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
const OPEN_CACHE = "agent-link-web-open";
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

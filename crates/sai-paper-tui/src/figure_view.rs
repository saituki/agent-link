//! PAPER 画面の FIGURES セクション用の画像表示。
//!
//! 内蔵 PDF ビューア（[`crate::pdf_viewer`]）と同じ `ratatui-image` の仕組みを使う。
//! 新たな画像ライブラリは追加しない。
//!
//! プロトコル（Kitty / iTerm2 / Sixel / ハーフブロック）の判定は
//! `Picker::from_query_stdio()` に任せ、自前で判定しない。
//!
//! 画像は `(<画像パス>, <表示領域>)` の組でエンコード結果をキャッシュする。
//! 図の再抽出でパスが変わった場合は [`retain_paths`] で古いエントリを捨てる。

use std::collections::HashMap;
use std::io::IsTerminal;
use std::sync::{Mutex, OnceLock};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

/// 図画像のプロトコルキャッシュ。
struct FigureImages {
    /// 端末のプロトコル判定（初回描画時に 1 度だけ実行）。
    picker: Option<Picker>,
    /// 画像パス → エンコード済みプロトコル。
    protocols: HashMap<String, StatefulProtocol>,
}

static IMAGES: OnceLock<Mutex<FigureImages>> = OnceLock::new();

fn images() -> &'static Mutex<FigureImages> {
    IMAGES.get_or_init(|| {
        Mutex::new(FigureImages {
            picker: Picker::from_query_stdio().ok(),
            protocols: HashMap::new(),
        })
    })
}

/// 図画像のプロトコル判定を先に済ませる。
///
/// `Picker::from_query_stdio()` は非応答端末で最大 1 秒ブロックするため、
/// 初回描画ではなく起動直後（代替スクリーン確立後）に 1 度だけ実行する。
pub fn init() {
    if !std::io::stdout().is_terminal() {
        return;
    }
    let _ = images();
}

/// 図画像を `area` に描画する。
///
/// 端末が画像プロトコルに対応していない場合や、画像ファイルが読めない場合は
/// 何も描画しない（呼び出し側は代替テキストを別途表示すること）。
pub fn render_figure_image(frame: &mut Frame<'_>, area: Rect, image_path: &str) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // 端末以外（テストの TestBackend 等）ではプロトコルを持たないので描画しない。
    if !std::io::stdout().is_terminal() {
        return;
    }
    let mut guard = images().lock().unwrap_or_else(|e| e.into_inner());
    if !guard.protocols.contains_key(image_path) {
        let proto = image::open(image_path)
            .ok()
            .and_then(|img| guard.picker.as_ref().map(|p| p.new_resize_protocol(img)));
        if let Some(proto) = proto {
            guard.protocols.insert(image_path.to_string(), proto);
        }
    }
    let Some(protocol) = guard.protocols.get_mut(image_path) else {
        return;
    };
    // `Fit` は縦横比を保って領域内に収める。
    frame.render_stateful_widget(
        StatefulImage::new().resize(Resize::Fit(None)),
        area,
        protocol,
    );
}

/// `keep` に含まれない画像のキャッシュを破棄する（図の再抽出後など）。
pub fn retain_paths(keep: &[String]) {
    let mut guard = images().lock().unwrap_or_else(|e| e.into_inner());
    guard.protocols.retain(|path, _| keep.contains(path));
}

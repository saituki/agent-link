//! エッジ描画モジュール。
//!
//! レイアウト計算済みのノード位置情報に基づき、罫線文字（`│─╰╯╭╮┌┐└┘`）と
//! 矢印（`▼▲▶◀`）で card（ブロック）間接続を描画します。
//!
//! ルーティング方針（ekphos 方式・`docs/block-position-rule.md` 準拠）:
//! - card の**辺（枠線）には一切重ね描きしない**。接続線は card の**外側**で終端し、
//!   card 自身は上辺・側辺とも常に無傷（`───` / `│`）のまま
//!   （旧 `GroundDot` によるノード描画後の上書きは廃止）
//! - 接続線は長方形の中に**入り込まない**（水平セグメントはノード間のギャップ内にクランプ）
//! - 縦方向エッジ（main 同士の waggle_token 等）: 出力元の下端中点 → 入力先の上端中点。
//!   入力先ごとに「入力先の**2 行上**」に**バス（水平幹線）**を引き、入力元の縦線は
//!   自分の列をまっすぐ下りてバスで合流する（**ターゲット側バス**）。
//!   バスと card の間の 1 行（上辺の直前）には、分岐の有無によらず**常に矢印**
//!   （`▼`/`▲`）を置く。
//!   - 入力元の合流点は T 字（`├` `┤`）またはコーナー（`╰` `╯`）
//!   - バスは分岐・合流があるときだけ入力先の中点で折れ曲がる（`┬` 等）。
//!     直進のみのバスには折れ曲がり文字を置かない
//!   - バス行を**通過するだけ**の縦線（別の入力先へ向かう線）は `┼` で交差する
//!   - これにより、途中の行を横切る偽の縦線（例: ID8→ID13 が ID7→ID9 の
//!     幹線に合流して見える問題）は発生しない
//! - 横方向エッジ（ブロック-サブブロック）: main の右辺中点 ↔ サブの左辺中点。
//!   `task_delegation`（main→sub）と `task_output`（sub→main）は同じ経路を通るため、
//!   双方向がある場合は**1本の接続線**に統合する（矢印は両端それぞれの色で描く）。
//!   出力元側のコーナーは T 字（`┐` `└` 等）で描画し、十字 `┼` に貫通させない。
//!   入力先（子）側は、card の直前 1 セルに常に矢印（`▶`/`◀`）を置いて終端する

use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
};

use sai_link_widgets::workflow::{DagGraph, DagLayout, NodeId, NodePosition, SeparatorLine};

// ============================================================================
// 内部構造体
// ============================================================================

/// 1本のエッジのルーティング情報。
struct EdgeRoute {
    /// 出力元ノード ID（相互接続のペア判定用）
    from_id: NodeId,
    /// 入力先ノード ID（相互接続のペア判定用）
    to_id: NodeId,
    from_x: i32,
    from_y: i32,
    to_x: i32,
    to_y: i32,
    /// 左右辺接地（ブロック-サブブロック）かどうか
    side: bool,
    style: Style,
    /// 接続種別（バス行の段差割り当てに使用）
    kind: sai_link_widgets::workflow::EdgeKind,
}

// ============================================================================
// パブリック API
// ============================================================================

/// 全エッジを描画する。card の辺（枠線）には一切重ね描きしない
/// （ekphos 方式: 接続点は card の**外側 1 セル**に矢印を置くだけで、
/// card 自身の枠は `───` / `│` のまま無傷を保つ）。
///
/// `selected` にノード ID を渡すと、そのノードに接続する全エッジが
/// ハイライト色（黄色）で描画される（選択ブロックの接続線の強調用）。
pub fn render_edges<N>(
    dag: &DagGraph<N>,
    layout: &DagLayout,
    scroll_x: i32,
    scroll_y: i32,
    top_down: bool,
    selected: Option<NodeId>,
    viewport: Rect,
    buf: &mut Buffer,
) {
    EdgeRenderer {
        scroll_x,
        scroll_y,
        top_down,
        selected,
    }
    .render(dag, layout, viewport, buf)
}

/// ラウンド区切り線（`roundN────...`）を描画する。
///
/// GR が変わる行グループの上に、ラベルを左端、その後ろにコンテンツ幅いっぱいの
/// 水平線を描画する。ラベル部分が占めるスクリーン座標の矩形を
/// `(global_round, Rect)` として返す（呼び出し側がドラッグの当たり判定に使う。
/// 線全体ではなくラベル部分だけを当たり判定にすることで、card のドラッグと
/// 取り合いにならないようにする）。
pub fn render_separators(
    separators: &[SeparatorLine],
    content_width: i32,
    scroll_x: i32,
    scroll_y: i32,
    viewport: Rect,
    buf: &mut Buffer,
) -> Vec<(i64, Rect)> {
    let style = Style::default().fg(Color::Blue);
    let mut label_rects = Vec::new();

    for sep in separators {
        let screen_y = sep.y - scroll_y + i32::from(viewport.y);
        if screen_y < i32::from(viewport.y) || screen_y >= i32::from(viewport.y + viewport.height) {
            continue;
        }

        // ラベルを左端に描画
        let mut x = 0i32;
        for ch in sep.label.chars() {
            draw_char_plain(
                buf,
                x - scroll_x + i32::from(viewport.x),
                screen_y,
                ch,
                style,
                viewport,
            );
            x += 1;
        }

        // ラベル矩形（当たり判定用。ビューポート内にクランプする）
        let label_left = (0 - scroll_x + i32::from(viewport.x)).max(i32::from(viewport.x));
        let label_right =
            (x - scroll_x + i32::from(viewport.x)).min(i32::from(viewport.x + viewport.width));
        if label_right > label_left && screen_y >= 0 {
            label_rects.push((
                sep.global_round,
                Rect::new(
                    label_left as u16,
                    screen_y as u16,
                    (label_right - label_left) as u16,
                    1,
                ),
            ));
        }

        // ラベルの後ろからパネル右端（content と viewport の広い方）まで水平線。
        // content_width（ノード右端）よりパネルが広い場合も右端まで届かせる。
        let right = content_width
            .max(scroll_x + i32::from(viewport.width) - 1);
        for x2 in x..=right {
            draw_char_plain(
                buf,
                x2 - scroll_x + i32::from(viewport.x),
                screen_y,
                '─',
                style,
                viewport,
            );
        }
    }

    label_rects
}

// ============================================================================
// ヘルパー関数
// ============================================================================

/// 親ノード側の Y 座標（縦方向エッジの開始点）を返す。
fn parent_y(pos: &NodePosition, top_down: bool) -> i32 {
    if top_down {
        pos.y + i32::from(pos.height) // ノードの下端
    } else {
        pos.y // ノードの上端
    }
}

/// 子ノード側の Y 座標（縦方向エッジの終了点）を返す。
fn child_y(pos: &NodePosition, top_down: bool) -> i32 {
    if top_down {
        pos.y // ノードの上端
    } else {
        pos.y + i32::from(pos.height) // ノードの下端
    }
}

/// 2つの X 座標を小さい順に並べる。
fn ordered_x_pair(a: i32, b: i32) -> (i32, i32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// 罫線文字 → 4 方向の線のビットマスク（上|下|左|右 = 0b1000|0b0100|0b0010|0b0001）。
///
/// Unicode のボックス描画の幾何に合わせる:
/// - `┬` は横棒 + 下向き軸、`┴` は横棒 + 上向き軸
/// - `├` は縦棒 + 右向きバー、`┤` は縦棒 + 左向きバー
/// - コーナーは `┌`(下+右) `┐`(下+左) `└`(上+右) `┘`(上+左)
fn line_mask(ch: char) -> u8 {
    match ch {
        '│' => 0b1100,
        '─' => 0b0011,
        '┌' | '╭' => 0b0101,
        '┐' | '╮' => 0b0110,
        '└' | '╰' => 0b1001,
        '┘' | '╯' => 0b1010,
        '├' => 0b1101,
        '┤' => 0b1110,
        '┬' => 0b0111,
        '┴' => 0b1011,
        '┼' => 0b1111,
        _ => 0,
    }
}

/// 4 方向の線のビットマスク → 罫線文字。
fn mask_to_line(mask: u8) -> char {
    match mask {
        0b1100 => '│',
        0b0011 => '─',
        0b0101 => '╭',
        0b0110 => '╮',
        0b1001 => '╰',
        0b1010 => '╯',
        0b1101 => '├',
        0b1110 => '┤',
        0b0111 => '┬',
        0b1011 => '┴',
        0b1111 => '┼',
        _ => ' ',
    }
}

/// 既存セルの罫線文字と新しく描く文字を合成する。
///
/// 2 本の接続線が交差・合流する位置では、描画順序に関わらず
/// 正しい罫線文字（┼ / ├ / ┤ など）になるよう、線の向きを OR で合成する。
fn compose_chars(existing: char, new: char) -> char {
    mask_to_line(line_mask(existing) | line_mask(new))
}

// ============================================================================
// EdgeRenderer — エッジ描画の実体
// ============================================================================

struct EdgeRenderer {
    scroll_x: i32,
    scroll_y: i32,
    top_down: bool,
    /// 選択中ノード ID。このノードに接続するエッジはハイライト色で描画する。
    selected: Option<NodeId>,
}

/// 選択中ブロックに接続するエッジのハイライト色（ブロック選択時の文字色に合わせる）。
const HIGHLIGHT_FG: Color = Color::Yellow;

impl EdgeRenderer {
    /// ルートが選択中ノードに接続する場合はハイライト色を返す。
    fn effective_style(
        selected: Option<NodeId>,
        from_id: NodeId,
        to_id: NodeId,
        base: Style,
    ) -> Style {
        if selected == Some(from_id) || selected == Some(to_id) {
            Style::default().fg(HIGHLIGHT_FG)
        } else {
            base
        }
    }
    fn render<N>(
        &self,
        dag: &DagGraph<N>,
        layout: &DagLayout,
        viewport: Rect,
        buf: &mut Buffer,
    ) {
        let routes: Vec<EdgeRoute> = layout
            .nodes
            .iter()
            .flat_map(|pos| self.routes_from_node(pos, dag, layout))
            .collect();

        // 分岐行は出力元の直下に固定するため、エッジごとのチャネル計算は不要。
        // 同じ行に集まるエッジは同じ分岐行を通り、1 本のバスに合成される

        if std::env::var("DAG_DEBUG").is_ok() {
            for r in &routes {
                let bus_row = if r.side {
                    -1
                } else if r.from_y < r.to_y {
                    r.to_y - 2
                } else {
                    r.to_y + 2
                };
                eprintln!(
                    "[edge] {}->{} from=({},{}) to=({},{}) side={} bus_row={}",
                    r.from_id, r.to_id, r.from_x, r.from_y, r.to_x, r.to_y, r.side, bus_row
                );
            }
        }

        // 左右辺（サイド）エッジの (from_id, to_id) → インデックス
        let mut side_index: HashMap<(NodeId, NodeId), usize> = HashMap::new();
        for (idx, route) in routes.iter().enumerate() {
            if route.side {
                side_index.insert((route.from_id, route.to_id), idx);
            }
        }

        let mut consumed = vec![false; routes.len()];

        // ── 左右辺エッジ（ブロック-サブブロック）──
        // card の枠（辺）には触れない。3本以上の main が横並びチェーンで連なる場合、
        // 遠い main への長い直線（例: 3番目の sub → main の task_output）が途中の
        // gap を通過するため、先に**全ての経路線**を描いてから、矢印・分岐文字を
        // **まとめて最後に**重ねる（後から描く経路線が先に描いた矢印を消さないため）。
        let mut arrow_routes: Vec<&EdgeRoute> = Vec::new();
        let mut out_marker_routes: Vec<&EdgeRoute> = Vec::new();
        for (idx, route) in routes.iter().enumerate() {
            if consumed[idx] {
                continue;
            }
            consumed[idx] = true;

            if route.side {
                // 相互接続（双方向）: 同じペアの逆方向エッジがあれば経路は1本に統合する。
                // 両端とも「受け取る側」なので、分岐文字は使わず両端に矢印を描く
                // （各方向の色で。中央の経路線は1本のみ描画）。
                if let Some(&rev_idx) = side_index.get(&(route.to_id, route.from_id)) {
                    if rev_idx != idx && routes[rev_idx].side && !consumed[rev_idx] {
                        consumed[rev_idx] = true;
                        self.draw_side_path(route, viewport, buf);
                        arrow_routes.push(route);
                        arrow_routes.push(&routes[rev_idx]);
                        continue;
                    }
                }
                self.draw_side_path(route, viewport, buf);
                out_marker_routes.push(route);
                arrow_routes.push(route);
            }
        }
        // 矢印より分岐文字（出力側マーカー）を後に描く: 稀に別 main への長距離
        // エッジ（例: 3番目の sub から main への直接 task_output）の出力側マーカーが
        // 別エッジの矢印と同じセルに重なるケースがあり、分岐の起点を示す
        // マーカーを優先して見せる。
        for route in &arrow_routes {
            self.draw_side_arrow(route, viewport, buf);
        }
        for route in &out_marker_routes {
            self.draw_side_out_marker(route, viewport, buf);
        }

        // ── 縦方向エッジ（main 同士）── 入力先ごとにターゲット側バスで合流 ──
        self.draw_vertical_edges(&routes, viewport, buf);
    }

    /// 1つのノードから出る全エッジのルーティングを計算する。
    fn routes_from_node<'a, N>(
        &self,
        pos: &'a NodePosition,
        dag: &'a DagGraph<N>,
        layout: &'a DagLayout,
    ) -> impl Iterator<Item = EdgeRoute> + 'a {
        let top_down = self.top_down;
        let selected = self.selected; // Copy: クロージャのキャプチャ用

        dag.edges_from(pos.index)
            .filter_map(move |(child_idx, edge_data)| {
                let child_pos = layout.find(child_idx)?;
                let from_id = pos.node_id;
                let to_id = child_pos.node_id;
                let base_style = edge_data.style.unwrap_or_default();
                // 選択中ブロックに接続するエッジはハイライト色にする
                let style = Self::effective_style(selected, from_id, to_id, base_style);

                // 左右辺接地（サイド）か、上下辺接地（縦）かは EdgeData.side で判定
                // （幾何位置ではなく、接続種別・サブブロック関係に基づく）
                let (from_x, from_y, to_x, to_y) = if edge_data.side {
                    // 辺の中点 = 実際のボーダーセル（右辺は x + width - 1）
                    let parent_right = pos.x + i32::from(pos.width) - 1;
                    let child_right = child_pos.x + i32::from(child_pos.width) - 1;
                    if child_pos.x >= parent_right {
                        // 子が右側: 親の右辺中点 → 子の左辺中点
                        (
                            parent_right,
                            pos.y + i32::from(pos.height) / 2,
                            child_pos.x,
                            child_pos.y + i32::from(child_pos.height) / 2,
                        )
                    } else {
                        // 子が左側: 親の左辺中点 → 子の右辺中点
                        (
                            pos.x,
                            pos.y + i32::from(pos.height) / 2,
                            child_right,
                            child_pos.y + i32::from(child_pos.height) / 2,
                        )
                    }
                } else {
                    (
                        pos.x + i32::from(pos.width) / 2,
                        parent_y(pos, top_down),
                        child_pos.x + i32::from(child_pos.width) / 2,
                        child_y(child_pos, top_down),
                    )
                };

                Some(EdgeRoute {
                    from_id,
                    to_id,
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    side: edge_data.side,
                    style,
                    kind: edge_data.kind,
                })
            })
    }

    /// 縦方向エッジ（上下辺接地）を入力先ごとに束ねて描画する（ターゲット側バス）。
    ///
    /// 入力先の**上辺の 2 行上**をバス行（水平幹線）とし、入力元の縦線は自分の列を
    /// まっすぐ下りてバスで合流する。バスと card の間の 1 行（上辺の直前）には
    /// 分岐の有無によらず**常に矢印**（`▼`/`▲`）を置く。card の辺（枠線）には
    /// 一切重ね描きしない（ekphos 方式。node 描画後の上書きも不要）。
    ///
    /// - **接続種別（EdgeKind）ごとにバス行を段差させる**: 同じ入力先に異なる種別の
    ///   接続（例: フェーズ鎖 cyan + task_output green + waggle magenta）が集まる場合、
    ///   同じバス行に混ぜると水平線の色が 1 種類に上書きされ区別できないため、
    ///   種別ごとに 1 行ずつ上（上方向接続は下）へずらして別バスとする。
    ///   直進する縦線（from_x == to_x、例: フェーズ鎖の ID6 → P_1）を含む種別を
    ///   最も入力先に近いバス行（offset 0）とし、他は 1 行ずつ遠ざける。
    /// - 入力元の合流点: 縦線がこのバスより先へ伸びるなら T 字（`├` `┤` `┼`）、
    ///   ここで終わるならコーナー（`╰` `╯` `┴`）
    /// - バス行を通過するだけの縦線（別の入力先へ向かう線）は `┼` で交差する
    fn draw_vertical_edges<'a>(&self, routes: &[EdgeRoute], viewport: Rect, buf: &mut Buffer) {
        use std::collections::HashMap;

        let mut verts: Vec<&EdgeRoute> = routes.iter().filter(|r| !r.side).collect();
        if verts.is_empty() {
            return;
        }
        // 決定論的な描画順を保証する（並行エッジが同じセルを共有するとき、
        // どのエッジの色が勝つかを固定して「片方のみ」点滅させず表示する）。
        // to_id 昇順にソートし、重なり合う縦線は to_id の大きな（後から描く）
        // エッジの色で統一する。これにより HashMap の反復順に依存せず、
        // 毎回同じエッジの色が選ばれる。
        verts.sort_by(|a, b| a.to_id.cmp(&b.to_id).then(a.from_id.cmp(&b.from_id)));

        // ① (to_id, 基準バス行) ごとに仮グループ化し、種別ごとのオフセットを決める
        #[derive(Clone, Copy)]
        struct Assign<'a> {
            route: &'a EdgeRoute,
            bus_row: i32,
            down: bool,
        }
        let mut base_groups: HashMap<(NodeId, i32), Vec<&EdgeRoute>> = HashMap::new();
        for r in &verts {
            let down = r.from_y < r.to_y;
            let base = if down { r.to_y - 2 } else { r.to_y + 2 };
            base_groups.entry((r.to_id, base)).or_default().push(r);
        }

        // 種別ごとの実際のバス行を決定し、全ルートへ割り当てる
        let mut assignments: Vec<Assign> = Vec::with_capacity(verts.len());
        let mut base_keys: Vec<(NodeId, i32)> = base_groups.keys().cloned().collect();
        base_keys.sort();
        for &(to_id, base) in &base_keys {
            let rs = &base_groups[&(to_id, base)];
            let down = rs[0].from_y < rs[0].to_y;
            // 種別ごとにまとめる（kinds: 種別 → ルート群）
            let mut kinds: Vec<(sai_link_widgets::workflow::EdgeKind, Vec<&EdgeRoute>)> = Vec::new();
            for r in rs {
                match kinds.iter_mut().find(|(k, _)| *k == r.kind) {
                    Some((_, list)) => list.push(r),
                    None => kinds.push((r.kind, vec![r])),
                }
            }
            // オフセット順: 直進する縦線（from_x == to_x）を含む種別を offset 0、
            // 他は種別 enum 順（決定論的）で 1, 2, ... と遠ざける
            kinds.sort_by_key(|(k, list)| {
                let straight = list.iter().any(|r| r.from_x == r.to_x);
                (if straight { 0 } else { 1 }, *k)
            });
            for (offset, (_, list)) in kinds.iter().enumerate() {
                for r in list {
                    let bus_row = if down {
                        base - offset as i32
                    } else {
                        base + offset as i32
                    };
                    assignments.push(Assign {
                        route: r,
                        bus_row,
                        down,
                    });
                }
            }
        }

        // ② (to_id, 実バス行) ごとに再グループ化 = 同じバスに合流する
        let mut groups: HashMap<(NodeId, i32), Vec<&Assign>> = HashMap::new();
        // 出力元ごとの「最遠」バス行（下方向は最大、上方向は最小）:
        // 縦線がこのバスより先へ伸びるか（合流点の T字/コーナー判定）に使う
        let mut ext_down: HashMap<NodeId, i32> = HashMap::new();
        let mut ext_up: HashMap<NodeId, i32> = HashMap::new();

        for a in &assignments {
            groups
                .entry((a.route.to_id, a.bus_row))
                .or_default()
                .push(a);
            if a.down {
                let e = ext_down.entry(a.route.from_id).or_insert(a.bus_row);
                *e = (*e).max(a.bus_row);
            } else {
                let e = ext_up.entry(a.route.from_id).or_insert(a.bus_row);
                *e = (*e).min(a.bus_row);
            }
        }

        // グループも決定論的な順序で処理する（key = (to_id, bus_row) でソート）。
        // 縦線が別グループのエッジと重なる場合も、常に同じ順序で描いて
        // 「片方のみ」の色を安定表示する。
        let mut group_keys: Vec<(NodeId, i32)> = groups.keys().cloned().collect();
        group_keys.sort();
        for &(_to_id, bus_row) in &group_keys {
            let rs = &groups[&(_to_id, bus_row)];
            let to_x = rs[0].route.to_x;
            let down = rs[0].down;
            let style = rs[0].route.style;

            // バスの水平範囲（全出力元の列と入力先の中点を含む）
            let mut min_x = to_x;
            let mut max_x = to_x;
            for a in rs.iter() {
                min_x = min_x.min(a.route.from_x);
                max_x = max_x.max(a.route.from_x);
            }

            // 1. 各出力元の縦線（自分の列をバス行までまっすぐ下りる。card の辺には触れない）
            for a in rs.iter() {
                let (r, y1, y2) = if down {
                    (a.route, a.route.from_y, bus_row)
                } else {
                    (a.route, bus_row, a.route.from_y - 1)
                };
                self.draw_vertical_line(r.from_x, y1, y2, r.style, '│', viewport, buf);
            }

            // 2. バス本体（合流点・中点を除いた横線）
            let mut special_cols: Vec<i32> = rs.iter().map(|a| a.route.from_x).collect();
            special_cols.push(to_x);
            for x in min_x..=max_x {
                if special_cols.contains(&x) {
                    continue;
                }
                self.draw_char_composed(x, bus_row, style, '─', viewport, buf);
            }

            // 3. 出力元の合流点（T字 / コーナー）: 同じ列に複数の縦線が重なる場合は
            //    列ごとに 1 回だけ描画し、継続判定はその列の全ルートを OR する
            let mut join_cols: Vec<i32> = rs.iter().map(|a| a.route.from_x).collect();
            join_cols.sort_unstable();
            join_cols.dedup();
            for sx in join_cols {
                let continues = verts.iter().any(|r| {
                    r.from_x == sx
                        && if down {
                            ext_down.get(&r.from_id).copied().unwrap_or(bus_row) > bus_row
                        } else {
                            ext_up.get(&r.from_id).copied().unwrap_or(bus_row) < bus_row
                        }
                });
                let left = sx > min_x;
                let right = sx < max_x;
                let ch = match (continues, left, right) {
                    (true, true, true) => '┼',
                    (true, true, false) => '┤',
                    (true, false, true) => '├',
                    (false, true, true) => '┴',
                    (false, true, false) => '╯',
                    (false, false, true) => '╰',
                    _ => '│',
                };
                let style = rs
                    .iter()
                    .find(|a| a.route.from_x == sx)
                    .map(|a| a.route.style)
                    .unwrap_or(style);
                self.draw_char(sx, bus_row, style, ch, viewport, buf);
            }

            // 4. 入力先の中点（バス行での折れ曲がり）: 分岐・合流があるときだけ描く
            let left = to_x > min_x;
            let right = to_x < max_x;
            let stem = if down {
                match (left, right) {
                    (true, true) => '┬',
                    (true, false) => '┐',
                    (false, true) => '┌',
                    (false, false) => '│', // 直線（バスなし）: 縦線のまま
                }
            } else {
                match (left, right) {
                    (true, true) => '┴',
                    (true, false) => '┘',
                    (false, true) => '╰',
                    (false, false) => '│',
                }
            };
            if stem != '│' {
                self.draw_char_composed(to_x, bus_row, style, stem, viewport, buf);
            }

            // 5. card の直上/直下（バスと card の間）には分岐の有無によらず常に矢印を置く。
            //    card の辺自体には一切触れない。
            self.draw_char(
                to_x,
                if down { bus_row + 1 } else { bus_row - 1 },
                style,
                if down { '▼' } else { '▲' },
                viewport,
                buf,
            );
        }
    }

    /// 左右辺エッジ（ブロック-サブブロック）の経路（線分・出力元側コーナー）のみを描画する。
    ///
    /// card の辺（枠線）には触れない。出力元側のコーナーは**線分から除外**して
    /// 別途合成描画するため、十字 `┼` に貫通せず T 字（`┐` `└` 等）のままになる。
    /// 枠外マーカー（`draw_side_out_marker` / `draw_side_arrow`）は呼び出し側
    /// （`render()`）が、**全ルートの経路線を描き終えた後にまとめて**付ける
    /// （後から描く経路線が先に描いたマーカーを上書きしないようにするため）。
    fn draw_side_path(&self, route: &EdgeRoute, viewport: Rect, buf: &mut Buffer) {
        let px = route.from_x;
        let py = route.from_y;
        let cx = route.to_x;
        let cy = route.to_y;
        let style = route.style;

        let going_right = cx >= px;

        if py == cy {
            // 同じ高さ: 直線（端点は card の辺中点 = card 描画で上書きされる）
            let (x1, x2) = ordered_x_pair(px, cx);
            self.draw_horizontal_line(x1, x2, py, style, '─', viewport, buf);
        } else {
            // バス（垂直幹線）: 子の辺から 1 マス手前
            let bus_x = if going_right { cx - 1 } else { cx + 1 };

            // 線分はコーナーセルを除いて描画する（出力元側のコーナーは最後に合成。
            // 入力先側のコーナーは矢印に置き換わるため描かない）
            if going_right {
                self.draw_horizontal_line(px, bus_x - 1, py, style, '─', viewport, buf);
                if py < cy - 1 {
                    self.draw_vertical_line(bus_x, py + 1, cy - 1, style, '│', viewport, buf);
                }
                self.draw_horizontal_line(bus_x + 1, cx, cy, style, '─', viewport, buf);
            } else {
                self.draw_horizontal_line(bus_x + 1, px, py, style, '─', viewport, buf);
                if py < cy - 1 {
                    self.draw_vertical_line(bus_x, py + 1, cy - 1, style, '│', viewport, buf);
                }
                self.draw_horizontal_line(cx, bus_x - 1, cy, style, '─', viewport, buf);
            }

            // 出力元側のコーナー（上下の向きに応じて正しい字形を選択）
            let up = cy < py;
            let c1 = match (going_right, up) {
                (true, false) => '┐',  // 右・下
                (true, true) => '└',   // 右・上
                (false, false) => '┌', // 左・下
                (false, true) => '└',  // 左・上
            };
            self.draw_char_composed(bus_x, py, style, c1, viewport, buf);
        }
    }

    /// 左右辺エッジの出力元（親）側、card 枠の 1 セル外側に分岐文字（`├`/`┤`）を置く。
    ///
    /// 単方向（非相互接続）のときだけ呼ぶ。相互接続（双方向）は両端とも
    /// 「受け取る側」なので、分岐文字ではなく `draw_side_arrow` を両端に使う。
    fn draw_side_out_marker(&self, route: &EdgeRoute, viewport: Rect, buf: &mut Buffer) {
        let going_right = route.to_x >= route.from_x;
        let out_x = if going_right {
            route.from_x + 1
        } else {
            route.from_x - 1
        };
        self.draw_char(
            out_x,
            route.from_y,
            route.style,
            if going_right { '├' } else { '┤' },
            viewport,
            buf,
        );
    }

    /// 左右辺エッジの入力先（子）側の終端に、card の外側 1 セルへ矢印を描く。
    fn draw_side_arrow(&self, route: &EdgeRoute, viewport: Rect, buf: &mut Buffer) {
        let going_right = route.to_x >= route.from_x;
        let arrow_x = if going_right {
            route.to_x - 1
        } else {
            route.to_x + 1
        };
        self.draw_char(
            arrow_x,
            route.to_y,
            route.style,
            if going_right { '▶' } else { '◀' },
            viewport,
            buf,
        );
    }

    /// 1文字をスクリーン座標に描画する（ビューポートクリッピング付き）。
    fn draw_char(
        &self,
        vx: i32,
        vy: i32,
        style: Style,
        ch: char,
        viewport: Rect,
        buf: &mut Buffer,
    ) {
        draw_char_plain(
            buf,
            vx - self.scroll_x + i32::from(viewport.x),
            vy - self.scroll_y + i32::from(viewport.y),
            ch,
            style,
            viewport,
        );
    }

    /// 既存のセル内容と罫線文字を合成して描画する。
    ///
    /// 交差・合流位置では描画順序に関わらず正しい罫線文字になる。
    /// （例: 別エッジの垂直線の上にコーナーを描くと ├ / ┤ に合成される）
    fn draw_char_composed(
        &self,
        vx: i32,
        vy: i32,
        style: Style,
        ch: char,
        viewport: Rect,
        buf: &mut Buffer,
    ) {
        let sx = vx - self.scroll_x + i32::from(viewport.x);
        let sy = vy - self.scroll_y + i32::from(viewport.y);
        if sx < i32::from(viewport.x)
            || sx >= i32::from(viewport.x + viewport.width)
            || sy < i32::from(viewport.y)
            || sy >= i32::from(viewport.y + viewport.height)
        {
            return;
        }
        if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
            let existing = cell.symbol().chars().next().unwrap_or(' ');
            cell.set_char(compose_chars(existing, ch)).set_style(style);
        }
    }

    /// 垂直線を描画する（既存の罫線と合成する）。
    fn draw_vertical_line(
        &self,
        x: i32,
        y1: i32,
        y2: i32,
        style: Style,
        ch: char,
        viewport: Rect,
        buf: &mut Buffer,
    ) {
        let (start, end) = if y1 < y2 { (y1, y2) } else { (y2, y1) };
        for y in start..=end {
            self.draw_char_composed(x, y, style, ch, viewport, buf);
        }
    }

    /// 水平線を描画する（既存の罫線と合成する）。
    fn draw_horizontal_line(
        &self,
        x1: i32,
        x2: i32,
        y: i32,
        style: Style,
        ch: char,
        viewport: Rect,
        buf: &mut Buffer,
    ) {
        for x in x1..=x2 {
            self.draw_char_composed(x, y, style, ch, viewport, buf);
        }
    }
}

/// スクリーン座標に 1 文字を描画する（クリッピング付き・常に上書き）。
fn draw_char_plain(buf: &mut Buffer, sx: i32, sy: i32, ch: char, style: Style, viewport: Rect) {
    if sx < i32::from(viewport.x)
        || sx >= i32::from(viewport.x + viewport.width)
        || sy < i32::from(viewport.y)
        || sy >= i32::from(viewport.y + viewport.height)
    {
        return;
    }
    if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
        cell.set_char(ch).set_style(style);
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 区切り線（roundN）は content_width よりパネルが広い場合も
    /// Workflow パネル右端まで描画されること（P4）。
    #[test]
    fn separator_reaches_viewport_right_edge() {
        // viewport 幅 40・content_width は 10（ノード右端）とする。
        // ラベル "round1"（6 文字）の後ろから右端まで線が伸びる。
        let area = Rect::new(0, 0, 40, 3);
        let mut buf = Buffer::empty(area);
        let separators = vec![crate::graph_layout::SeparatorLine {
            y: 0,
            label: "round1".to_string(),
            global_round: 1,
        }];
        render_separators(&separators, 10, 0, 0, area, &mut buf);

        // ラベル直後（x=6）は線
        assert_eq!(buf.cell((6, 0)).unwrap().symbol(), "─");
        // ラベル前半はラベル文字
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "r");
        // content_width 越え（x=20・30）も線が続く（旧実装では途切れていた）
        assert_eq!(buf.cell((20, 0)).unwrap().symbol(), "─");
        assert_eq!(buf.cell((30, 0)).unwrap().symbol(), "─");
        // 右端の 1 つ手前（x=39）まで届く
        assert_eq!(buf.cell((39, 0)).unwrap().symbol(), "─");
    }

    /// スクロール時もスクロール後のパネル右端まで描画されること。
    #[test]
    fn separator_follows_scroll_to_right_edge() {
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        let separators = vec![crate::graph_layout::SeparatorLine {
            y: 0,
            label: "round2".to_string(),
            global_round: 2,
        }];
        // scroll_x = 5: コンテンツ x=5 が画面左端に来る
        render_separators(&separators, 10, 5, 0, area, &mut buf);

        // 画面右端（x=19）まで線が描かれる
        assert_eq!(buf.cell((19, 0)).unwrap().symbol(), "─");
    }

    /// 分岐・合流のない縦方向エッジ（main→main）には向きを示す矢印が付くこと。
    #[test]
    fn vertical_edge_gets_direction_arrow() {
        use sai_link_widgets::workflow::{DagGraph, EdgeData, EdgeKind};

        let mut dag: DagGraph<()> = DagGraph::new();
        let top = dag.add_node(1, ());
        let bottom = dag.add_node(2, ());
        dag.add_edge(
            top,
            bottom,
            EdgeData {
                style: None,
                side: false,
                kind: EdgeKind::Other,
            },
        );

        let layout = DagLayout {
            nodes: vec![
                NodePosition {
                    index: top,
                    node_id: 1,
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 3,
                },
                NodePosition {
                    index: bottom,
                    node_id: 2,
                    x: 0,
                    y: 5,
                    width: 10,
                    height: 3,
                },
            ],
            content_width: 10,
            content_height: 8,
            separators: Vec::new(),
        };

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render_edges(&dag, &layout, 0, 0, true, None, area, &mut buf);

        // from_x=to_x=5（直進）、bus_row=to_y-1=4 に下向き矢印が重なる。
        assert_eq!(buf.cell((5, 4)).unwrap().symbol(), "▼");
    }

    /// 分岐・合流のない左右辺エッジ（main→sub）には向きを示す矢印が付くこと。
    #[test]
    fn side_edge_gets_direction_arrow() {
        use sai_link_widgets::workflow::{DagGraph, EdgeData, EdgeKind};

        let mut dag: DagGraph<()> = DagGraph::new();
        let main = dag.add_node(1, ());
        let sub = dag.add_node(2, ());
        dag.add_edge(
            main,
            sub,
            EdgeData {
                style: None,
                side: true,
                kind: EdgeKind::Delegation,
            },
        );

        let layout = DagLayout {
            nodes: vec![
                NodePosition {
                    index: main,
                    node_id: 1,
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 3,
                },
                NodePosition {
                    index: sub,
                    node_id: 2,
                    x: 14,
                    y: 0,
                    width: 10,
                    height: 3,
                },
            ],
            content_width: 24,
            content_height: 3,
            separators: Vec::new(),
        };

        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        render_edges(&dag, &layout, 0, 0, true, None, area, &mut buf);

        // 親右辺(x=9)→子左辺(x=14)、子の直前(x=13)・同じ高さ(y=1)に右向き矢印。
        assert_eq!(buf.cell((13, 1)).unwrap().symbol(), "▶");
    }
}

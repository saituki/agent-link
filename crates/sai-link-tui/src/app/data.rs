//! DB アクセスとデータ構築（app.rs から分割）。

use super::*;
use crate::ui::main_panel::MarkdownView;

// 移設先（sai-link-widgets）からの再エクスポート。
// `super::data::…` 参照（kanban.rs のテスト）を維持する。
#[cfg(test)]
pub(crate) use sai_link_widgets::card_entries::scan_todo_cards;

impl AppState {
    /// Reflection 記録の記入状況（Notification 色分け）を覚え置く。
    ///
    /// 判定本体は `sai_link_widgets::workflow::load_workflow` 側にある
    /// （`sai-link phone workflow` と共有）。ここは結果の覚え置きのみ。
    fn judge_reflection_statuses(
        &mut self,
        statuses: HashMap<i64, sai_link_engine::reflection_status::ReflectionStatus>,
    ) {
        self.reflection_status = statuses;
    }

    pub fn load_data(&mut self) {
        // 検索フォームの Project 行（L2）: 自プロジェクト名（.sai/project.json）を表示するのみ
        self.search.project_name = ProjectConfig::load().name;

        // 実行一覧・run フォルダ同期・mb 複製集合・todo カード走査・Kanban 札を 1 度だけ集める
        // （`sai-link phone workflow` / `phone tree` と同じ部品）。
        let prep = sai_link_widgets::card_entries::prepare_view(self.store.as_ref());
        // 実行バッチのメイン複製実行ID（run_batches.mb_execution_id）一覧。
        // これらの実行レコードは「メインブロック複製（mb1.x）」として表示する。
        // U13・R25: 左SB の実行一覧から除外するため AppState に保持し、
        // メイン画面（DAG）では引き続き mb_replica フラグで表示する。
        self.mb_replica_execs = prep.mb_replica_execs.clone();

        // Workflow（DAG）の材料（ブロック・接続・finalized 親・幽霊・費用・review 記入状況）。
        // 判定条件はすべて `sai-link-widgets::workflow::load_workflow` にあり、
        // `sai-link phone workflow` と同じ物を使う。
        let src = sai_link_widgets::workflow::load_workflow(self.store.as_ref(), &prep);
        // メイン画面用: 実行履歴ベースでノード構築（各実行を独立ノードとして表示）
        // Kanban / Workflow の排他：`d` 済み（`workflow_moved=true`）の実行だけ
        // Workflow の DAG に載せる。未移動は Kanban ボードに載せる。
        // ファイル（review / カード）は削除しない。
        self.block_displays = src.blocks;
        // sai-write 検出済み（finalized 実行バッチを持つ）実メインの集合。
        // メイン画面で最終フェーズノード（P_n = sai-write 由来）を合成する判定に使用。
        self.finalized_run_parents = src.finalized_runs;
        // P1-1: 幽霊（`status='running'` の取り残し）の実行 ID 集合。
        // DB直読みのためデーモン停止中も表示できる。空なら表示不変。
        self.orphaned_run_ids = src.orphaned_runs;
        // B02: 完了実行の費用（Card 表示用。例 `$0.42` / `費用不明`）。
        self.main_panel.cost_labels = src.cost_labels;
        // Notification 判定は全実行対象（DAG 載否にかかわらず覚え置きする）。
        // sidebar の running/review/done 配下リーフにも色を付けるため。
        self.judge_reflection_statuses(src.reflection);
        // 接続情報（実行単位。waggle_token / task_delegation / task_output を含む）
        self.connection_displays = src.connections;

        // 左SB ツリーは `prep` の実行一覧と札一覧をそのまま使う（再クエリなし）。
        // 順序に注意：ツリーは `self.kanban_entries` を覚え置き前に借りられないため、
        // `prep.kanban_entries` を渡す。
        let kanban_entries = prep.kanban_entries;
        self.build_left_tree_from_executions_slice(&prep.executions, &kanban_entries);

        // Kanban ボード表示用の一覧を覚え置きする（描画時の再計算なし）。
        self.kanban_entries = kanban_entries;
    }

    /// 左サイドバーのツリーを取得済みの実行履歴から構築する。
    ///
    /// sidebar は両モードで共通（`todo`＋R1/R2...）。モードで出し分けない。
    /// `Card`（長方形ブロック）はメイン画面（Workflow DAG／Kanban ボード）のみの概念で、
    /// sidebar にあるのは文字列（todo の md 名・`ID<run_id>/<block名>`）だけである。
    /// Card の排他（どちらか一方のメイン画面にしか存在できない）は
    /// `block_displays`（Workflow DAG）と `kanban_entries`（Kanban ボード）で行う。
    /// ```text
    /// 󰉋 R1
    /// ├─󰉋 r1
    /// │  ├─ID10/block_name1
    /// │  └─ID11/block_name2
    /// └─󰉋 r2
    ///    └─ID20/block_name
    /// ```
    /// R（グローバルラウンド）・r（ローカルラウンド）はフォルダとして
    /// 折りたたみ/展開でき、リーフ（実行）は Enter で右SB と連動する。
    fn build_left_tree_from_executions_slice(
        &mut self,
        executions: &[sai_link_engine::db::models::ExecutionHistory],
        lane_entries: &[crate::app::kanban::CardEntry],
    ) {
        // 組み立て本体は sai_link_widgets::left_tree へ移設（sai-control-tui /
        // `sai-link phone tree` と同一ロジック）。TUI は TreeItem（描画用）へ変換する。
        if executions.is_empty() {
            self.left_sidebar.tree_items = Vec::new();
            self.left_sidebar.all_items = Vec::new();
            return;
        }

        // リロード時に見ていた位置（選択中リーフ）を元のまま保つため、
        // 再構築前の選択リーフの実行 ID・カードパスを覚えておく。
        // カードは実行 ID が無いためパスで復元する。
        let prev_selected = self
            .left_sidebar
            .tree_items
            .get(self.left_sidebar.selected_index)
            .filter(|t| !t.is_folder())
            .map(|t| (t.id, t.card_path.clone()));

        // 前回のツリーの展開状態を保持（フォルダID → expanded）
        let mut prev_expanded: HashMap<i64, bool> = HashMap::new();
        for t in &self.left_sidebar.all_items {
            if t.is_folder() {
                prev_expanded.insert(t.id, t.expanded);
            }
        }

        let built = sai_link_widgets::left_tree::build_left_tree(
            &self.store,
            executions,
            lane_entries,
            &self.mb_replica_execs,
            &prev_expanded,
        );
        let items: Vec<TreeItem> = built
            .into_iter()
            .map(|it| TreeItem {
                id: it.id,
                name: it.name,
                depth: it.depth,
                expanded: it.expanded_default,
                is_last_child: it.is_last_child,
                kind: match it.kind {
                    sai_link_widgets::left_tree::LeftTreeKind::Folder => TreeItemKind::Folder,
                    sai_link_widgets::left_tree::LeftTreeKind::Leaf => TreeItemKind::Leaf,
                    sai_link_widgets::left_tree::LeftTreeKind::Card => TreeItemKind::Card,
                },
                ancestor_last: it.ancestor_last,
                card_path: it.card_path,
            })
            .collect();

        self.left_sidebar.all_items = items;
        self.rebuild_visible_left_tree();
        // リロード時も見ていた位置から続きを確認できるよう、
        // 再構築前の選択リーフ（実行 ID・カードパス）と同じ項目へ選択を復元する。
        // 見つからない場合（履歴削除など）のみ先頭へ戻す。
        self.left_sidebar.selected_index = prev_selected
            .and_then(|(id, card_path)| {
                self.left_sidebar
                    .tree_items
                    .iter()
                    .position(|t| !t.is_folder() && t.id == id && t.card_path == card_path)
            })
            .unwrap_or(0);
    }


    /// 全ツリー（all_items）から可視アイテム（tree_items）を再構築する。
    ///
    /// 折りたたみ中のフォルダの子孫は非表示になり、
    /// 選択・スクロール位置は可視範囲にクランプされる。
    fn rebuild_visible_left_tree(&mut self) {
        let full = self.left_sidebar.all_items.clone();
        let mut visible: Vec<TreeItem> = Vec::with_capacity(full.len());
        // 折りたたみ中フォルダの深さ（その子孫をスキップする）
        let mut collapsed_depth: Option<usize> = None;

        for item in &full {
            if let Some(cd) = collapsed_depth {
                if item.depth <= cd {
                    // スキップ範囲を抜けた
                    collapsed_depth = None;
                } else {
                    continue; // 子孫はスキップ
                }
            }
            visible.push(item.clone());
            if item.is_folder() && !item.expanded {
                collapsed_depth = Some(item.depth);
            }
        }

        self.left_sidebar.tree_items = visible;
        // 選択・スクロールのクランプ
        let last = self.left_sidebar.tree_items.len().saturating_sub(1);
        if self.left_sidebar.selected_index > last {
            self.left_sidebar.selected_index = last;
        }
        if self.left_sidebar.scroll_offset > last {
            self.left_sidebar.scroll_offset = last;
        }
    }

    /// 左SB のフォルダを折りたたみ/展開をトグルし、可視ツリーを再構築する。
    ///
    /// `index` は可視ツリー（tree_items）上のインデックス。
    /// トグル後、選択カーソルは対象フォルダに戻る。
    pub fn toggle_left_folder(&mut self, index: usize) {
        let Some(item) = self.left_sidebar.tree_items.get(index) else {
            return;
        };
        if !item.is_folder() {
            return;
        }
        let folder_id = item.id;

        // 全ツリー側の expanded を反転
        if let Some(full) = self
            .left_sidebar
            .all_items
            .iter_mut()
            .find(|t| t.id == folder_id && t.is_folder())
        {
            full.expanded = !full.expanded;
        }

        self.rebuild_visible_left_tree();

        // トグルしたフォルダへ選択を戻す
        if let Some(idx) = self
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.id == folder_id)
        {
            self.left_sidebar.selected_index = idx;
        }
    }

    /// 実行詳細データ（Log 画面表示用）を構築する。
    ///
    /// 本体は `sai_link_widgets::block_log::build` に移設済み（sai-control-tui と
    /// 共通のビルダ）。本ラッパは UI 側の副作用（選択実行 ID の覚え置き）のみ行う。
    pub fn build_block_detail(&mut self, exec_id: i64) -> Option<BlockDetail> {
        self.main_panel.selected_block_id = Some(exec_id);
        sai_link_widgets::block_log::build(&self.store, exec_id)
    }
    /// History モード: 選択した実行の History 画面データをメイン画面に表示する。
    ///
    /// 「Token Usage by Category」（type ごとの合計値ボックス + 円グラフ）と
    /// 「Timeline（Time / Type / name / token）」のみを表示する
    /// （セッション出力本文は表示しない）。
    ///
    /// **History 画面は近似値のみ使用する**: token はセッション jsonl の
    /// 文字数換算（近似）であり、provider の実際の usage（`execution_usage`）は
    /// 使用しない（Project 画面のみ正確値）。
    ///
    /// `h` 押下時に呼ばれる。DAG データ（block_displays / connection_displays）は
    /// クリアしない（Workflow 戻り時に再読み込みせず済むようにするため）。
    pub(crate) fn load_session_output_to_main(&mut self, exec_id: i64) {
        self.main_panel.md_view = None;
        self.main_panel.history = None;

        // 本体は sai_link_widgets::block_history::build に移設済み（sai-control-tui と共通）
        self.main_panel.history = sai_link_widgets::block_history::build(&self.store, exec_id);
    }

    /// Stats ビュー: 実行のセッション jsonl を解析してキャッシュ統計を読み込む。
    ///
    /// 左SB / メイン画面で選択した実行の `session_dir` から
    /// `sai_link_engine::session::find_main_session_file` でメインセッション
    /// jsonl を解決し、`sai-cache-tui-rs` と同じ解析（usage 集計）を行う。
    ///
    /// `exec_id` が None の場合は、メイン画面の選択 → 左SB 選択の順で解決する。
    pub fn load_cache_view(&mut self, exec_id: Option<i64>) {
        // 表示対象の実行 ID を解決（引数のみ。呼び出し側が sidebar 選択から決める）。
        // 以前はメイン画面選択・左SB選択へフォールバックしていたが、表示と実ファイルの
        // 乖離（stale な保持値で別ブロックの session を開く）を防ぐため廃止した。
        let Some(exec_id) = exec_id else {
            self.cache_view.error = Some("Stats: 実行が選択されていません".to_string());
            return;
        };

        // 同一実行の再読み込みは省略（キャッシュ）
        if self.cache_view.loaded_exec_id == Some(exec_id) && self.cache_view.metrics.is_some() {
            return;
        }

        // 本体は sai_link_widgets::cache_stats::load に移設済み（sai-control-tui と共通）。
        // チャートモードのみ現行値を引き継ぐ（sai-link 従来動作）。
        let prev_mode = self.cache_view.chart_mode;
        let mut view = sai_link_widgets::cache_stats::load(&self.store, exec_id);
        view.chart_mode = prev_mode;
        self.cache_view = view;
    }

    /// message ビュー: 選択実行の session jsonl から user / assistant message のみを
    /// 抽出し、`.sai/message/<run_id>.md` に保存して md 画面に表示する。
    ///
    /// `m` 押下時に呼ばれる（押下時点の選択実行で固定）。保存は毎回上書きし、
    /// 最新の session 内容を反映する。session が無い場合は frontmatter のみの
    /// md を表示する。
    /// md-tui: 指定 md ファイルを読み込みレンダリングしてメイン画面に表示する。
    ///
    /// `Space r/t/m` から呼ばれる共通本体（押下時点の選択実行で固定）。
    /// message のみ session から再生成（毎回上書き）し、review/task は既存ファイルを
    /// そのまま読む（無ければエラーメッセージ）。
    pub(crate) fn load_md_view(&mut self, exec_id: i64, kind: crate::ui::MdKind) {
        self.main_panel.md_view = None;
        self.main_panel.history = None;
        self.main_panel.project_dash = None;
        // パス解決は sai_link_widgets::phone_view へ移設（`sai-link phone md` と同一）
        let path = match sai_link_widgets::phone_view::resolve_md_path(&self.store, exec_id, kind) {
            sai_link_widgets::phone_view::MdResolve::Found(p) => p,
            sai_link_widgets::phone_view::MdResolve::Missing(msg)
            | sai_link_widgets::phone_view::MdResolve::Failed(msg) => {
                self.set_message(msg);
                return;
            }
        };
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let block_name = self
            .store
            .get_execution_by_id(exec_id)
            .ok()
            .and_then(|e| self.store.get_block_by_id(e.block_id).ok())
            .map(|b| b.display_name)
            .unwrap_or_default();
        self.main_panel.md_view = Some(MarkdownView::from_markdown(
            &content,
            &self.theme,
            format!("{} run_id {}", kind.label(), exec_id),
            format!("{} | {}", block_name, path.display()),
        ));
        self.view_mode = ViewMode::Md;
    }


    /// md ファイルを直接開いてメイン画面に表示する（todo カード用・A5）。
    ///
    /// `Space t` で todo カード（DB `status='todo'` 行のみ・`selected_exec_id` なし）を
    /// 選択中に呼ぶ。`load_md_view` と異なり DB 引きを使わずパス直読みする。
    /// run_id は frontmatter `run_id:` から取り、パース失敗時はディレクトリ名を使う。
    pub(crate) fn load_md_file_view(&mut self, path: &std::path::Path, kind: crate::ui::MdKind) {
        use crate::ui::MdKind;
        self.main_panel.md_view = None;
        self.main_panel.history = None;
        self.main_panel.project_dash = None;
        if !path.exists() {
            self.set_message(format!("task: task.md がありません ({})", path.display()));
            return;
        }
        let content = std::fs::read_to_string(path).unwrap_or_default();
        // run_id・block名は task.md の frontmatter から取る（DB行なしでも表示できる）
        let (run_id, block_name) = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| sai_link_engine::kanban::parse_card(&t))
            .map(|c| {
                let rid = c.frontmatter.run_id.trim().parse::<i64>().ok().or_else(|| {
                    path.parent()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .and_then(|n| n.parse::<i64>().ok())
                });
                let b = c.frontmatter.block.trim().to_string();
                (rid, if b.is_empty() { "-".to_string() } else { b })
            })
            .unwrap_or((None, "-".to_string()));
        let title = match run_id {
            Some(rid) => format!("{} run_id {}", kind.label(), rid),
            None => format!("{} run_id ?", kind.label()),
        };
        self.main_panel.md_view = Some(MarkdownView::from_markdown(
            &content,
            &self.theme,
            title,
            format!("{} | {}", block_name, path.display()),
        ));
        // `MdKind` は `Task` 想定だが他種別でも開ける（将来の拡張用）。
        let _ = MdKind::Task;
        self.view_mode = ViewMode::Md;
    }

    /// 選択実行の md waggle トークン一覧を返す（`w` キー用）。
    ///
    /// `output_waggle_tokens` を解決し、`.md` のトークンのみを対象にする。
    /// 解決できないトークン・非 md トークンはスキップする。
    pub(crate) fn waggle_choices_for(
        &self,
        exec_id: i64,
    ) -> Vec<crate::ui::modals::WaggleChoice> {
        // 組み立ては sai_link_widgets::phone_view へ移設（`sai-link phone waggle` と同一）
        sai_link_widgets::phone_view::waggle_choices(&self.store, exec_id)
            .into_iter()
            .map(|c| crate::ui::modals::WaggleChoice {
                token: c.token,
                file_name: c.file_name,
                description: c.description,
                block_name: c.block_name,
                status: c.status,
            })
            .collect()
    }


    /// waggle トークンの内容を md-tui で表示する（`w` キー用）。
    ///
    /// 元ファイルが存在しない場合は Blob ストアから復元して表示する。
    pub(crate) fn load_waggle_token_view(&mut self, exec_id: i64, token: &str) {
        self.main_panel.md_view = None;
        self.main_panel.history = None;
        self.main_panel.project_dash = None;
        // 解決本体は sai_link_widgets::phone_view へ移設（`sai-link phone waggle` と同一）
        let doc = match sai_link_widgets::phone_view::load_waggle_token(token) {
            Ok(d) => d,
            Err(msg) => {
                self.set_message(msg);
                return;
            }
        };
        // 表示に成功したら sai-paper の活動記録に「開いた」を残す（sai-paper が無ければ何もしない）。
        let open_path = doc.path.clone();
        let subtitle_desc = if doc.description.is_empty() {
            doc.file_name
        } else {
            format!("{} - {}", doc.file_name, doc.description)
        };
        self.main_panel.md_view = Some(MarkdownView::from_markdown(
            &doc.content,
            &self.theme,
            format!("waggle {}", doc.token),
            format!("{} | {} | run_id {}", subtitle_desc, doc.block_name, exec_id),
        ));
        self.view_mode = ViewMode::Md;
        if let Some(path) = open_path {
            sai_link_engine::sai_paper_client::notify_open(&path);
        }
    }




    /// message ビュー（後方互換用の薄ラッパー）。
    pub(crate) fn load_message_view(&mut self, exec_id: i64) {
        self.main_panel.md_view = None;
        self.main_panel.history = None;
        self.main_panel.project_dash = None;
        match sai_link_engine::orchestrator::write_message_file(&self.store, exec_id) {
            Ok(path) => {
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                let block_name = self
                    .store
                    .get_execution_by_id(exec_id)
                    .ok()
                    .and_then(|e| self.store.get_block_by_id(e.block_id).ok())
                    .map(|b| b.display_name)
                    .unwrap_or_default();
                self.main_panel.md_view = Some(MarkdownView::from_markdown(
                    &content,
                    &self.theme,
                    format!("message run_id {}", exec_id),
                    format!("{} | {}", block_name, path.display()),
                ));
            }
            Err(e) => {
                self.set_message(format!("message: 保存に失敗: {}", e));
            }
        }
    }

    /// Project ビュー: プロジェクト概要（R14〜R18）と統計・グラフ（R19〜R21）を表示する。
    ///
    /// データ構築は sai_link_widgets::project_dashboard::build_dashboard に移設済み
    /// （sai-control-tui と共通。`ProjectDashData` の形が変わっても修正は 1 箇所）。
    pub(crate) fn load_project_view(&mut self) {
        let root = sai_link_engine::project::project_root();
        let data = sai_link_widgets::project_dashboard::build_dashboard(&self.store, &root, "");
        self.main_panel.project_dash = Some(data);
    }

    /// 実行 ID から表示用の（ブロック名, 開始日時）を返す。
    #[cfg(test)]
    fn exec_label(&self, exec_id: i64) -> (String, String) {
        match self.store.get_execution_by_id(exec_id) {
            Ok(exec) => {
                let name = self
                    .store
                    .get_block_by_id(exec.block_id)
                    .map(|b| b.display_name)
                    .unwrap_or_else(|_| format!("block#{}", exec.block_id));
                let started = exec.started_at.clone().unwrap_or_default();
                let started = if started.is_empty() {
                    "unknown".to_string()
                } else {
                    started
                };
                (name, started)
            }
            Err(_) => (format!("exec#{}", exec_id), "unknown".to_string()),
        }
    }

    // -----------------------------------------------------------------------
    // データ取得ヘルパー
    // -----------------------------------------------------------------------
    // 実行詳細のビルダ（waggle token / session 統計の取得）は
    // sai_link_widgets::block_log へ移設済み（sai-control-tui と共通）。

    // -----------------------------------------------------------------------
    // ビューモード切替
    // -----------------------------------------------------------------------

    /// 現在の view_mode に応じてメイン画面のデータを再構築する。
    ///
    /// Workflow 戻り時のトグル後半（`return_to_workflow`）は DB 再読み込みなしで
    /// 一時ビューを破棄するのみ。この関数は主に起動時・リロード後に使う。
    #[cfg(test)]
    pub fn rebuild_for_view_mode(&mut self) {
        match self.view_mode {
            ViewMode::Workflow => {
                self.load_data();
                self.main_panel.md_view = None;
                self.main_panel.project_dash = None;
            }
            ViewMode::History => {
                // `h` 押下時に固定済みの md_view を表示するのみ
                self.main_panel.project_dash = None;
            }
            ViewMode::Log => {
                // `l` 押下時に固定済みの log_state を表示するのみ
                self.main_panel.project_dash = None;
            }
            ViewMode::Project => {
                self.main_panel.md_view = None;
                // プロジェクト全体の概要を表示する（実行選択に依存しない）
                self.load_project_view();
            }
            ViewMode::Stats => {
                // Stats ビュー: `s` 押下時に固定済みの cache_view を表示するのみ
                self.main_panel.md_view = None;
                self.main_panel.project_dash = None;
            }
            ViewMode::Search => {
                // 検索画面: 前回のフォーム値・結果を保持したまま再表示する
            }
            ViewMode::Message => {
                // 旧単体 `m`（後方互換・固定済みの md_view を表示するのみ）
                self.main_panel.project_dash = None;
            }
            ViewMode::Md => {
                // md-tui（`Space r/t/m` で固定済みの md_view を表示するのみ）
                self.main_panel.project_dash = None;
            }
            ViewMode::Model => {
                // model-tui（`Shift+M` 押下時に読み込んだ一覧を表示するのみ）
            }
        }
    }

    /// マークダウン内容を直接メイン画面に表示する（レンダリング専用・編集なし）。
    ///
    /// waggle トークンの元ファイルが消えている場合（worktree 削除など）に
    /// Blob ストアから復元した内容を表示するために使用する。
    ///
    /// Project ビューのダッシュボード化（ratatui ウィジェット描画）により
    /// 現在は未使用。マークダウン表示が必要な機能追加のために保持する。
    #[cfg(test)]
    pub(crate) fn show_block_content(&mut self, title: String, subtitle: String, content: String) {
        self.main_panel.md_view = Some(MarkdownView::from_markdown(
            &content, &self.theme, title, subtitle,
        ));
    }

    // -----------------------------------------------------------------------
    // コマンド系（load/reload）
    // -----------------------------------------------------------------------

    /// データを再読み込みする。
    pub fn reload_data(&mut self) {
        self.load_data();
        // Project 画面表示中は Summary メモ（`.sai/Summary.md`）も読み込み直す。
        // （`Shift+A` で nvim 編集 → `Shift+R` で表示へ反映、という流れを想定）
        if self.view_mode == ViewMode::Project {
            // スクロール位置（ページ全体 / Recent 内部）は保持する
            let (scroll, offset) = self
                .main_panel
                .project_dash
                .as_ref()
                .map(|d| (d.scroll, d.recent_offset))
                .unwrap_or((0, 0));
            self.load_project_view();
            if let Some(d) = self.main_panel.project_dash.as_mut() {
                d.scroll = scroll;
                d.recent_offset = offset;
            }
        }
    }
}

// ============================================================================
// ヘルパー
// ============================================================================

// 実行が Kanban の所有か（Workflow との排他判定）は
// `sai_link_widgets::workflow::load` へ移した（`is_kanban_owned`）。

















/// JSON 値を表示用文字列に変換する（文字列は引用符なし）。
/// sai_link_widgets::block_log へ移設済み（同一ロジックの重複解消）。

// ---------------------------------------------------------------------------
// Project 設定（R15）・Run Next 抽出（R18）
// ---------------------------------------------------------------------------

/// プロジェクトの名前・概要設定（`.sai/project.json`）。
///
/// R15 の実体 `.sai/` 配下ファイル。読み取り失敗時は空の既定値を返す。
#[derive(Debug, Clone, Default)]
struct ProjectConfig {
    /// プロジェクト名
    name: String,
    /// プロジェクト概要（現状は保存時のみ使用・表示は name のみ）
    #[allow(dead_code)]
    description: String,
}

impl ProjectConfig {
    /// `.sai/project.json` から読み込む（R15）。
    ///
    /// ファイルが無い・JSON が不正でも既定値（空）を返し、表示を続行する。
    fn load() -> Self {
        let path = sai_link_engine::project::sai_path("project.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Self::default();
        };
        Self {
            name: v
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            description: v
                .get("description")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        }
    }

    /// `.sai/project.json` へ保存する（R15）。日本語を含む内容をそのまま UTF-8 で書く。
    #[cfg(test)]
    fn save(&self) -> std::io::Result<()> {
        let path = sai_link_engine::project::sai_path("project.json");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::json!({ "name": self.name, "description": self.description });
        std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
        Ok(())
    }
}

/// Markdown テキストから `## Run Next` セクションの本文を抽出する（R18 / KTD6）。
///
/// `## Run Next` 見出しに続く、次の見出し（`##`/`#`）までの改行置換済みテキストを
/// 返す。セクションが無い場合は `None`。
// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    /// SAI_PROJECT_ROOT はプロセスグローバルのため、ファイル系テストを直列化する。
    /// engine 側の同名ロックとは別 Mutex のため完全な直列化にならない点に注意。
    /// 本バイナリ内のファイル系テストは共有ロック（`crate::app::file_test_lock()`）で束ねる。
    /// バイナリ内ファイル系テストは `super::file_test_lock()` を使う。
    use sai_link_engine::db::models::BlockMetadata;

    /// テスト用：sidebar の全フォルダを展開する（初期は全閉鎖のため実行リーフ非表示）。
    ///
    /// 初期状態（全閉鎖）では `tree_items` に実行リーフが載らないため、
    /// 実行リーフ選択が前提のテストでは先にこれを呼ぶ。
    fn expand_all_folders(app: &mut AppState) {
        loop {
            let idx = app
                .left_sidebar
                .tree_items
                .iter()
                .position(|t| t.is_folder() && !t.expanded);
            let Some(i) = idx else { break };
            app.toggle_left_folder(i);
        }
    }

    // -------------------------------------------------------------------
    // R15: ProjectConfig の保存・読み込み
    // -------------------------------------------------------------------

    // `SAI_PROJECT_ROOT` はプロセスグローバルな環境変数のため、並列実行されると
    // 競合する。共有ロック（`crate::app::file_test_lock()`）で直列化する（下記テストで取得）。

    #[test]
    fn project_config_default_when_missing_file() {
        let _guard = crate::app::file_test_lock();
        // SAI_PROJECT_ROOT を一時ディレクトリへ向け、project.json が無い状態で既定値になる
        let tmp = std::env::temp_dir().join(format!("sai_proj_cfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        let cfg = ProjectConfig::load();
        assert!(cfg.name.is_empty());
        assert!(cfg.description.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn project_config_roundtrip_with_japanese() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_proj_cfg_rt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        let cfg = ProjectConfig {
            name: "プロジェクト".to_string(),
            description: "日本語の概要です。".to_string(),
        };
        cfg.save().unwrap();
        let loaded = ProjectConfig::load();
        assert_eq!(loaded.name, "プロジェクト");
        assert_eq!(loaded.description, "日本語の概要です。");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Project 画面でのリロード（`Shift+R`）は Summary メモ（`.sai/Summary.md`）を
    /// 読み込み直し、スクロール位置（ページ全体 / Recent 内部）は保持すること。
    #[test]
    fn project_reload_refreshes_summary_and_keeps_scroll() {
        let _guard = crate::app::file_test_lock();
        // SAI_PROJECT_ROOT を一時ディレクトリへ向け、Summary.md を用意する
        let tmp = std::env::temp_dir().join(format!("sai_proj_summary_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let mut app = AppState::new(Arc::new(store));
        app.view_mode = ViewMode::Project;
        app.rebuild_for_view_mode();

        // ファイルが無いときは空（セクション非表示）
        assert!(
            app.main_panel
                .project_dash
                .as_ref()
                .unwrap()
                .summary_md
                .is_empty(),
            "Summary.md が無いときは空"
        );

        // ファイルを作成してリロードすると内容が反映される
        std::fs::write(tmp.join(".sai/Summary.md"), "# Summary\n\n- やることA\n").unwrap();
        // スクロール位置を進めておく（リロード後も保持されることの確認用）
        app.main_panel.project_dash.as_mut().unwrap().scroll = 3;
        app.main_panel.project_dash.as_mut().unwrap().recent_offset = 1;
        app.reload_data();
        let dash = app.main_panel.project_dash.as_ref().unwrap();
        assert!(
            dash.summary_md.contains("やることA"),
            "リロードで Summary の内容が反映される: {:?}",
            dash.summary_md
        );
        assert_eq!(dash.scroll, 3, "ページ全体のスクロール位置は保持される");
        assert_eq!(
            dash.recent_offset, 1,
            "Recent の内部スクロール位置は保持される"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// テスト用のブロック定義を作成する。
    fn create_block(name: &str) -> BlockMetadata {
        BlockMetadata::new(
            name.to_string(),
            "テスト用ブロック".to_string(),
            "anthropic/claude-sonnet-4-5".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            format!("blocks/{}.md", name),
            None
        )
    }

    /// 実行履歴から GR → LR → 実行 の階層ツリーが構築されること。
    ///
    /// ```text
    /// R1
    /// ├─r1
    /// │  ├─ID{e1}/plan-ice
    /// │  └─ID{e2}/plan-ice
    /// └─r2
    ///    └─ID{e3}/plan-ice
    /// R2
    /// └─r1
    ///    └─ID{e4}/plan-ice
    /// ```
    #[test]
    fn left_tree_is_hierarchical() {
        let _guard = crate::app::file_test_lock();
        // SAI_PROJECT_ROOT を空の一時ディレクトリへ向け、実 `.sai/todo` の影響を断つ。
        // （Kanban 表示以外では todo フォルダ自体を出さないが、実カードの混入防止のため）
        let tmp = std::env::temp_dir().join(format!("sai_tree_hier_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();

        // GR1/LR1 ×2, GR1/LR2 ×1, GR2/LR1 ×1 の実行を作成
        let e1 = store.create_execution(block_id, None).unwrap();
        let e2 = store.create_execution(block_id, None).unwrap();
        let e3 = store.create_execution(block_id, None).unwrap();
        let e4 = store.create_execution(block_id, None).unwrap();
        for &e in &[e1, e2, e3, e4] {
            store.update_execution_status(e, "completed").unwrap();
            // R/r ツリーに載せるため `d` 移動済みにする（重複禁止のため review には載らない）
            store.update_execution_workflow_moved(e, true).unwrap();
        }
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();
        store
            .update_execution_overrides(e2, None, Some(1), Some(1), None, None, None)
            .unwrap();
        store
            .update_execution_overrides(e3, None, Some(2), Some(1), None, None, None)
            .unwrap();
        store
            .update_execution_overrides(e4, None, Some(1), Some(2), None, None, None)
            .unwrap();

        let mut app = AppState::new(Arc::new(store));
        app.load_data();

        // 最上位：初期は全閉鎖のため todo/running/review/done + R1 + R2 = 6件。
        // 移動済みのため review 配下は空（重複禁止）。
        assert_eq!(app.left_sidebar.tree_items.len(), 6);

        // R1 を展開 → r1 が現れる
        app.toggle_left_folder(4);
        let after_r1 = app.left_sidebar.tree_items.len();
        assert!(
            after_r1 > 6,
            "R1 展開で子が現れる: {}",
            after_r1
        );
        // r1 を探して展開 → 実行 2 件が現れる
        let r1_idx = app
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.is_folder() && t.name.contains("r1"))
            .expect("r1 フォルダが存在する");
        app.left_sidebar.selected_index = r1_idx;
        app.toggle_left_folder(r1_idx);
        assert_eq!(
            app.left_sidebar.tree_items.len(),
            after_r1 + 2,
            "r1 展開で実行 2 件が現れる"
        );
        assert_eq!(
            app.left_sidebar.selected_index, r1_idx,
            "トグルしたフォルダに選択が戻る"
        );
        // r1 を折りたたむ → 実行 2 件が隠れる
        app.toggle_left_folder(r1_idx);
        assert_eq!(
            app.left_sidebar.tree_items.len(),
            after_r1,
            "r1 折りたたみで実行 2 件が隠れる"
        );
        // R1 を折りたたむ → 配下が隠れる（6件）
        app.toggle_left_folder(4);
        assert_eq!(
            app.left_sidebar.tree_items.len(),
            6,
            "R1 折りたたみで配下が隠れる"
        );
        // 再展開 → 閉鎖状態を維持（r1 は折りたたみ中のまま）
        app.toggle_left_folder(4);
        assert_eq!(
            app.left_sidebar.tree_items.len(),
            after_r1,
            "R1 展開で r1 は表示（r1 は折りたたみ中）"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 左SB でリーフ（実行）を選択して Enter を押すと、右SB とメイン画面（DAG）に連動する。
    ///
    /// - `main_panel.selected_block_id` が実行 ID に設定される
    /// - メイン画面（DAG）描画時に選択ノードがビューポート内（中央付近）へ移動する
    #[test]
    fn left_sidebar_enter_links_to_main_panel() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_tree_enter_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use ratatui::buffer::Buffer as TuiBuffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::Widget;

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("brst-grill")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        let e2 = store.create_execution(block_id, None).unwrap();
        // completed 化する。
        for &e in &[e1, e2] {
            store.update_execution_status(e, "completed").unwrap();
        }
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();
        store
            .update_execution_overrides(e2, None, Some(1), Some(1), None, None, None)
            .unwrap();

        let mut app = AppState::new(Arc::new(store));
        app.load_data();
        expand_all_folders(&mut app);
        assert_eq!(app.view_mode, ViewMode::Workflow);
        // DAG 表示には `d` 移動済みが必要なため、2 件とも移動させる
        app.store.update_execution_workflow_moved(e1, true).unwrap();
        app.store.update_execution_workflow_moved(e2, true).unwrap();
        app.load_data();
        expand_all_folders(&mut app);

        // 左SB ツリーの R/r 配下に実行リーフが 2 件ある
        // （review/done フォルダ配下にも同一実行が並ぶため depth==2 で絞る）
        let leaves: Vec<i64> = app
            .left_sidebar
            .tree_items
            .iter()
            .filter(|t| t.kind == TreeItemKind::Leaf && t.depth == 2)
            .map(|t| t.id)
            .collect();
        assert_eq!(leaves.len(), 2, "R/r 配下に実行リーフが 2 件ある");

        // 2 番目のリーフを選択して Enter
        let idx = app
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.id == e2)
            .unwrap();
        app.left_sidebar.selected_index = idx;
        crate::app::input::move_sidebar_cursor(&mut app, 0);

        // 選択実行 ID とメイン画面の選択が連動する
        assert_eq!(app.selected_exec_id, Some(e2), "選択実行 ID が更新される");
        assert_eq!(
            app.main_panel.selected_block_id,
            Some(e2),
            "メイン画面の選択 ID が更新される"
        );

        // メイン画面（DAG）を描画し、選択ノードがビューポート内に収まること
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = TuiBuffer::empty(area);
        MainPanel {
            state: &mut app.main_panel,
            blocks: &app.block_displays,
            connections: &app.connection_displays,
            theme: &app.theme,
            finalized_runs: &app.finalized_run_parents,
            orphaned_runs: &app.orphaned_run_ids,
        }
        .render(area, &mut buf);

        let pos = app
            .main_panel
            .node_positions
            .get(&e2)
            .expect("選択ノードの位置が記録される");
        let vx = app.main_panel.viewport.scroll_x;
        let vy = app.main_panel.viewport.scroll_y;
        let left = pos.x - vx;
        let right = pos.x + pos.width as i32 - vx;
        let top = pos.y - vy;
        let bottom = pos.y + pos.height as i32 - vy;
        assert!(
            left >= 0 && right <= area.width as i32,
            "選択ノードが水平方向に可視領域内（left={left} right={right} vx={vx}）"
        );
        assert!(
            top >= 0 && bottom <= area.height as i32,
            "選択ノードが垂直方向に可視領域内（top={top} bottom={bottom} vy={vy}）"
        );

        // 選択ノードはハイライト（テキスト色 Yellow）で描画される。
        // DAG コンテンツはパネル境界線の 1 セル内側から描画されるため、
        // ノードの左上(┌)は (left+1, top+1)、内部テキストは 1 段内側となる。
        let sx = left as u16;
        let sy = top as u16;
        let cell = &buf[(sx + 2, sy + 2)];
        assert_eq!(
            cell.fg,
            ratatui::style::Color::Yellow,
            "選択ノードのテキストは Yellow でハイライトされる（sx={sx} sy={sy}）"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// マウスでパン/ズームした後（manual_scroll = true）でも、左SB で Enter すると
    /// 自動追従に戻り、選択ノードへ画面移動すること。
    #[test]
    fn left_sidebar_enter_resets_manual_scroll() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_tree_manual_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use ratatui::buffer::Buffer as TuiBuffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::Widget;

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("brst-grill")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        // completed 化する。
        store.update_execution_status(e1, "completed").unwrap();
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();

        let mut app = AppState::new(Arc::new(store));
        app.load_data();
        expand_all_folders(&mut app);
        // DAG 表示には `d` 移動済みが必要なため移動させる
        app.store.update_execution_workflow_moved(e1, true).unwrap();
        app.load_data();
        expand_all_folders(&mut app);

        // マウス操作で手動スクロール状態にする
        app.main_panel.viewport.manual_scroll = true;
        app.main_panel.viewport.scroll_x = 9999;
        app.main_panel.viewport.scroll_y = 9999;

        // 左SB でリーフを選択して Enter
        let idx = app
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.id == e1)
            .unwrap();
        app.left_sidebar.selected_index = idx;
        crate::app::input::move_sidebar_cursor(&mut app, 0);

        // 自動追従に戻る
        assert!(
            !app.main_panel.viewport.manual_scroll,
            "左SB で選択したら manual_scroll が解除される"
        );

        // 描画すると選択ノードへ移動する
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = TuiBuffer::empty(area);
        MainPanel {
            state: &mut app.main_panel,
            blocks: &app.block_displays,
            connections: &app.connection_displays,
            theme: &app.theme,
            finalized_runs: &app.finalized_run_parents,
            orphaned_runs: &app.orphaned_run_ids,
        }
        .render(area, &mut buf);

        let pos = app
            .main_panel
            .node_positions
            .get(&e1)
            .expect("選択ノードの位置が記録される");
        let left = pos.x - app.main_panel.viewport.scroll_x;
        assert!(
            left >= 0 && left < area.width as i32,
            "手動スクロール後でも選択ノードへ移動する（left={left}）"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// R22: MarkdownView は画面高を超える内容を縦スクロールで末尾まで閲覧できる。
    ///
    /// History / Reflection / Project の各ビューはこの `MarkdownView.scroll` を
    /// 共通のスクロール枠組みとして利用するため、そのスクロールが clamp されつつ
    /// 先頭〜末尾まで移動できることを検証する。
    #[test]
    fn r22_markdown_view_scrolls_to_end() {
        use ratatui::buffer::Buffer as TuiBuffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::Widget;
        use crate::ui::markdown::render_markdown;

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let mut app = AppState::new(Arc::new(store));

        // 画面高を超える行数（200 行）のマークダウンを構築する
        let mut text = String::new();
        for i in 0..200 {
            text.push_str(&format!("line {:03} 内容\n", i));
        }
        let (lines, _headings) = render_markdown(&text, &app.theme);
        assert!(
            lines.len() > 50,
            "テストは画面高を超える行数を使う（{}行）",
            lines.len()
        );

        app.main_panel.md_view = Some(MarkdownView::from_markdown(
            &text,
            &app.theme,
            "scroll".to_string(),
            String::new(),
        ));

        // 下方向: 末尾までスクロールでき、行数を超えない（clamp）
        let max = app
            .main_panel
            .md_view
            .as_ref()
            .unwrap()
            .lines
            .len()
            .saturating_sub(1);
        let mut guard = 0usize;
        while app.main_panel.md_view.as_ref().unwrap().scroll < max {
            md_scroll(&mut app, 10);
            let s = app.main_panel.md_view.as_ref().unwrap().scroll;
            assert!(
                s <= max,
                "スクロールは行数を超えない（clamp）: s={s} max={max}"
            );
            guard += 1;
            assert!(guard < 10000, "スクロールが収束しない");
        }
        assert_eq!(
            app.main_panel.md_view.as_ref().unwrap().scroll,
            max,
            "末尾までスクロールできる"
        );

        // 上方向: 先頭に戻り、0 未満にはならない
        let mut guard = 0usize;
        while app.main_panel.md_view.as_ref().unwrap().scroll > 0 {
            md_scroll(&mut app, -10);
            guard += 1;
            assert!(guard < 10000, "スクロールが収束しない");
        }
        assert_eq!(
            app.main_panel.md_view.as_ref().unwrap().scroll,
            0,
            "先頭まで戻れる"
        );

        // 描画しても panic しない（小さな領域でも安全にレンダリングできる）
        let area = Rect::new(20, 1, 40, 10);
        let mut buf = TuiBuffer::empty(area);
        MainPanel {
            state: &mut app.main_panel,
            blocks: &app.block_displays,
            connections: &app.connection_displays,
            theme: &app.theme,
            finalized_runs: &app.finalized_run_parents,
            orphaned_runs: &app.orphaned_run_ids,
        }
        .render(area, &mut buf);
    }

    /// Project ビューはダッシュボードデータ（project_dash）として構築され、
    /// キー操作（md_scroll）でスクロールできること。
    #[test]
    fn project_view_builds_scrollable_content() {
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();

        let mut app = AppState::new(Arc::new(store));

        // Project ビュー: 実行履歴を Recent に含むダッシュボードデータを生成する
        app.view_mode = ViewMode::Project;
        app.rebuild_for_view_mode();
        let dash = app
            .main_panel
            .project_dash
            .as_ref()
            .expect("Project ビューは project_dash を生成する");
        assert_eq!(dash.recent.len(), 1, "Recent に実行 1 件が含まれる");
        assert_eq!(dash.summary.total, 1, "サマリに実行数が反映される");

        // 同じスクロール枠組み（md_scroll）で既存キー操作が働く（max_scroll=0 では動かない）
        md_scroll(&mut app, 1);
        assert_eq!(app.main_panel.project_dash.as_ref().unwrap().scroll, 0);
    }

    /// ViewMode 回帰: 再定義後の全ビューモードの再構築が panic しないこと。
    #[test]
    fn existing_view_modes_regression() {
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();
        let mut app = AppState::new(Arc::new(store));

        // 全ビューモード（Workflow / Log / History / Stats / Project）の再構築が panic しない
        for mode in [
            ViewMode::Workflow,
            ViewMode::Log,
            ViewMode::History,
            ViewMode::Stats,
            ViewMode::Project,
        ] {
            app.view_mode = mode;
            app.rebuild_for_view_mode();
            assert_eq!(app.view_mode, mode);
        }
    }

    /// History ビュー（ratatui ネイティブ化）: 種別別近似トークンと Timeline イベントが
    /// 構築され、セッション出力（マークダウン）が下段データとして保持される。
    /// History 画面は近似値のみ使用する（execution_usage は使わない）。
    #[test]
    fn history_view_builds_type_tokens_and_timeline() {
        let dir = tempfile::tempdir().unwrap();
        let sess_dir = dir.path().join("pi-sessions-test");
        std::fs::create_dir_all(&sess_dir).unwrap();
        std::fs::write(
            sess_dir.join("session_01a04b0e-0000-0000-0000-00000000000a.jsonl"),
            concat!(
                r#"{"type":"session","version":3,"id":"x"}"#, "\n",
                r#"{"type":"message","timestamp":"2026-08-29T01:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"こんにちは"}]}}"#, "\n",
                r#"{"type":"message","timestamp":"2026-08-29T01:00:02.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"思考"},{"type":"text","text":"回答"}]}}"#, "\n",
            ),
        )
        .unwrap();

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();
        store
            .update_execution_session_info(e1, &sess_dir.to_string_lossy(), "pi")
            .unwrap();
        store
            .update_execution_output_content(e1, "# セッション出力\n本文です")
            .unwrap();

        let mut app = AppState::new(Arc::new(store));
        app.view_mode = ViewMode::History;
        app.load_session_output_to_main(e1);

        let hist = app
            .main_panel
            .history
            .as_ref()
            .expect("History ビューは history を生成する");
        assert!(app.main_panel.md_view.is_none(), "md_view は使わない");

        // 近似トークン（文字数換算）: user=5 / agent=2 / thinking=2
        assert_eq!(hist.type_tokens.user, 5);
        assert_eq!(hist.type_tokens.agent, 2);
        assert_eq!(hist.type_tokens.thinking, 2);
        assert_eq!(hist.type_tokens.tool_call, 0);

        // Timeline: user → thinking → agent（時刻順）
        let kinds: Vec<&str> = hist.events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec!["user", "thinking", "agent"]);
        assert_eq!(hist.events[0].name, "こんにちは");
        assert_eq!(hist.events[0].tokens, 5);

        // セッション出力本文は表示しない（HistoryViewData に含まれない）
        assert!(hist.timeline_offset == 0);
    }

    /// History ビュー: session_dir 不在の実行では近似値 0・イベント空で表示が壊れない。
    #[test]
    fn history_view_handles_missing_session_dir() {
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("no-session")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();

        let mut app = AppState::new(Arc::new(store));
        app.view_mode = ViewMode::History;
        app.load_session_output_to_main(e1);

        let hist = app.main_panel.history.as_ref().unwrap();
        assert!(hist.events.is_empty(), "イベントは空");
        assert_eq!(hist.type_tokens.user, 0, "近似値は 0");
    }

    // -------------------------------------------------------------------
    // U13・R25: 左SB からメイン複製（mb1.x）を除外
    // -------------------------------------------------------------------

    #[test]
    fn left_sidebar_excludes_mb_replica_execs() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_tree_replica_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        use crate::ui::left_sidebar::TreeItemKind;

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block = store.insert_block(&create_block("replica-test")).unwrap();
        // メインブロック（parent NULL）
        let main = store.create_execution(block, None).unwrap();
        // メイン複製（mb1.x）: run_batch の mb_execution_id に設定
        let replica = store
            .create_execution_for_subtask(block, None, main, None)
            .unwrap();
        store
            .create_run_batch(main, "E1", 1, Some(replica))
            .unwrap();
        // 通常のサブタスク実行（左SB に表示されるべき）
        // completed 化する。R/r ツリーに載せるため `d` 移動済みにする。
        let sub = store
            .create_execution_for_subtask(block, None, main, None)
            .unwrap();
        for &e in &[main, replica, sub] {
            store.update_execution_status(e, "completed").unwrap();
        }
        store.update_execution_workflow_moved(sub, true).unwrap();

        let mut app = AppState::new(Arc::new(store));
        app.rebuild_for_view_mode(); // Workflow → load_data → build_left_tree

        let ids: Vec<i64> = app
            .left_sidebar
            .all_items
            .iter()
            .filter(|t| matches!(t.kind, TreeItemKind::Leaf))
            .map(|t| t.id)
            .collect();
        assert!(
            !ids.contains(&replica),
            "メイン複製は左SB の実行一覧に表示しない (R25)"
        );
        assert!(ids.contains(&sub), "通常サブタスクは左SB に表示される");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -------------------------------------------------------------------
    // R26: プロジェクト概要（name / description）の読み込み・保存
    // （Project ビューは閲覧専用。編集は Summary.md（Shift+A）が代替する）
    // -------------------------------------------------------------------

    #[test]
    fn project_config_save_and_load_roundtrip() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_proj_edit_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // 名前・概要を保存 → project.json に UTF-8 のまま書き込まれる
        let cfg = ProjectConfig {
            name: "日本語プロジェクト".to_string(),
            description: "概要その１\n概要その２".to_string(),
        };
        cfg.save().expect("保存に成功する");

        // 再読み込みで反映される（AE7）
        let loaded = ProjectConfig::load();
        assert_eq!(loaded.name, "日本語プロジェクト");
        assert_eq!(loaded.description, "概要その１\n概要その２");
        assert!(loaded.save().is_ok(), "保存後も再度保存できる");
    }

    // -------------------------------------------------------------------
    // ショートカット（l / h / P）トグル遷移（仕様書 §2.2・§4）
    // -------------------------------------------------------------------

    /// `l` ショートカット: Log ビューへ遷移し、押下時点の選択実行で固定される。
    /// Log 表示中は左SBカーソル移動が無効になり、再度 `l` で Workflow に戻る。
    #[test]
    fn l_shortcut_toggles_log_view() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();

        let mut app = AppState::new(Arc::new(store));
        app.load_data();
        expand_all_folders(&mut app);
        app.selected_exec_id = Some(e1);
        // `l` は sidebar カーソル基準のため、実行リーフを選択状態にする
        if let Some(idx) = app
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.id == e1)
        {
            app.left_sidebar.selected_index = idx;
        }

        // 1 回目の `l`: Log へ遷移し、押下時点の選択実行で固定
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
        );
        assert_eq!(app.view_mode, ViewMode::Log, "`l` で Log へ遷移する");
        assert!(
            !app.is_sidebar_enabled(),
            "Log 表示中は左SBカーソル移動が無効"
        );
        let detail = app.log_state.detail.as_ref().expect("Log 詳細が固定される");
        assert_eq!(detail.exec_id, e1, "押下時点の選択実行で固定される");
        assert_eq!(detail.block_name, "plan-ice");

        // `l` 再押しでは戻らない。Esc で Workflow に戻る
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
        );
        assert_eq!(app.view_mode, ViewMode::Log, "`l` 再押しでは戻らない");
        handle_key_event(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.view_mode, ViewMode::Workflow, "Esc で Workflow に戻る");
        assert!(app.is_sidebar_enabled());
    }

    /// `h` ショートカット: History ビューへ遷移し、再度 `h` で Workflow に戻る。
    #[test]
    fn h_shortcut_toggles_history_view() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();

        let mut app = AppState::new(Arc::new(store));
        app.load_data();
        expand_all_folders(&mut app);
        app.selected_exec_id = Some(e1);
        // `h` は sidebar カーソル基準のため、実行リーフを選択状態にする
        if let Some(idx) = app
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.id == e1)
        {
            app.left_sidebar.selected_index = idx;
        }

        // 1 回目の `h`: History へ遷移（md_view が構築される）
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
        );
        assert_eq!(
            app.view_mode,
            ViewMode::History,
            "`h` で History へ遷移する"
        );
        assert!(!app.is_sidebar_enabled());
        assert!(
            app.main_panel.history.is_some(),
            "History ビューは history を生成する"
        );

        // `h` 再押しでは戻らない。Esc で Workflow に戻る
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
        );
        assert_eq!(app.view_mode, ViewMode::History, "`h` 再押しでは戻らない");
        handle_key_event(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.view_mode, ViewMode::Workflow);
        assert!(
            app.main_panel.history.is_none(),
            "Workflow 戻りで history は破棄される"
        );
        assert!(app.is_sidebar_enabled());
    }

    /// 未選択で `h` を押すと遷移せずメッセージ表示のみであること。
    #[test]
    fn h_shortcut_without_selection_shows_message() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let mut app = AppState::new(Arc::new(store));
        app.load_data();

        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
        );
        assert_eq!(app.view_mode, ViewMode::Workflow, "未選択時は遷移しない");
        assert!(app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("選択されていません"));
    }

    /// `P` ショートカット: Project ビューへ遷移し、再度 `P` で Workflow に戻る。
    /// 小文字 `p` は Session（pane 作成）であり遷移しない。
    #[test]
    fn p_shortcut_toggles_project_view() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let mut app = AppState::new(Arc::new(store));
        app.load_data();

        // 1 回目の `P`（Shift+p）: Project へ遷移
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT),
        );
        assert_eq!(
            app.view_mode,
            ViewMode::Project,
            "`P` で Project へ遷移する"
        );
        assert!(
            app.main_panel.project_dash.is_some(),
            "Project ビューはダッシュボードを生成する"
        );

        // `P` 再押しでは戻らない。Esc で Workflow に戻る
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT),
        );
        assert_eq!(app.view_mode, ViewMode::Project, "`P` 再押しでは戻らない");
        handle_key_event(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.view_mode, ViewMode::Workflow);

        // 小文字 `p` は Session 専用（未選択ならメッセージのみで遷移しない）
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        );
        assert_eq!(app.view_mode, ViewMode::Workflow, "小文字 `p` は遷移しない");
        assert!(
            app.message.as_deref().unwrap_or("").contains("Session"),
            "Session（pane 作成）のメッセージが表示される: {:?}",
            app.message
        );
    }

    // -------------------------------------------------------------------
    // Stats ビュー（s ショートカット + sai-cache-tui-rs 移植）
    // -------------------------------------------------------------------

    /// テスト用のセッション jsonl（assistant usage 2 件付き）を作成する。
    fn write_sample_session(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("2026-08-29T10-00-00-main_test.jsonl");
        let lines = [
            r#"{"type":"message","id":"e1","parentId":null,"message":{"role":"user"}}"#,
            r#"{"type":"message","id":"e2","parentId":"e1","message":{"role":"assistant","provider":"anthropic","model":"claude","usage":{"input":100,"output":50,"cacheRead":300,"cacheWrite":100,"totalTokens":550,"cost":{"input":0.001,"output":0.002,"cacheRead":0.0003,"cacheWrite":0.001,"total":0.0043}}}}"#,
            r#"{"type":"message","id":"e3","parentId":"e2","message":{"role":"assistant","provider":"anthropic","model":"claude","usage":{"input":200,"output":80,"cacheRead":400,"cacheWrite":0,"totalTokens":680,"cost":{"input":0.002,"output":0.003,"cacheRead":0.0004,"cacheWrite":0.0,"total":0.0054}}}}"#,
        ];
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    /// `s` ショートカットで Stats ビューへ遷移し、選択実行のセッション jsonl が
    /// 解析されること（sai-cache-tui-rs と同じ集計）。再度 `s` で元のビューへ戻る。
    #[test]
    fn s_shortcut_enters_stats_view_and_parses_session() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();

        // 実行に session_dir（セッション jsonl の親ディレクトリ）を紐付ける
        let dir = std::env::temp_dir().join(format!("sai_stats_t{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let session_file = write_sample_session(&dir);
        store
            .update_execution_session_info(e1, &dir.to_string_lossy(), "pi")
            .unwrap();

        let mut app = AppState::new(Arc::new(store));
        app.load_data();
        expand_all_folders(&mut app);
        // 左SB で実行を選択した状態を作る（`s` は sidebar カーソル基準）
        app.selected_exec_id = Some(e1);
        if let Some(idx) = app
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.id == e1)
        {
            app.left_sidebar.selected_index = idx;
        }

        // 1 回目の `s`: Stats ビューへ遷移
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        );
        assert_eq!(
            app.view_mode,
            ViewMode::Stats,
            "`s` で Stats ビューへ遷移する"
        );

        assert!(
            app.cache_view.error.is_none(),
            "エラーなし: {:?}",
            app.cache_view.error
        );
        let metrics = app
            .cache_view
            .metrics
            .as_ref()
            .expect("メトリクスが読み込まれる");
        assert_eq!(metrics.all_messages.len(), 2, "assistant usage 2 件");
        assert_eq!(metrics.tree_totals.input, 300);
        assert_eq!(metrics.tree_totals.cache_read, 700);
        assert_eq!(
            app.cache_view.session_file.as_deref(),
            Some(session_file.as_path()),
            "解決したセッション jsonl が記録される"
        );

        // チャートモード切替（`3` → 累積トークン）
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE),
        );
        assert_eq!(
            app.cache_view.chart_mode,
            crate::ui::cache_view::CacheChartMode::CumulativeTotal
        );

        // `s` 再押しでは戻らない。Esc で Workflow へ戻る
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        );
        assert_eq!(app.view_mode, ViewMode::Stats, "`s` 再押しでは戻らない");
        handle_key_event(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.view_mode, ViewMode::Workflow, "Esc で Workflow へ戻る");

        std::fs::remove_dir_all(&dir).ok();
    }

    // -------------------------------------------------------------------
    // R14: Notification（Reflection 記録の記入状況による色分け）
    // -------------------------------------------------------------------

    #[test]
    fn notification_status_judged_and_reflected() {
        // SAI_PROJECT_ROOT はプロセスグローバルのため、既存の env 系テストと
        // 同じロックで直列化し、テスト専用の一時ディレクトリへ向ける。
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_notif_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        let dir = sai_link_engine::project::sai_path(".sai/reflection");
        std::fs::create_dir_all(&dir).unwrap();

        let store = Arc::new(sai_link_engine::db::SaiStore::open_in_memory().unwrap());
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();

        // 完了済み実行を 2 件作成（GR1/LR1）
        let e1 = store.create_execution(block_id, None).unwrap();
        let e2 = store.create_execution(block_id, None).unwrap();
        for &e in &[e1, e2] {
            store
                .update_execution_overrides(e, None, Some(1), Some(1), None, None, None)
                .unwrap();
            store.update_execution_status(e, "completed").unwrap();
        }

        // e1: 「内容の記録のみ」書いたファイルを用意（青）。
        // e2: ファイル無し（赤扱い）。
        let f1 = dir.join(format!("{}.md", e1));
        std::fs::write(&f1, "## Run Reason\n\n理由を書いた\n").unwrap();

        let mut app = AppState::new(Arc::clone(&store));
        app.load_data();

        // 未移動の実行は Kanban 所有のため DAG には載らない
        assert!(app.block_displays.iter().all(|b| b.exec_id != e1));
        assert!(app.block_displays.iter().all(|b| b.exec_id != e2));
        // `d`（移動）後は DAG に載る（色分け廃止のため載りのみ検証）
        app.store.update_execution_workflow_moved(e1, true).unwrap();
        app.store.update_execution_workflow_moved(e2, true).unwrap();
        app.load_data();
        let disp = &app.block_displays;
        assert!(disp.iter().any(|b| b.exec_id == e1));
        assert!(disp.iter().any(|b| b.exec_id == e2));

        // sidebar の重複禁止：同一実行 ID はツリー内に 1 件のみ
        for &e in &[e1, e2] {
            let n = app
                .left_sidebar
                .all_items
                .iter()
                .filter(|t| !t.is_folder() && t.id == e)
                .count();
            assert_eq!(n, 1, "ID{} は sidebar に1件のみ", e);
        }

        // テストで作った reflection ファイルを掃除する
        std::fs::remove_file(&f1).ok();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn running_executions_are_not_colored() {
        // env・ファイル不要（in-memory DB のみで検証する）
        let store = Arc::new(sai_link_engine::db::SaiStore::open_in_memory().unwrap());
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();
        // 実行中のまま（色分け対象外）
        store.update_execution_status(e1, "running").unwrap();

        let mut app = AppState::new(Arc::clone(&store));
        app.load_data();

        // 色分け廃止のため block_displays の reflection_status は常に None
        assert!(app
            .block_displays
            .iter()
            .all(|b| b.reflection_status.is_none()));
    }

    #[test]
    fn reload_keeps_selection_position() {
        let store = Arc::new(sai_link_engine::db::SaiStore::open_in_memory().unwrap());
        let block_id = store.insert_block(&create_block("plan-ice")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        let e2 = store.create_execution(block_id, None).unwrap();
        for &e in &[e1, e2] {
            store
                .update_execution_overrides(e, None, Some(1), Some(1), None, None, None)
                .unwrap();
            store.update_execution_status(e, "completed").unwrap();
        }

        let mut app = AppState::new(Arc::clone(&store));
        app.load_data();
        expand_all_folders(&mut app);

        // 2 番目のリーフ（ID{e2}）を選択してからリロードする
        let second_leaf = app
            .left_sidebar
            .tree_items
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.is_folder())
            .nth(1)
            .map(|(i, _)| i)
            .unwrap();
        app.left_sidebar.selected_index = second_leaf;
        let selected_id = app.left_sidebar.tree_items[second_leaf].id;

        // リロード（Shift+R と同じ経路）
        app.reload_data();

        // 見ていた位置（選択中の実行）が元のまま保たれること
        assert_eq!(
            app.left_sidebar
                .tree_items
                .get(app.left_sidebar.selected_index)
                .map(|t| t.id),
            Some(selected_id),
            "リロード後も選択位置が保たれる"
        );
    }
}

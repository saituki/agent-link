//! `card` グループ（新しい木）— カード（実行予約）の参照・作成・実行。
//!
//! 処理の本体は `views` の実装をそのまま呼ぶ（写さない・出力を変えない）。

use crate::cli::{CardCmd, ViewCardCmd, ViewCmd};
use crate::cmd_common::{print_view, CmdError};

/// `agent-link card <sub>` のハンドラ。
pub(crate) fn run(cmd: CardCmd) -> Result<(), CmdError> {
    match cmd {
        CardCmd::Options { json: _ } => print_view(ViewCmd::Card {
            cmd: ViewCardCmd::Options,
        }),
        CardCmd::Docs { limit, json: _ } => print_view(ViewCmd::Card {
            cmd: ViewCardCmd::Docs { limit },
        }),
        CardCmd::Create { json: _ } => print_view(ViewCmd::Card {
            cmd: ViewCardCmd::Create,
        }),
        CardCmd::Schedule {
            run_id,
            at,
            json: _,
        } => print_view(ViewCmd::Card {
            cmd: ViewCardCmd::Schedule { run_id, at },
        }),
        CardCmd::Run { run_id, json: _ } => print_view(ViewCmd::Card {
            cmd: ViewCardCmd::Run { run_id },
        }),
    }
}

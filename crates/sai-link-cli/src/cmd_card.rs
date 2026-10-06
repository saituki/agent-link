//! `card` グループ（新しい木）— カード（実行予約）の参照・作成・実行。
//!
//! 処理の本体は `phone` の実装をそのまま呼ぶ（写さない・出力を変えない）。

use crate::cli::{CardCmd, PhoneCardCmd, PhoneCmd, PhoneUserCmd};
use crate::cmd_common::{print_phone, CmdError};

/// `sai-moon-link card <sub>` のハンドラ。
pub(crate) fn run(cmd: CardCmd) -> Result<(), CmdError> {
    match cmd {
        CardCmd::Options { json: _ } => print_phone(PhoneCmd::Card {
            cmd: PhoneCardCmd::Options,
        }),
        CardCmd::Docs { limit, json: _ } => print_phone(PhoneCmd::Card {
            cmd: PhoneCardCmd::Docs { limit },
        }),
        CardCmd::Create { json: _ } => print_phone(PhoneCmd::Card {
            cmd: PhoneCardCmd::Create,
        }),
        CardCmd::Schedule {
            run_id,
            at,
            json: _,
        } => print_phone(PhoneCmd::Card {
            cmd: PhoneCardCmd::Schedule { run_id, at },
        }),
        CardCmd::Wait {
            run_id,
            job,
            json: _,
        } => print_phone(PhoneCmd::Card {
            cmd: PhoneCardCmd::Wait { run_id, job },
        }),
        CardCmd::Run { run_id, json: _ } => print_phone(PhoneCmd::Card {
            cmd: PhoneCardCmd::Run { run_id },
        }),
        CardCmd::Start { run_id, json: _ } => print_phone(PhoneCmd::User {
            cmd: PhoneUserCmd::Start { run_id },
        }),
        CardCmd::Finish { run_id, json: _ } => print_phone(PhoneCmd::User {
            cmd: PhoneUserCmd::Finish { run_id },
        }),
    }
}

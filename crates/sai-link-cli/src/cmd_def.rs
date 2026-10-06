//! `def` グループ（新しい木）— 定義ファイル（block / template / settings）の参照・編集。
//!
//! - `def list` は `phone def list` の実装（block に `templates` が付く）。
//! - `def list --parent <block>` は `block_discover` の形。
//! - `def show --kind block --name N`（`--scope` なし）は `block_inspect` の形。

use crate::cli::{DefCmd, PhoneCmd, PhoneDefCmd};
use crate::cmd_common::{agent_result, other_error, print_json, print_phone, CmdError};

/// `sai-link def <sub>` のハンドラ。
pub(crate) fn run(cmd: DefCmd) -> Result<(), CmdError> {
    match cmd {
        DefCmd::List { parent, json: _ } => match parent {
            // 呼べる子の一覧（subblocks 制限つき）
            Some(parent) => {
                let v = agent_result(sai_link_engine::agent_api::block_discover(Some(
                    parent.as_str(),
                )))?;
                print_json(&v)
            }
            // 定義ファイルの一覧（project → global）
            None => print_phone(PhoneCmd::Def {
                cmd: PhoneDefCmd::List,
            }),
        },
        DefCmd::Show {
            kind,
            scope,
            name,
            json: _,
        } => match scope {
            Some(scope) => print_phone(PhoneCmd::Def {
                cmd: PhoneDefCmd::Show { kind, scope, name },
            }),
            None => {
                // `--scope` を省けるのは解決済みの block 定義のときだけ
                if kind != "block" {
                    return Err(CmdError::input(
                        "--scope を省けるのは --kind block のときだけです",
                    ));
                }
                let v = agent_result(sai_link_engine::agent_api::block_inspect(&name))?;
                print_json(&v)
            }
        },
        DefCmd::Write {
            kind,
            scope,
            name,
            create,
            json: _,
        } => print_phone(PhoneCmd::Def {
            cmd: PhoneDefCmd::Write {
                kind,
                scope,
                name,
                create,
            },
        }),
        DefCmd::Delete {
            kind,
            scope,
            name,
            json: _,
        } => print_phone(PhoneCmd::Def {
            cmd: PhoneDefCmd::Delete { kind, scope, name },
        }),
        DefCmd::Preview {
            definition_file,
            user_input,
            direction,
            timeout,
            round,
            global_round,
            tag,
            think,
            model,
            block_timeout,
        } => crate::cli::handle_block_preview(
            &definition_file,
            user_input.as_deref(),
            &direction,
            timeout,
            round,
            global_round,
            tag,
            think,
            model,
            block_timeout,
        )
        .map_err(other_error),
    }
}

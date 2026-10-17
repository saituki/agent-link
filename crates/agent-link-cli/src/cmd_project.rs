//! `project` グループと `session` グループ（新しい木）。
//!
//! - `project summary` / `project dashboard` は `views` の実装をそのまま使う。
//! - `session search` は旧トップの `search`（`search::run_search_command`）をそのまま使う。

use crate::cli::{ViewCmd, ProjectCmd, SessionCmd};
use crate::cmd_common::{other_error, print_view, CmdError};

/// `agent-link project <sub>` のハンドラ。
pub(crate) fn run(cmd: ProjectCmd) -> Result<(), CmdError> {
    match cmd {
        ProjectCmd::Summary { json: _ } => print_view(ViewCmd::Summary),
        ProjectCmd::Dashboard { period, json: _ } => {
            print_view(ViewCmd::Dashboard { period })
        }
    }
}

/// `agent-link session search …` のハンドラ。
pub(crate) fn run_session(cmd: SessionCmd) -> Result<(), CmdError> {
    match cmd {
        SessionCmd::Search {
            id,
            block,
            date,
            status,
            r#type,
            word,
            project,
            json,
        } => crate::search::run_search_command(crate::search::SearchArgs {
            id,
            block,
            date,
            status,
            r#type,
            word,
            project,
            json,
        })
        .map_err(other_error),
    }
}

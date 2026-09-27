// ============================================================================
// index.ts — pi-litbase 拡張のエントリポイント
// ============================================================================
//
// sai-paper CLI をツールとして登録する。slash command は登録しない
// （コマンドは sai-link の block.md として提供する）。

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { litbaseTools } from "./tools.ts";

export default function (pi: ExtensionAPI) {
  for (const tool of litbaseTools) {
    pi.registerTool(tool);
  }

  console.log(
    `[pi-litbase] ${litbaseTools.length} tools registered: ${litbaseTools
      .map((tool) => tool.name)
      .join(", ")}`,
  );
}

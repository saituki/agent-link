// pi 拡張 API の最小スタブ（型検査用）。
// 実体は pi ランタイムが提供する。`@earendil-works/pi-coding-agent` を
// インストールしていない環境でも `tsc --noEmit` を通すための宣言。
declare module "@earendil-works/pi-coding-agent" {
  /** 拡張登録 API（使用する範囲のみ）。 */
  export interface ExtensionAPI {
    /** LLM が呼べるツールを登録する。 */
    registerTool(tool: unknown): void;
    /** slash command を登録する。 */
    registerCommand(name: string, options: unknown): void;
    /** ライフサイクルイベントを購読する。 */
    on(event: string, handler: (...args: unknown[]) => unknown): void;
  }
}

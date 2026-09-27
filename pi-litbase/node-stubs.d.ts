// Node 標準 API の最小スタブ（型検査用）。
// @types/node を入れずに `tsc --noEmit` を通すための宣言。実行時は Node が提供する。

declare const process: {
  env: Record<string, string | undefined>;
  cwd(): string;
};

declare const console: { log(...args: unknown[]): void };

declare interface AbortSignal {
  readonly aborted: boolean;
}

declare type Buffer = { toString(): string };

declare module "node:child_process" {
  export function execFileSync(
    file: string,
    args?: readonly string[],
    options?: Record<string, unknown>,
  ): unknown;
}

declare module "node:fs" {
  export function existsSync(path: string): boolean;
}

declare module "node:path" {
  export function join(...parts: string[]): string;
  export function dirname(path: string): string;
}

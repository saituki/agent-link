// ============================================================================
// sai-paper-cli.ts — sai-moon-paper CLI の解決と実行ラッパ
// ============================================================================
//
// pi-litbase 拡張の全ツールは、このラッパ経由で `sai-moon-paper` CLI を実行する。
// pi agent は papr.db を直接操作せず、必ず CLI（決定論的 API）を経由する。

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";

/** `sai-moon-paper` バイナリ名（または絶対パス）。環境変数 `SAI_PAPER_BIN` で上書き可能。 */
export function saiPaperBinary(): string {
  return process.env.SAI_PAPER_BIN ?? "sai-moon-paper";
}

/**
 * 研究プロジェクトルートを解決する。
 *
 * 優先順:
 * 1. 環境変数 `SAI_PAPER_ROOT`
 * 2. cwd から上向きに `.sai/paper/` または `.sai/` を探索
 * 3. cwd
 */
export function saiPaperRoot(): string {
  const env = process.env.SAI_PAPER_ROOT;
  if (env && env.length > 0) {
    return env;
  }
  let dir = process.cwd();
  for (let i = 0; i < 8; i += 1) {
    if (existsSync(join(dir, ".sai", "paper"))) {
      return dir;
    }
    if (existsSync(join(dir, ".sai"))) {
      return dir;
    }
    const parent = dirname(dir);
    if (parent === dir) {
      break;
    }
    dir = parent;
  }
  return process.cwd();
}

/** `runSaiPaper` のオプション。 */
export interface RunOptions {
  /** タイムアウト（ms）。既定 120 秒。 */
  timeoutMs?: number;
  /** 標準入力へ渡す文字列（`--file -` 用）。 */
  input?: string;
}

/**
 * `sai-moon-paper` CLI を実行し、標準出力を JSON として返す。
 *
 * JSON でない場合は `{ text }` を返す。失敗時は stderr を含む Error を投げる。
 */
export function runSaiPaper(args: string[], options: RunOptions = {}): unknown {
  const bin = saiPaperBinary();
  const root = saiPaperRoot();
  try {
    const output = execFileSync(bin, args, {
      cwd: root,
      encoding: "utf-8",
      timeout: options.timeoutMs ?? 120_000,
      input: options.input,
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, SAI_PAPER_ROOT: root },
      maxBuffer: 64 * 1024 * 1024,
    });
    const text = String(output).trim();
    if (text.length === 0) {
      return {};
    }
    try {
      return JSON.parse(text);
    } catch {
      return { text };
    }
  } catch (error) {
    const err = error as {
      stderr?: Buffer | string;
      stdout?: Buffer | string;
      message?: string;
    };
    const stderr = err.stderr ? String(err.stderr).trim() : "";
    const stdout = err.stdout ? String(err.stdout).trim() : "";
    throw new Error(
      stderr || stdout || err.message || "sai-paper の実行に失敗しました",
    );
  }
}

/** ツール結果（JSON テキスト）。 */
export function jsonResult(value: unknown) {
  const text =
    typeof value === "string" ? value : JSON.stringify(value, null, 2);
  return {
    content: [{ type: "text" as const, text }],
    details: {},
  };
}

/** ツール結果（エラー）。 */
export function errorResult(message: string) {
  return {
    content: [
      { type: "text" as const, text: JSON.stringify({ error: message }) },
    ],
    details: {},
  };
}

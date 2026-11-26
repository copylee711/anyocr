// 设置、文件扫描、批量任务相关的 Tauri 命令与事件。
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { ParserKey } from "./ocr";

export type OutputFormat = "txt" | "txt_plain" | "txt_individual" | "md" | "csv" | "jsonl";
export const OUTPUT_FORMATS: OutputFormat[] = ["txt", "txt_plain", "txt_individual", "md", "csv", "jsonl"];
export type LayerMode = "skip-text" | "force" | "redo";

export interface OutputArgs {
  dirType: "source" | "custom";
  dir: string;
  formats: OutputFormat[];
  fileNameTemplate: string;
  datetimeFormat: string;
  ignoreBlank: boolean;
}

export interface DocJobArgs {
  files: string[];
  mode: LayerMode;
  pages: string;
  dpi: number;
  parser: ParserKey;
  password?: string;
  writePdf: boolean;
  pdfSuffix: string;
  output: OutputArgs;
}

export interface BatchJobArgs {
  files: string[];
  parser: ParserKey;
  output: OutputArgs;
}

export type PageStatus =
  | { status: "skipped"; text_chars: number }
  | { status: "done"; blocks: number; chars: number; ocr_ms: number; score: number }
  | { status: "failed"; error: string };
export type PageReport = { page: number } & PageStatus;

export interface Report {
  total_pages: number;
  pages: PageReport[];
  elapsed_ms: number;
  render_ms: number;
  ocr_ms: number;
  input_bytes: number;
  output_bytes: number;
}

export type JobEvent =
  | { kind: "file_start"; job: number; file: number }
  | { kind: "planned"; job: number; file: number; total: number; toOcr: number }
  | { kind: "page"; job: number; file: number; page: PageReport }
  | { kind: "progress"; job: number; file: number; stage: string; done: number; total: number }
  | {
      kind: "file_done";
      job: number;
      file: number;
      ok: boolean;
      error: string | null;
      elapsedMs: number;
      outputs: string[];
      report: Report | null;
      text: string | null;
      blocks: number;
    }
  | { kind: "job_done"; job: number; cancelled: boolean; outputs: string[]; error: string | null };

export interface AppInfo {
  version: string;
  modelsDir: string;
}

export const getSettings = () => invoke<Record<string, unknown>>("get_settings");
export const saveSettings = (value: unknown) => invoke<void>("save_settings", { value });
export const appInfo = () => invoke<AppInfo>("app_info");
export const scanFiles = (paths: string[], recurse: boolean, kind: "image" | "doc") =>
  invoke<string[]>("scan_files", { paths, recurse, kind });
export const docStart = (args: DocJobArgs) => invoke<number>("doc_start", { args });
export const batchStart = (args: BatchJobArgs) => invoke<number>("batch_start", { args });
export const jobCancel = (job: number) => invoke<boolean>("job_cancel", { job });

export function onJobEvent(cb: (e: JobEvent) => void): Promise<UnlistenFn> {
  if (!isTauri()) return Promise.resolve(() => {});
  return listen<JobEvent>("job", (e) => cb(e.payload));
}

/** 从路径取文件名（兼容 Windows 与 POSIX 分隔符）。 */
export function baseName(p: string): string {
  return p.split(/[\\/]/).pop() ?? p;
}

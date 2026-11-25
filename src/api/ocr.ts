// 与 Rust 侧 anyocr-core 数据模型对应的类型，以及 Tauri 命令封装。
import { invoke } from "@tauri-apps/api/core";

export type Point = [number, number];

export interface TextBlock {
  text: string;
  box: [Point, Point, Point, Point];
  score: number;
  end: string;
  from?: "ocr" | "text" | "reviewed";
}

export interface PageResult {
  width: number;
  height: number;
  blocks: TextBlock[];
  elapsed_ms: number;
  engine: string;
}

export const PARSERS = [
  "multi_para",
  "multi_line",
  "multi_none",
  "single_para",
  "single_line",
  "single_none",
  "single_code",
  "none",
] as const;
export type ParserKey = (typeof PARSERS)[number];

/** 拼接全文（与 Rust `PageResult::text` 一致）。 */
export function pageText(page: PageResult): string {
  return page.blocks.map((b) => b.text + b.end).join("");
}

/** 识别本地图片文件。 */
export function ocrImageFile(path: string, parser: ParserKey): Promise<PageResult> {
  return invoke<PageResult>("ocr_image_file", { path, parser });
}

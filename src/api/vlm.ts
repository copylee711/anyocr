// 视觉大模型：服务预设、测试连接、单张图片复核。密钥用 online.ts 的 secretSet（引擎名 `vlm_<服务>`）。
import { invoke } from "@tauri-apps/api/core";

import type { PageResult } from "./ocr";

export interface VlmPreset {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  models: string[];
  keyUrl: string;
  needsKey: boolean;
  extra: string;
  note: string;
  verified: boolean;
}

export interface LineChange {
  index: number;
  before: string;
  after: string;
  accepted: boolean;
  reason?: string;
}

export interface ReviewOutcome {
  changes: LineChange[];
  elapsed_ms: number;
  model: string;
}

export interface ReviewResult {
  outcome: ReviewOutcome;
  page: PageResult;
  text: string;
  recopied: boolean;
}

export interface VlmTestResult {
  ok: boolean;
  elapsedMs: number;
  fixed: number;
  model: string;
  error: string | null;
}

export const vlmSecretId = (provider: string) => `vlm_${provider}`;

export const vlmPresets = () => invoke<VlmPreset[]>("vlm_presets");

export const vlmTest = () => invoke<VlmTestResult>("vlm_test");

/** 复核一张图片的识别结果；`recopy` 为已复制到剪贴板的原文，剪贴板未变时替换为新文本。 */
export const reviewImage = (path: string, page: PageResult, recopy: string | null) =>
  invoke<ReviewResult>("review_image", { path, page, recopy });

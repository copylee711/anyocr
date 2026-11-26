// 在线引擎：预设、密钥（存系统凭据管理器）、测试连接、MinerU 文档转 Markdown。
import { invoke } from "@tauri-apps/api/core";

export interface EngineField {
  key: string;
  label: string;
  secret: boolean;
  required: boolean;
  default?: string;
  options?: string[];
  hint?: string;
}

export interface EngineInfo {
  id: string;
  name: string;
  kind: "local" | "online" | "vlm";
  key_url?: string;
  homepage?: string;
  has_boxes: boolean;
}

export interface Preset {
  info: EngineInfo;
  fields: EngineField[];
  note: string;
  verified: boolean;
}

export interface Presets {
  engines: Preset[];
  mineru: Preset;
}

export interface TestResult {
  ok: boolean;
  elapsedMs: number;
  text: string;
  error: string | null;
}

export const onlinePresets = () => invoke<Presets>("online_presets");

/** 字段 → 已保存密钥的末 4 位；未保存为 null。 */
export const secretStatus = (engine: string) => invoke<Record<string, string | null>>("secret_status", { engine });

/** 保存密钥；空字符串表示清除。 */
export const secretSet = (engine: string, field: string, value: string) =>
  invoke<void>("secret_set", { engine, field, value });

export const onlineTest = (engine: string) => invoke<TestResult>("online_test", { engine });

export const mineruStart = (args: { files: string[]; pages: string; outDir: string }) =>
  invoke<number>("mineru_start", { args });

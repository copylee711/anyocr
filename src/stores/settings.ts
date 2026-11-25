import { defineStore } from "pinia";
import { isTauri } from "@tauri-apps/api/core";
import { ref, watch } from "vue";

import { getSettings, type LayerMode, type OutputArgs, saveSettings } from "../api/app";
import type { ParserKey } from "../api/ocr";

export interface Settings {
  engine: {
    model: string;
    device: "auto" | "cpu" | "directml" | "cuda";
    workers: number;
    textlineOrientation: boolean;
  };
  ui: { theme: "system" | "light" | "dark"; language: "auto" | "zh-CN" | "en" };
  screenshot: { parser: ParserKey; copy: boolean; popMainWindow: boolean; hideWindow: boolean };
  hotkeys: { screenshot: string; paste: string; reScreenshot: string; qrScreenshot: string };
  qrcode: {
    preprocess: { medianFilterSize: number; sharpnessFactor: number; contrastFactor: number; threshold: number };
    write: { format: string; width: number; height: number; quietZone: number; ecLevel: string };
  };
  doc: {
    mode: LayerMode;
    pages: string;
    dpi: number;
    parser: ParserKey;
    writePdf: boolean;
    pdfSuffix: string;
    output: OutputArgs;
  };
  batch: { parser: ParserKey; recurse: boolean; output: OutputArgs };
}

const defaultOutput = (formats: OutputArgs["formats"]): OutputArgs => ({
  dirType: "source",
  dir: "",
  formats,
  fileNameTemplate: "[OCR]_%name_%date",
  datetimeFormat: "%Y-%m-%d_%H-%M",
  ignoreBlank: true,
});

export function defaultSettings(): Settings {
  return {
    engine: { model: "pp-ocrv6-small", device: "auto", workers: 0, textlineOrientation: false },
    ui: { theme: "system", language: "auto" },
    screenshot: { parser: "multi_para", copy: true, popMainWindow: true, hideWindow: true },
    hotkeys: { screenshot: "F4", paste: "", reScreenshot: "", qrScreenshot: "" },
    qrcode: {
      preprocess: { medianFilterSize: 0, sharpnessFactor: 1, contrastFactor: 1, threshold: 0 },
      write: { format: "QR_CODE", width: 256, height: 256, quietZone: 2, ecLevel: "M" },
    },
    doc: {
      mode: "skip-text",
      pages: "",
      dpi: 200,
      parser: "multi_para",
      writePdf: true,
      pdfSuffix: ".ocr",
      output: defaultOutput([]),
    },
    batch: { parser: "multi_para", recurse: true, output: defaultOutput(["txt"]) },
  };
}

/** 以默认值为骨架合并已保存的设置（只接受类型一致的字段，兼容旧版本配置）。 */
export function mergeSettings<T>(base: T, saved: unknown): T {
  if (Array.isArray(base)) return (Array.isArray(saved) ? saved : base) as T;
  if (base && typeof base === "object") {
    const out: Record<string, unknown> = { ...(base as Record<string, unknown>) };
    if (saved && typeof saved === "object" && !Array.isArray(saved)) {
      for (const [k, v] of Object.entries(saved as Record<string, unknown>)) {
        if (k in out) out[k] = mergeSettings(out[k], v);
      }
    }
    return out as T;
  }
  return (typeof saved === typeof base ? saved : base) as T;
}

export const useSettings = defineStore("settings", () => {
  const s = ref<Settings>(defaultSettings());
  const loaded = ref(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function load() {
    if (isTauri()) {
      try {
        s.value = mergeSettings(defaultSettings(), await getSettings());
      } catch (e) {
        console.error("读取设置失败", e);
      }
    }
    loaded.value = true;
  }

  // 任意修改 400ms 后自动保存
  watch(
    s,
    (v) => {
      if (!loaded.value || !isTauri()) return;
      clearTimeout(timer);
      timer = setTimeout(() => saveSettings(JSON.parse(JSON.stringify(v))).catch(console.error), 400);
    },
    { deep: true },
  );

  return { s, loaded, load };
});

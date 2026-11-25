// 截图 / 粘贴识别的最新结果（全局保存，由 App 中的 `capture-result` 事件驱动）。
import { defineStore } from "pinia";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ref } from "vue";

import type { PageResult } from "../api/ocr";

export type CaptureMode = "ocr" | "qrcode";

export interface QrCode {
  format: string;
  text: string;
  points: [number, number][];
}

export interface CaptureResult {
  mode: CaptureMode;
  image: string;
  page: PageResult | null;
  codes: QrCode[];
  text: string;
  error: string | null;
}

export const captureStart = (mode: CaptureMode) => invoke<void>("capture_start", { mode });
export const captureRepeat = (mode: CaptureMode) => invoke<void>("capture_repeat", { mode });
export const clipboardRecognize = (mode: CaptureMode) => invoke<void>("clipboard_recognize", { mode });

export const useCapture = defineStore("capture", () => {
  const ocr = ref<CaptureResult | null>(null);
  const qr = ref<CaptureResult | null>(null);
  const busy = ref(false);
  const lastError = ref<string | null>(null);

  function listenAll(onResult: (r: CaptureResult) => void, onError: (e: string) => void): Promise<UnlistenFn[]> {
    if (!isTauri()) return Promise.resolve([]);
    return Promise.all([
      listen<CaptureResult>("capture-result", (e) => {
        busy.value = false;
        if (e.payload.mode === "ocr") ocr.value = e.payload;
        else qr.value = e.payload;
        onResult(e.payload);
      }),
      listen<string>("capture-error", (e) => {
        busy.value = false;
        lastError.value = e.payload;
        onError(e.payload);
      }),
    ]);
  }

  return { ocr, qr, busy, lastError, listenAll };
});

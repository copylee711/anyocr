// 截图 / 粘贴识别的最新结果（全局保存，由 App 中的 `capture-result` 事件驱动）。
import { defineStore } from "pinia";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ref } from "vue";

import { pageText, type PageResult } from "../api/ocr";
import { type LineChange, reviewImage } from "../api/vlm";

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

/** 截图结果的复核状态。 */
export interface ReviewState {
  status: "running" | "done" | "failed";
  /** 复核前的页面，用于撤销 */
  original: PageResult;
  changes: LineChange[];
  elapsedMs?: number;
  model?: string;
  error?: string;
  undone?: boolean;
}

export const useCapture = defineStore("capture", () => {
  const ocr = ref<CaptureResult | null>(null);
  const qr = ref<CaptureResult | null>(null);
  const busy = ref(false);
  const lastError = ref<string | null>(null);
  const review = ref<ReviewState | null>(null);
  let seq = 0;

  /** 用视觉大模型复核当前截图结果；OCR 结果先显示，复核返回后就地更新。`recopy` 为已复制的原文。 */
  async function startReview(recopy: string | null) {
    const cur = ocr.value;
    if (!cur?.page || !cur.page.blocks.length) return;
    const id = ++seq;
    const original = JSON.parse(JSON.stringify(cur.page)) as PageResult;
    review.value = { status: "running", original, changes: [] };
    try {
      const r = await reviewImage(cur.image, cur.page, recopy);
      if (id !== seq || ocr.value !== cur) return; // 期间已有新的截图
      cur.page = r.page;
      cur.text = r.text;
      review.value = {
        status: "done",
        original,
        changes: r.outcome.changes,
        elapsedMs: r.outcome.elapsed_ms,
        model: r.outcome.model,
      };
    } catch (e) {
      if (id === seq) review.value = { status: "failed", original, changes: [], error: String(e) };
    }
  }

  /** 撤销复核，恢复 OCR 原文。 */
  function undoReview() {
    if (!ocr.value || !review.value) return;
    ocr.value.page = review.value.original;
    ocr.value.text = pageText(review.value.original);
    review.value.undone = true;
  }

  /** 新结果到来时清除上一次的复核状态。 */
  function resetReview() {
    seq++;
    review.value = null;
  }

  function listenAll(onResult: (r: CaptureResult) => void, onError: (e: string) => void): Promise<UnlistenFn[]> {
    if (!isTauri()) return Promise.resolve([]);
    return Promise.all([
      listen<CaptureResult>("capture-result", (e) => {
        busy.value = false;
        if (e.payload.mode === "ocr") {
          resetReview();
          ocr.value = e.payload;
        } else qr.value = e.payload;
        onResult(e.payload);
      }),
      listen<string>("capture-error", (e) => {
        busy.value = false;
        lastError.value = e.payload;
        onError(e.payload);
      }),
    ]);
  }

  return { ocr, qr, busy, lastError, review, startReview, undoReview, resetReview, listenAll };
});

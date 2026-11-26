<script setup lang="ts">
// 截图识别：快捷键 / 按钮截图、粘贴图片、打开图片。结果来自全局 capture store。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { NButton, NSelect, NSpin, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";

import { ocrImageFile, type PageResult, pageText, PARSERS, type ParserKey } from "../api/ocr";
import AppIcon from "../components/AppIcon.vue";
import { captureRepeat, captureStart, clipboardRecognize, useCapture } from "../stores/capture";
import { useSettings } from "../stores/settings";

const { t } = useI18n();
const message = useMessage();
const settings = useSettings();
const capture = useCapture();

const busy = ref(false);
const parser = computed<ParserKey>({
  get: () => settings.s.screenshot.parser,
  set: (v) => (settings.s.screenshot.parser = v),
});
const hovered = ref<number | null>(null);
const result = computed<PageResult | null>(() => capture.ocr?.page ?? null);
const imagePath = computed(() => capture.ocr?.image ?? null);

const parserOptions = computed(() => PARSERS.map((p) => ({ label: t(`parser.${p}`), value: p })));
const text = computed(() => (result.value ? pageText(result.value) : ""));
const imageSrc = computed(() => (imagePath.value ? convertFileSrc(imagePath.value) : ""));
const hotkey = computed(() => settings.s.hotkeys.screenshot);

async function recognizeFile(path: string) {
  busy.value = true;
  try {
    const page = await ocrImageFile(path, parser.value);
    capture.resetReview();
    capture.ocr = { mode: "ocr", image: path, page, codes: [], text: pageText(page), error: null };
    if (settings.s.vlm.review.screenshot && page.blocks.length) capture.startReview(null);
  } catch (e) {
    message.error(String(e));
  } finally {
    busy.value = false;
  }
}

/** 换排版方案后，用同一张图重新识别。 */
function rerun() {
  if (imagePath.value) recognizeFile(imagePath.value);
}

async function pick() {
  const path = await open({
    multiple: false,
    filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff", "gif"] }],
  });
  if (typeof path === "string") await recognizeFile(path);
}

async function call(fn: () => Promise<void>) {
  try {
    await fn();
  } catch (e) {
    message.warning(String(e));
  }
}

const shoot = () => call(() => captureStart("ocr"));
const paste = () => call(() => clipboardRecognize("ocr"));
const again = () => call(() => captureRepeat("ocr"));

function onKey(e: KeyboardEvent) {
  // 页面内 Ctrl+V：识别剪贴板图片（输入框内除外）
  const tag = (e.target as HTMLElement | null)?.tagName;
  if (e.ctrlKey && e.key.toLowerCase() === "v" && tag !== "INPUT" && tag !== "TEXTAREA") paste();
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));

async function copy() {
  await navigator.clipboard.writeText(text.value);
  message.success(t("common.copied"));
}

/** 被复核修改过的行：行号 → 原文（鼠标悬停显示）。 */
const reviewedBefore = computed(() => {
  const m = new Map<number, string>();
  const r = capture.review;
  if (r && r.status === "done" && !r.undone) for (const c of r.changes) if (c.accepted) m.set(c.index, c.before);
  return m;
});

const reviewLabel = computed(() => {
  const r = capture.review;
  if (!r) return "";
  if (r.status === "running") return t("screenshot.reviewing");
  if (r.status === "failed") return t("screenshot.reviewFailed");
  if (r.undone) return t("screenshot.reviewUndone");
  const n = r.changes.filter((c) => c.accepted).length;
  return n
    ? t("screenshot.reviewFixed", { n, ms: r.elapsedMs ?? 0 })
    : t("screenshot.reviewClean", { ms: r.elapsedMs ?? 0 });
});

function polygon(b: PageResult["blocks"][number]) {
  return b.box.map(([x, y]) => `${x},${y}`).join(" ");
}
</script>

<template>
  <section class="page">
    <header class="page-head row">
      <div>
        <h1>{{ t("screenshot.title") }}</h1>
        <p>{{ t("screenshot.desc") }}</p>
      </div>
      <div class="actions">
        <NSelect
          v-model:value="parser"
          :options="parserOptions"
          size="small"
          style="width: 168px"
          @update:value="rerun"
        />
        <NButton @click="pick">
          <template #icon><AppIcon name="open" :size="16" /></template>
          {{ t("screenshot.open") }}
        </NButton>
        <NButton @click="paste">{{ t("screenshot.paste") }}</NButton>
        <NButton v-if="capture.ocr" quaternary @click="again">{{ t("screenshot.again") }}</NButton>
        <NButton type="primary" @click="shoot">
          <template #icon><AppIcon name="scan" :size="16" /></template>
          {{ t("screenshot.capture") }}
          <kbd v-if="hotkey" class="hk">{{ hotkey }}</kbd>
        </NButton>
      </div>
    </header>

    <div class="workspace">
      <div class="card viewer">
        <NSpin :show="busy" :description="result ? t('screenshot.recognizing') : t('screenshot.loadingEngine')">
          <div v-if="imagePath" class="canvas">
            <img :src="imageSrc" alt="" />
            <svg
              v-if="result"
              class="overlay"
              :viewBox="`0 0 ${result.width} ${result.height}`"
              preserveAspectRatio="xMidYMid meet"
            >
              <polygon
                v-for="(b, i) in result.blocks"
                :key="i"
                :points="polygon(b)"
                :class="{ on: hovered === i }"
                @mouseenter="hovered = i"
                @mouseleave="hovered = null"
              />
            </svg>
          </div>
          <div v-else class="placeholder" @click="shoot">
            <AppIcon name="scan" :size="36" />
            <strong>{{ t("screenshot.empty") }}</strong>
            <span>{{ t("screenshot.emptyHint", { key: hotkey || "—" }) }}</span>
          </div>
        </NSpin>
      </div>

      <div class="card result">
        <div class="result-head">
          <span v-if="result" class="meta">
            {{ t("common.blocks", { n: result.blocks.length }) }} · {{ t("common.elapsed", { ms: result.elapsed_ms }) }}
          </span>
          <span
            v-if="capture.review"
            class="review-chip"
            :class="capture.review.status"
            :title="capture.review.error ?? capture.review.model ?? ''"
          >
            <AppIcon name="sparkle" :size="13" />
            {{ reviewLabel }}
          </span>
          <span class="grow" />
          <NButton
            v-if="capture.review?.status === 'done' && !capture.review.undone && reviewedBefore.size"
            size="small"
            quaternary
            @click="capture.undoReview()"
          >
            {{ t("screenshot.undoReview") }}
          </NButton>
          <NButton
            v-else-if="result?.blocks.length && capture.review?.status !== 'running'"
            size="small"
            quaternary
            @click="capture.startReview(null)"
          >
            <template #icon><AppIcon name="sparkle" :size="15" /></template>
            {{ t("screenshot.review") }}
          </NButton>
          <NButton size="small" quaternary :disabled="!text" @click="copy">
            <template #icon><AppIcon name="copy" :size="15" /></template>
            {{ t("common.copy") }}
          </NButton>
        </div>
        <div class="text">
          <template v-if="result">
            <span
              v-for="(b, i) in result.blocks"
              :key="i"
              :class="{ on: hovered === i, fixed: reviewedBefore.has(i) }"
              :title="reviewedBefore.has(i) ? t('screenshot.before', { text: reviewedBefore.get(i) }) : undefined"
              @mouseenter="hovered = i"
              @mouseleave="hovered = null"
              >{{ b.text }}{{ b.end }}</span
            >
          </template>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.row {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}
.actions {
  display: flex;
  gap: 10px;
  align-items: center;
}
.workspace {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 1.15fr) minmax(0, 1fr);
  gap: 16px;
}
.viewer {
  overflow: hidden;
  padding: 12px;
  background: var(--c-surface-2);
}
.viewer :deep(.n-spin-container),
.viewer :deep(.n-spin-content) {
  height: 100%;
}
.canvas {
  position: relative;
  height: 100%;
  display: grid;
  place-items: center;
}
.canvas img,
.overlay {
  grid-area: 1 / 1;
  max-width: 100%;
  max-height: 100%;
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.overlay polygon {
  fill: color-mix(in srgb, var(--c-primary) 8%, transparent);
  stroke: var(--c-primary);
  stroke-width: 1.5;
  vector-effect: non-scaling-stroke;
  transition: fill 0.12s;
}
.overlay polygon.on {
  fill: color-mix(in srgb, var(--c-accent) 30%, transparent);
  stroke: var(--c-accent);
}
.placeholder {
  height: 100%;
  min-height: 320px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--c-text-3);
  border: 1.5px dashed var(--c-border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition:
    border-color 0.15s,
    color 0.15s;
}
.placeholder strong {
  color: var(--c-text);
  font-size: 15px;
}
.placeholder span {
  font-size: 12.5px;
}
.hk {
  margin-left: 6px;
  padding: 0 5px;
  border-radius: 4px;
  font: 600 11px/16px var(--font-ui);
  background: rgba(255, 255, 255, 0.22);
}
.placeholder:hover {
  border-color: var(--c-primary);
  color: var(--c-primary);
}
.result {
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.result-head {
  display: flex;
  gap: 10px;
  align-items: center;
  padding: 10px 12px 10px 16px;
  border-bottom: 1px solid var(--c-border);
  min-height: 48px;
}
.meta {
  color: var(--c-text-2);
  font-size: 12.5px;
  white-space: nowrap;
}
.text {
  flex: 1;
  overflow: auto;
  padding: 14px 16px;
  white-space: pre-wrap;
  line-height: 1.75;
  user-select: text;
}
.text span.on {
  background: color-mix(in srgb, var(--c-accent) 22%, transparent);
  border-radius: 4px;
}
.text span.fixed {
  background: color-mix(in srgb, #34c759 18%, transparent);
  box-shadow: inset 0 -1.5px 0 #34c759;
  border-radius: 3px;
}
.grow {
  flex: 1;
}
.review-chip {
  white-space: nowrap;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  padding: 2px 8px;
  border-radius: 999px;
  color: var(--c-primary);
  background: var(--c-primary-soft);
}
.review-chip.running :deep(svg) {
  animation: pulse 1.2s ease-in-out infinite;
}
.review-chip.failed {
  color: #d70015;
  background: color-mix(in srgb, #ff3b30 10%, transparent);
}
@keyframes pulse {
  50% {
    opacity: 0.3;
  }
}
</style>

<script setup lang="ts">
import { computed, ref } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { NButton, NSelect, NSpin, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";

import { ocrImageFile, type PageResult, pageText, PARSERS, type ParserKey } from "../api/ocr";
import AppIcon from "../components/AppIcon.vue";

const { t } = useI18n();
const message = useMessage();

const imagePath = ref<string | null>(null);
const result = ref<PageResult | null>(null);
const busy = ref(false);
const parser = ref<ParserKey>("multi_para");
const hovered = ref<number | null>(null);

const parserOptions = computed(() => PARSERS.map((p) => ({ label: t(`parser.${p}`), value: p })));
const text = computed(() => (result.value ? pageText(result.value) : ""));
const imageSrc = computed(() => (imagePath.value ? convertFileSrc(imagePath.value) : ""));

async function run() {
  if (!imagePath.value) return;
  busy.value = true;
  try {
    result.value = await ocrImageFile(imagePath.value, parser.value);
  } catch (e) {
    message.error(String(e));
  } finally {
    busy.value = false;
  }
}

async function pick() {
  const path = await open({
    multiple: false,
    filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff", "gif"] }],
  });
  if (typeof path !== "string") return;
  imagePath.value = path;
  result.value = null;
  await run();
}

async function copy() {
  await navigator.clipboard.writeText(text.value);
  message.success(t("common.copied"));
}

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
          @update:value="run"
        />
        <NButton type="primary" @click="pick">
          <template #icon><AppIcon name="open" :size="16" /></template>
          {{ t("screenshot.open") }}
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
          <div v-else class="placeholder" @click="pick">
            <AppIcon name="scan" :size="36" />
            <span>{{ t("screenshot.empty") }}</span>
          </div>
        </NSpin>
      </div>

      <div class="card result">
        <div class="result-head">
          <span v-if="result" class="meta">
            {{ t("common.blocks", { n: result.blocks.length }) }} · {{ t("common.elapsed", { ms: result.elapsed_ms }) }}
          </span>
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
              :class="{ on: hovered === i }"
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
  justify-content: space-between;
  align-items: center;
  padding: 10px 12px 10px 16px;
  border-bottom: 1px solid var(--c-border);
  min-height: 48px;
}
.meta {
  color: var(--c-text-2);
  font-size: 12.5px;
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
</style>

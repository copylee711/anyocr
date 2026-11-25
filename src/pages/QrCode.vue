<script setup lang="ts">
// 二维码：截图 / 粘贴 / 打开图片识别（可预处理）；由文本生成多种码制并保存。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { convertFileSrc, invoke, isTauri } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { NButton, NInput, NInputNumber, NSelect, NSlider, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";

import AppIcon from "../components/AppIcon.vue";
import Group from "../components/ui/Group.vue";
import Row from "../components/ui/Row.vue";
import Segmented from "../components/ui/Segmented.vue";
import { captureStart, clipboardRecognize, type QrCode, useCapture } from "../stores/capture";
import { useSettings } from "../stores/settings";

const { t } = useI18n();
const message = useMessage();
const settings = useSettings();
const capture = useCapture();
const pre = computed(() => settings.s.qrcode.preprocess);
const wo = computed(() => settings.s.qrcode.write);

const tab = ref<"read" | "write">("read");
const codes = computed<QrCode[]>(() => capture.qr?.codes ?? []);
const image = computed(() => (capture.qr?.image ? convertFileSrc(capture.qr.image) : ""));

async function call(fn: () => Promise<void>) {
  try {
    await fn();
  } catch (e) {
    message.warning(String(e));
  }
}

async function readFile(path: string) {
  const found = await invoke<QrCode[]>("qr_read_file", { path, preprocess: { ...pre.value } });
  capture.qr = {
    mode: "qrcode",
    image: path,
    page: null,
    codes: found,
    text: found.map((c) => c.text).join("\n"),
    error: null,
  };
  if (!found.length) message.info(t("qrcode.none"));
}

async function pick() {
  const path = await open({
    multiple: false,
    filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "bmp", "webp", "gif"] }],
  });
  if (typeof path === "string") await call(() => readFile(path));
}

/** 修改预处理参数后，对当前图片重新识别。 */
function reread() {
  if (capture.qr?.image) call(() => readFile(capture.qr!.image));
}

async function copy(text: string) {
  await navigator.clipboard.writeText(text);
  message.success(t("common.copied"));
}

// ---------- 生成 ----------
const text = ref("https://github.com/copylee711/anyocr");
const preview = ref("");
let timer: ReturnType<typeof setTimeout> | undefined;

async function generate(path?: string) {
  if (!isTauri() || !text.value) return "";
  return invoke<string>("qr_write", { text: text.value, options: { ...wo.value }, path: path ?? null });
}

watch(
  [text, () => JSON.stringify(wo.value)],
  () => {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      try {
        const p = await generate();
        preview.value = p ? `${convertFileSrc(p)}?t=${Date.now()}` : "";
      } catch (e) {
        preview.value = "";
        message.warning(String(e));
      }
    }, 250);
  },
  { immediate: true },
);

async function saveAs() {
  const path = await save({ defaultPath: "code.png", filters: [{ name: "PNG", extensions: ["png"] }] });
  if (path)
    await call(async () => {
      await generate(path);
      message.success(t("qrcode.saved"));
    });
}

const formats = [
  "QR_CODE",
  "DATA_MATRIX",
  "AZTEC",
  "PDF_417",
  "CODE_128",
  "CODE_39",
  "EAN_13",
  "EAN_8",
  "UPC_A",
  "ITF",
];

function onKey(e: KeyboardEvent) {
  const tag = (e.target as HTMLElement | null)?.tagName;
  if (tab.value === "read" && e.ctrlKey && e.key.toLowerCase() === "v" && tag !== "INPUT" && tag !== "TEXTAREA") {
    call(() => clipboardRecognize("qrcode"));
  }
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <section class="page">
    <header class="page-head row">
      <div>
        <h1>{{ t("qrcode.title") }}</h1>
        <p>{{ t("qrcode.desc") }}</p>
      </div>
      <Segmented
        v-model="tab"
        :options="[
          { label: t('qrcode.read'), value: 'read' },
          { label: t('qrcode.write'), value: 'write' },
        ]"
      />
    </header>

    <div v-if="tab === 'read'" class="workspace">
      <div class="left">
        <div class="toolbar">
          <NButton type="primary" @click="call(() => captureStart('qrcode'))">
            <template #icon><AppIcon name="scan" :size="16" /></template>
            {{ t("screenshot.capture") }}
          </NButton>
          <NButton @click="call(() => clipboardRecognize('qrcode'))">{{ t("screenshot.paste") }}</NButton>
          <NButton @click="pick">
            <template #icon><AppIcon name="open" :size="16" /></template>
            {{ t("screenshot.open") }}
          </NButton>
        </div>
        <div class="card viewer">
          <img v-if="image" :src="image" alt="" />
          <div v-else class="placeholder">
            <AppIcon name="qrcode" :size="34" />
            <span>{{ t("qrcode.empty") }}</span>
          </div>
        </div>
        <div class="card results">
          <div v-if="!codes.length" class="muted">{{ capture.qr ? t("qrcode.none") : t("qrcode.resultsHint") }}</div>
          <div v-for="(c, i) in codes" :key="i" class="code">
            <span class="fmt">{{ c.format }}</span>
            <span class="txt">{{ c.text }}</span>
            <NButton size="tiny" quaternary @click="copy(c.text)">{{ t("common.copy") }}</NButton>
          </div>
        </div>
      </div>
      <aside class="options">
        <Group :title="t('qrcode.preprocess')" :footer="t('qrcode.preprocessHint')">
          <Row :label="t('qrcode.median')">
            <NSelect
              v-model:value="pre.medianFilterSize"
              size="small"
              style="width: 110px"
              :options="[0, 3, 5, 7].map((v) => ({ label: v ? `${v}×${v}` : t('qrcode.off'), value: v }))"
              @update:value="reread"
            />
          </Row>
          <Row :label="t('qrcode.sharpness')" stacked>
            <NSlider v-model:value="pre.sharpnessFactor" :min="0" :max="4" :step="0.1" @dragend="reread" />
          </Row>
          <Row :label="t('qrcode.contrast')" stacked>
            <NSlider v-model:value="pre.contrastFactor" :min="0" :max="4" :step="0.1" @dragend="reread" />
          </Row>
          <Row :label="t('qrcode.threshold')" :hint="t('qrcode.thresholdHint')">
            <NInputNumber
              v-model:value="pre.threshold"
              size="small"
              :min="0"
              :max="255"
              style="width: 110px"
              @blur="reread"
            />
          </Row>
        </Group>
      </aside>
    </div>

    <div v-else class="workspace">
      <div class="left">
        <NInput
          v-model:value="text"
          type="textarea"
          :autosize="{ minRows: 4, maxRows: 10 }"
          :placeholder="t('qrcode.textPlaceholder')"
        />
        <div class="card viewer gen">
          <img v-if="preview" :src="preview" alt="" class="pixel" />
          <div v-else class="placeholder">
            <span>{{ t("qrcode.empty") }}</span>
          </div>
        </div>
      </div>
      <aside class="options">
        <Group :title="t('qrcode.writeOptions')">
          <Row :label="t('qrcode.format')">
            <NSelect
              v-model:value="wo.format"
              size="small"
              style="width: 150px"
              :options="formats.map((f) => ({ label: f, value: f }))"
            />
          </Row>
          <Row :label="t('qrcode.size')">
            <NInputNumber v-model:value="wo.width" size="small" :min="32" :max="4096" style="width: 90px" />
            ×
            <NInputNumber v-model:value="wo.height" size="small" :min="32" :max="4096" style="width: 90px" />
          </Row>
          <Row :label="t('qrcode.quietZone')">
            <NInputNumber v-model:value="wo.quietZone" size="small" :min="0" :max="20" style="width: 90px" />
          </Row>
          <Row v-if="wo.format === 'QR_CODE'" :label="t('qrcode.ecLevel')" :hint="t('qrcode.ecHint')">
            <Segmented v-model="wo.ecLevel" :options="['L', 'M', 'Q', 'H'].map((v) => ({ label: v, value: v }))" />
          </Row>
        </Group>
        <NButton type="primary" block :disabled="!preview" @click="saveAs">{{ t("qrcode.save") }}</NButton>
      </aside>
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
.workspace {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 320px;
  gap: 16px;
}
.left {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 0;
}
.toolbar {
  display: flex;
  gap: 8px;
}
.viewer {
  flex: 1;
  min-height: 160px;
  display: grid;
  place-items: center;
  padding: 12px;
  background: var(--c-surface-2);
  overflow: hidden;
}
.viewer img {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
}
.viewer.gen img.pixel {
  image-rendering: pixelated;
  max-width: min(100%, 420px);
}
.placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  color: var(--c-text-3);
}
.results {
  max-height: 34%;
  overflow: auto;
  padding: 6px;
}
.muted {
  padding: 10px;
  color: var(--c-text-2);
  font-size: 13px;
}
.code {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border-radius: 10px;
}
.code:hover {
  background: color-mix(in srgb, var(--c-text) 4%, transparent);
}
.fmt {
  flex: none;
  font: 600 11px/18px var(--font-mono);
  padding: 0 6px;
  border-radius: 5px;
  color: var(--c-primary);
  background: var(--c-primary-soft);
}
.txt {
  flex: 1;
  min-width: 0;
  word-break: break-all;
  user-select: text;
}
.options {
  display: flex;
  flex-direction: column;
  gap: 18px;
  overflow: auto;
}
</style>

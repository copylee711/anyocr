<script setup lang="ts">
// 批量图片：识别文件 / 文件夹中的图片并导出。
import { computed, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { NButton, NSelect, NSwitch, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";

import { batchStart, jobCancel, scanFiles } from "../api/app";
import { PARSERS } from "../api/ocr";
import AppIcon from "../components/AppIcon.vue";
import FileList from "../components/FileList.vue";
import OutputOptions from "../components/OutputOptions.vue";
import Group from "../components/ui/Group.vue";
import Row from "../components/ui/Row.vue";
import { useDropFiles } from "../composables/useDropFiles";
import { useSettings } from "../stores/settings";
import { useTasks } from "../stores/tasks";

const { t } = useI18n();
const message = useMessage();
const settings = useSettings();
const tasks = useTasks();
const task = tasks.batch;
const cfg = computed(() => settings.s.batch);
const selected = ref<number | null>(null);
const selectedFile = computed(() => (selected.value === null ? null : task.files[selected.value]));

const IMAGE_EXTS = ["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff", "gif", "jfif"];

async function add(paths: string[]) {
  const files = await scanFiles(paths, cfg.value.recurse, "image");
  if (!files.length) message.warning(t("task.noImages"));
  tasks.addFiles(task, files);
}

const { hovering } = useDropFiles((paths) => !task.running && add(paths));

async function pickFiles() {
  const sel = await open({ multiple: true, filters: [{ name: "Image", extensions: IMAGE_EXTS }] });
  if (Array.isArray(sel)) await add(sel);
}

async function pickFolder() {
  const dir = await open({ directory: true, multiple: false });
  if (typeof dir === "string") await add([dir]);
}

async function start() {
  try {
    const job = await batchStart({
      files: task.files.map((f) => f.path),
      parser: cfg.value.parser,
      output: { ...cfg.value.output, formats: [...cfg.value.output.formats] },
    });
    tasks.begin(task, job);
  } catch (e) {
    message.error(String(e));
  }
}

async function stop() {
  if (task.jobId !== null) await jobCancel(task.jobId);
}

function remove(i: number) {
  task.files.splice(i, 1);
  selected.value = null;
}

const parserOptions = computed(() => PARSERS.map((p) => ({ label: t(`parser.${p}`), value: p })));
</script>

<template>
  <section class="page">
    <header class="page-head row">
      <div>
        <h1>{{ t("batchOcr.title") }}</h1>
        <p>{{ t("batchOcr.desc") }}</p>
      </div>
      <div class="actions">
        <NButton v-if="task.files.length && !task.running" quaternary @click="task.files.splice(0)">
          {{ t("task.clear") }}
        </NButton>
        <NButton :disabled="task.running" @click="pickFolder">{{ t("task.addFolder") }}</NButton>
        <NButton :disabled="task.running" @click="pickFiles">
          <template #icon><AppIcon name="open" :size="16" /></template>
          {{ t("task.addFiles") }}
        </NButton>
        <NButton v-if="task.running" type="error" secondary @click="stop">{{ t("task.stop") }}</NButton>
        <NButton v-else type="primary" :disabled="!task.files.length" @click="start">{{ t("task.start") }}</NButton>
      </div>
    </header>

    <div class="workspace">
      <div class="left">
        <FileList
          :task="task"
          kind="image"
          :hovering="hovering"
          :selected="selected"
          @add="pickFiles"
          @remove="remove"
          @select="(i) => (selected = i)"
        />
        <div v-if="selectedFile?.text !== undefined" class="card preview">
          <div class="preview-head">{{ selectedFile?.name }}</div>
          <pre>{{ selectedFile?.text }}</pre>
        </div>
      </div>

      <aside class="options">
        <Group :title="t('batchDoc.recognition')">
          <Row :label="t('screenshot.parser')">
            <NSelect
              v-model:value="cfg.parser"
              :options="parserOptions"
              size="small"
              :disabled="task.running"
              style="width: 168px"
            />
          </Row>
          <Row :label="t('batchOcr.recurse')">
            <NSwitch v-model:value="cfg.recurse" :disabled="task.running" />
          </Row>
        </Group>
        <OutputOptions
          v-model="cfg.output"
          :title="t('output.title')"
          :formats-title="t('output.export')"
          :disabled="task.running"
        />
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
.actions {
  display: flex;
  gap: 8px;
}
.workspace {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 340px;
  gap: 16px;
}
.left {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 0;
}
.left > :first-child {
  flex: 1;
}
.preview {
  height: 38%;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.preview-head {
  padding: 9px 16px;
  font-size: 12.5px;
  color: var(--c-text-2);
  border-bottom: 0.5px solid var(--c-border);
}
.preview pre {
  margin: 0;
  padding: 12px 16px;
  overflow: auto;
  flex: 1;
  white-space: pre-wrap;
  font-family: var(--font-ui);
  line-height: 1.7;
  user-select: text;
}
.options {
  display: flex;
  flex-direction: column;
  gap: 18px;
  overflow: auto;
  padding-bottom: 4px;
}
</style>

<script setup lang="ts">
// 文档 / PDF：为扫描件添加可搜索文本层，并可同时导出文本。
import { computed } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { NButton, NInput, NSelect, NSwitch, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";

import { docStart, jobCancel, scanFiles } from "../api/app";
import { mineruStart } from "../api/online";
import { PARSERS } from "../api/ocr";
import AppIcon from "../components/AppIcon.vue";
import FileList from "../components/FileList.vue";
import OutputOptions from "../components/OutputOptions.vue";
import Group from "../components/ui/Group.vue";
import Row from "../components/ui/Row.vue";
import Segmented from "../components/ui/Segmented.vue";
import { useDropFiles } from "../composables/useDropFiles";
import { useSettings } from "../stores/settings";
import { useTasks } from "../stores/tasks";

const { t } = useI18n();
const message = useMessage();
const settings = useSettings();
const tasks = useTasks();
const task = tasks.doc;
const cfg = computed(() => settings.s.doc);

async function add(paths: string[]) {
  const files = await scanFiles(paths, true, "doc");
  if (!files.length) message.warning(t("task.noDocs"));
  tasks.addFiles(task, files);
}

const { hovering } = useDropFiles((paths) => !task.running && add(paths));

async function pick() {
  const sel = await open({ multiple: true, filters: [{ name: "PDF", extensions: ["pdf"] }] });
  if (Array.isArray(sel)) await add(sel);
  else if (typeof sel === "string") await add([sel]);
}

const isMineru = computed(() => cfg.value.engineMode === "mineru");

const canStart = computed(
  () =>
    task.files.length > 0 &&
    !task.running &&
    (isMineru.value || cfg.value.writePdf || cfg.value.output.formats.length > 0),
);

async function pickDir() {
  const dir = await open({ directory: true, multiple: false });
  if (typeof dir === "string") {
    cfg.value.output.dir = dir;
    cfg.value.output.dirType = "custom";
  }
}

async function start() {
  try {
    if (isMineru.value) {
      const job = await mineruStart({
        files: task.files.map((f) => f.path),
        pages: cfg.value.pages,
        outDir: cfg.value.output.dirType === "custom" ? cfg.value.output.dir : "",
      });
      tasks.begin(task, job);
      return;
    }
    const job = await docStart({
      files: task.files.map((f) => f.path),
      mode: cfg.value.mode,
      pages: cfg.value.pages,
      dpi: cfg.value.dpi,
      parser: cfg.value.parser,
      writePdf: cfg.value.writePdf,
      pdfSuffix: cfg.value.pdfSuffix,
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

function clear() {
  task.files.splice(0);
}

const parserOptions = computed(() => PARSERS.map((p) => ({ label: t(`parser.${p}`), value: p })));
const dpiOptions = [150, 200, 300, 400].map((d) => ({ label: `${d} DPI`, value: d }));
</script>

<template>
  <section class="page">
    <header class="page-head row">
      <div>
        <h1>{{ t("batchDoc.title") }}</h1>
        <p>{{ t("batchDoc.desc") }}</p>
      </div>
      <div class="actions">
        <NButton v-if="task.files.length && !task.running" quaternary @click="clear">{{ t("task.clear") }}</NButton>
        <NButton :disabled="task.running" @click="pick">
          <template #icon><AppIcon name="open" :size="16" /></template>
          {{ t("task.addFiles") }}
        </NButton>
        <NButton v-if="task.running" type="error" secondary @click="stop">{{ t("task.stop") }}</NButton>
        <NButton v-else type="primary" :disabled="!canStart" @click="start">{{ t("task.start") }}</NButton>
      </div>
    </header>

    <div class="workspace">
      <FileList :task="task" kind="doc" :hovering="hovering" @add="pick" @remove="(i) => task.files.splice(i, 1)" />

      <aside class="options">
        <Group :footer="t(`batchDoc.engineModeHint.${cfg.engineMode}`)">
          <Row :label="t('batchDoc.engineMode')" stacked>
            <Segmented
              v-model="cfg.engineMode"
              :disabled="task.running"
              :options="[
                { label: t('batchDoc.engineModes.pages'), value: 'pages' },
                { label: t('batchDoc.engineModes.mineru'), value: 'mineru' },
              ]"
            />
          </Row>
        </Group>

        <template v-if="isMineru">
          <Group>
            <Row :label="t('batchDoc.pages')">
              <NInput
                v-model:value="cfg.pages"
                size="small"
                :placeholder="t('batchDoc.allPages')"
                :disabled="task.running"
                style="width: 140px"
              />
            </Row>
            <Row :label="t('output.location')">
              <Segmented
                v-model="cfg.output.dirType"
                :disabled="task.running"
                :options="[
                  { label: t('output.source'), value: 'source' },
                  { label: t('output.custom'), value: 'custom' },
                ]"
              />
            </Row>
            <Row v-if="cfg.output.dirType === 'custom'" :label="t('output.folder')" :hint="cfg.output.dir || '—'">
              <NButton size="small" :disabled="task.running" @click="pickDir">{{ t("output.browse") }}</NButton>
            </Row>
          </Group>
        </template>

        <template v-else>
          <Group :title="t('batchDoc.layer')" :footer="t(`batchDoc.modeHint.${cfg.mode}`)">
            <Row :label="t('batchDoc.writePdf')" :hint="t('batchDoc.writePdfHint')">
              <NSwitch v-model:value="cfg.writePdf" :disabled="task.running" />
            </Row>
            <Row :label="t('batchDoc.mode')" stacked>
              <Segmented
                v-model="cfg.mode"
                :disabled="task.running"
                :options="[
                  { label: t('batchDoc.modes.skip-text'), value: 'skip-text' },
                  { label: t('batchDoc.modes.force'), value: 'force' },
                  { label: t('batchDoc.modes.redo'), value: 'redo' },
                ]"
              />
            </Row>
          </Group>
          <Group :title="t('batchDoc.recognition')">
            <Row :label="t('batchDoc.pages')">
              <NInput
                v-model:value="cfg.pages"
                size="small"
                :placeholder="t('batchDoc.allPages')"
                :disabled="task.running"
                style="width: 140px"
              />
            </Row>
            <Row :label="t('batchDoc.dpi')">
              <NSelect
                v-model:value="cfg.dpi"
                :options="dpiOptions"
                size="small"
                :disabled="task.running"
                style="width: 120px"
              />
            </Row>
            <Row :label="t('screenshot.parser')">
              <NSelect
                v-model:value="cfg.parser"
                :options="parserOptions"
                size="small"
                :disabled="task.running"
                style="width: 168px"
              />
            </Row>
            <Row :label="t('batchDoc.suffix')" :hint="t('batchDoc.suffixHint')">
              <NInput
                v-model:value="cfg.pdfSuffix"
                size="small"
                :disabled="task.running || !cfg.writePdf"
                style="width: 120px"
              />
            </Row>
          </Group>
          <OutputOptions
            v-model="cfg.output"
            :title="t('output.title')"
            :formats-title="t('batchDoc.alsoExport')"
            :disabled="task.running"
          />
        </template>
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
.options {
  display: flex;
  flex-direction: column;
  gap: 18px;
  overflow: auto;
  padding-bottom: 4px;
}
</style>

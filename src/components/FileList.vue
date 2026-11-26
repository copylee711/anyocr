<script setup lang="ts">
// 任务文件列表：拖放区、每个文件的状态与进度、打开输出。
import { computed } from "vue";
import { revealItemInDir, openPath } from "@tauri-apps/plugin-opener";
import { NButton, NProgress, NTooltip } from "naive-ui";
import { useI18n } from "vue-i18n";

import type { FileItem, TaskState } from "../stores/tasks";
import AppIcon from "./AppIcon.vue";

const props = defineProps<{ task: TaskState; kind: "doc" | "image"; hovering: boolean; selected?: number | null }>();
const emit = defineEmits<{ add: []; remove: [index: number]; select: [index: number] }>();
const { t } = useI18n();

const summary = computed(() => {
  const f = props.task.files;
  const done = f.filter((x) => x.status === "done").length;
  const failed = f.filter((x) => x.status === "failed").length;
  return { total: f.length, done, failed };
});

function percent(f: FileItem) {
  if (f.status === "done") return 100;
  if (props.kind === "doc" && f.total) return Math.round((f.done / f.total) * 100);
  return f.status === "running" ? 50 : 0;
}

function detail(f: FileItem) {
  if (f.status === "failed") return f.error ?? t("task.failed");
  if (props.kind === "doc") {
    if (f.report) {
      const r = f.report;
      const ocr = r.pages.filter((p) => p.status === "done").length;
      const skip = r.pages.filter((p) => p.status === "skipped").length;
      return t("task.docDone", { ocr, skip, s: (r.elapsed_ms / 1000).toFixed(1) });
    }
    if (f.status === "done" && f.elapsedMs !== undefined)
      return t("task.mdDone", { s: (f.elapsedMs / 1000).toFixed(1) });
    // 在线解析（MinerU）：显示阶段与页数
    if (f.stage && f.status === "running") {
      const stage = t(`batchDoc.stages.${f.stage}`, f.stage);
      return f.total ? `${stage} ${t("task.pages", { done: f.done, total: f.total })}` : stage;
    }
    if (f.total !== undefined) return t("task.pages", { done: f.done, total: f.total });
  } else if (f.status === "done") {
    return t("task.imageDone", { n: f.blocks ?? 0, ms: f.elapsedMs ?? 0 });
  }
  return t(`task.status.${f.status}`);
}

async function openMain(f: FileItem) {
  const p = f.outputs.find((o) => o.toLowerCase().endsWith(".pdf")) ?? f.outputs[0];
  if (p) await openPath(p);
}
</script>

<template>
  <div class="card files" :class="{ hovering }">
    <div v-if="task.files.length === 0" class="empty" @click="emit('add')">
      <AppIcon :name="kind === 'doc' ? 'document' : 'images'" :size="34" />
      <strong>{{ kind === "doc" ? t("task.dropDocs") : t("task.dropImages") }}</strong>
      <span>{{ t("task.dropHint") }}</span>
    </div>
    <template v-else>
      <div class="head">
        <span>{{ t("task.summary", summary) }}</span>
      </div>
      <ul class="list">
        <li v-for="(f, i) in task.files" :key="f.path" :class="{ sel: selected === i }" @click="emit('select', i)">
          <span class="dot" :class="f.status" />
          <div class="main">
            <NTooltip placement="top-start" :delay="600">
              <template #trigger>
                <span class="name">{{ f.name }}</span>
              </template>
              {{ f.path }}
            </NTooltip>
            <small :class="{ err: f.status === 'failed' }">{{ detail(f) }}</small>
            <NProgress
              v-if="f.status === 'running' && kind === 'doc'"
              type="line"
              :percentage="percent(f)"
              :show-indicator="false"
              :height="4"
            />
          </div>
          <div class="ops">
            <NButton v-if="f.outputs.length" size="tiny" quaternary @click.stop="openMain(f)">
              {{ t("task.open") }}
            </NButton>
            <NButton v-if="f.outputs.length" size="tiny" quaternary @click.stop="revealItemInDir(f.outputs[0])">
              {{ t("task.reveal") }}
            </NButton>
            <NButton v-if="!task.running" size="tiny" quaternary circle @click.stop="emit('remove', i)"> ✕ </NButton>
          </div>
        </li>
      </ul>
    </template>
    <div v-if="hovering" class="drop-overlay">
      <AppIcon name="open" :size="28" />
      <span>{{ t("task.dropRelease") }}</span>
    </div>
  </div>
</template>

<style scoped>
.files {
  position: relative;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}
.empty {
  flex: 1;
  margin: 12px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--c-text-2);
  border: 1.5px dashed var(--c-border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition:
    border-color 0.15s,
    color 0.15s;
}
.empty:hover {
  border-color: var(--c-primary);
  color: var(--c-primary);
}
.empty strong {
  color: var(--c-text);
  font-size: 15px;
}
.empty span {
  font-size: 12.5px;
}
.head {
  padding: 10px 16px;
  font-size: 12.5px;
  color: var(--c-text-2);
  border-bottom: 0.5px solid var(--c-border);
}
.list {
  list-style: none;
  margin: 0;
  padding: 6px;
  overflow: auto;
  flex: 1;
}
li {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 10px;
  border-radius: 10px;
  cursor: default;
}
li:hover {
  background: color-mix(in srgb, var(--c-text) 4%, transparent);
}
li.sel {
  background: var(--c-primary-soft);
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex: none;
  background: var(--c-text-3);
}
.dot.running {
  background: var(--c-primary);
  box-shadow: 0 0 0 4px var(--c-primary-soft);
}
.dot.done {
  background: #34c759;
}
.dot.failed {
  background: #ff3b30;
}
.main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
small {
  font-size: 12px;
  color: var(--c-text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
small.err {
  color: #ff3b30;
}
.ops {
  display: flex;
  gap: 2px;
  opacity: 0.85;
}
.drop-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--c-primary);
  font-weight: 600;
  background: color-mix(in srgb, var(--c-surface) 82%, transparent);
  border: 2px dashed var(--c-primary);
  border-radius: var(--radius);
  backdrop-filter: blur(6px);
}
</style>

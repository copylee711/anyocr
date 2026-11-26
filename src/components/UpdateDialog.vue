<script setup lang="ts">
// 新版本提醒：版本号、更新说明，一键更新 / 忽略此版本 / 稍后。
import { computed } from "vue";
import { NButton, NModal, NProgress } from "naive-ui";
import { useI18n } from "vue-i18n";

import { useUpdater } from "../stores/updater";

const { t } = useI18n();
const up = useUpdater();

const busy = computed(() => up.status === "downloading" || up.status === "ready");
const percent = computed(() => (up.total ? Math.min(100, Math.round((up.downloaded / up.total) * 100)) : 0));
const mb = (b: number) => (b / 1024 / 1024).toFixed(1);
const date = computed(() => (up.update?.date ? up.update.date.slice(0, 10) : ""));
</script>

<template>
  <NModal
    :show="up.dialog"
    :mask-closable="!busy"
    :close-on-esc="!busy"
    preset="card"
    style="width: 460px"
    :title="t('update.title', { v: up.update?.version ?? '' })"
    :closable="!busy"
    @update:show="(v: boolean) => (up.dialog = v)"
  >
    <div class="meta">
      {{ t("update.current", { v: up.update?.currentVersion ?? "" }) }}
      <template v-if="date"> · {{ t("update.date", { d: date }) }}</template>
    </div>
    <pre v-if="up.update?.body" class="notes">{{ up.update.body }}</pre>
    <div v-if="busy" class="progress">
      <NProgress type="line" :percentage="percent" :show-indicator="false" :processing="!up.total" />
      <small>
        {{
          up.status === "ready"
            ? t("update.restarting")
            : up.total
              ? t("update.downloading", { a: mb(up.downloaded), b: mb(up.total) })
              : t("update.downloadingUnknown", { a: mb(up.downloaded) })
        }}
      </small>
    </div>
    <div v-else-if="up.status === 'error'" class="error">{{ up.error }}</div>
    <template #footer>
      <div class="actions">
        <NButton quaternary :disabled="busy" @click="up.ignore()">{{ t("update.ignore") }}</NButton>
        <span class="grow" />
        <NButton :disabled="busy" @click="up.dialog = false">{{ t("update.later") }}</NButton>
        <NButton type="primary" :loading="busy" @click="up.install()">{{ t("update.install") }}</NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.meta {
  font-size: 13px;
  color: var(--c-text-2);
}
.notes {
  margin: 12px 0 0;
  max-height: 260px;
  overflow: auto;
  padding: 10px 12px;
  border-radius: 10px;
  background: var(--c-surface-2);
  font: 13px/1.6 var(--font-ui, inherit);
  white-space: pre-wrap;
  user-select: text;
}
.progress {
  margin-top: 14px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.progress small {
  color: var(--c-text-2);
}
.error {
  margin-top: 12px;
  font-size: 12px;
  color: #d70015;
  word-break: break-all;
}
.actions {
  display: flex;
  gap: 8px;
}
.grow {
  flex: 1;
}
</style>

<script setup lang="ts">
// 设置：识别引擎、外观、存储。在线引擎与视觉大模型在后续里程碑加入。
import { onMounted, ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { NButton, NSelect, NSwitch } from "naive-ui";
import { useI18n } from "vue-i18n";

import { appInfo, type AppInfo } from "../api/app";
import Group from "../components/ui/Group.vue";
import Row from "../components/ui/Row.vue";
import Segmented from "../components/ui/Segmented.vue";
import { useSettings } from "../stores/settings";

const { t } = useI18n();
const settings = useSettings();
const info = ref<AppInfo | null>(null);

onMounted(async () => {
  if (isTauri()) info.value = await appInfo();
});

const models = ["pp-ocrv6-tiny", "pp-ocrv6-small", "pp-ocrv6-medium", "pp-ocrv5-mobile", "pp-ocrv5-server"];
</script>

<template>
  <section class="page">
    <header class="page-head">
      <h1>{{ t("settings.title") }}</h1>
      <p>{{ t("settings.desc") }}</p>
    </header>

    <div class="groups">
      <Group :title="t('settings.engine')" :footer="t('settings.engineHint')">
        <Row :label="t('settings.model')">
          <NSelect
            v-model:value="settings.s.engine.model"
            size="small"
            style="width: 220px"
            :options="models.map((m) => ({ label: t(`model.${m}`), value: m }))"
          />
        </Row>
        <Row :label="t('settings.device')" :hint="t('settings.deviceHint')">
          <Segmented
            v-model="settings.s.engine.device"
            :options="[
              { label: t('settings.devices.auto'), value: 'auto' },
              { label: 'CPU', value: 'cpu' },
              { label: 'DirectML', value: 'directml' },
              { label: 'CUDA', value: 'cuda' },
            ]"
          />
        </Row>
        <Row :label="t('settings.workers')" :hint="t('settings.workersHint')">
          <NSelect
            v-model:value="settings.s.engine.workers"
            size="small"
            style="width: 120px"
            :options="
              [0, 1, 2, 3, 4].map((n) => ({ label: n === 0 ? t('settings.devices.auto') : String(n), value: n }))
            "
          />
        </Row>
        <Row :label="t('settings.textline')" :hint="t('settings.textlineHint')">
          <NSwitch v-model:value="settings.s.engine.textlineOrientation" />
        </Row>
      </Group>

      <Group :title="t('settings.appearance')">
        <Row :label="t('settings.theme')">
          <Segmented
            v-model="settings.s.ui.theme"
            :options="[
              { label: t('settings.themes.system'), value: 'system' },
              { label: t('settings.themes.light'), value: 'light' },
              { label: t('settings.themes.dark'), value: 'dark' },
            ]"
          />
        </Row>
        <Row :label="t('settings.language')">
          <Segmented
            v-model="settings.s.ui.language"
            :options="[
              { label: t('settings.themes.system'), value: 'auto' },
              { label: '简体中文', value: 'zh-CN' },
              { label: 'English', value: 'en' },
            ]"
          />
        </Row>
      </Group>

      <Group :title="t('settings.storage')" :footer="t('settings.storageHint')">
        <Row :label="t('settings.modelsDir')" :hint="info?.modelsDir">
          <NButton size="small" :disabled="!info" @click="info && openPath(info.modelsDir)">
            {{ t("task.reveal") }}
          </NButton>
        </Row>
        <Row :label="t('settings.version')">
          <span class="muted">{{ info?.version ?? "—" }}</span>
        </Row>
      </Group>

      <Group :title="t('settings.online')" :footer="t('settings.onlineHint')">
        <Row :label="t('settings.onlineEngines')" :hint="t('common.milestone', { m: 'M5 / M6' })">
          <span class="muted">{{ t("common.comingSoon") }}</span>
        </Row>
      </Group>
    </div>
  </section>
</template>

<style scoped>
.groups {
  display: flex;
  flex-direction: column;
  gap: 22px;
  max-width: 720px;
  padding-bottom: 12px;
}
.muted {
  color: var(--c-text-2);
  font-size: 13px;
}
</style>

<script setup lang="ts">
// 设置：识别引擎、在线引擎链与密钥、快捷键、外观、存储。
import { computed, onMounted, ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { NButton, NSelect, NSwitch } from "naive-ui";
import { useI18n } from "vue-i18n";

import { appInfo, type AppInfo } from "../api/app";
import Group from "../components/ui/Group.vue";
import Row from "../components/ui/Row.vue";
import HotkeyInput from "../components/ui/HotkeyInput.vue";
import OnlineEngines from "../components/OnlineEngines.vue";
import VlmSettings from "../components/VlmSettings.vue";
import Segmented from "../components/ui/Segmented.vue";
import { useSettings } from "../stores/settings";
import { useUpdater } from "../stores/updater";

const { t } = useI18n();
const settings = useSettings();
const info = ref<AppInfo | null>(null);
const updater = useUpdater();

const updateStatus = computed(() => {
  switch (updater.status) {
    case "checking":
      return t("update.checking");
    case "latest":
      return t("update.latest");
    case "available":
      return t("update.available", { v: updater.update?.version ?? "" });
    case "error":
      return t("update.failed", { e: updater.error ?? "" });
    default:
      return undefined;
  }
});

/** 手动检查：有新版本时弹窗（即使之前忽略过）。 */
async function manualCheck() {
  const found = await updater.checkNow(false);
  if (found) updater.dialog = true;
}

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

      <OnlineEngines />

      <VlmSettings />

      <Group :title="t('settings.hotkeys')" :footer="t('settings.hotkeysHint')">
        <Row
          v-for="k in ['screenshot', 'paste', 'reScreenshot', 'qrScreenshot'] as const"
          :key="k"
          :label="t(`hotkey.names.${k}`)"
        >
          <HotkeyInput v-model="settings.s.hotkeys[k]" />
        </Row>
      </Group>

      <Group :title="t('settings.screenshot')">
        <Row :label="t('settings.copyResult')">
          <NSwitch v-model:value="settings.s.screenshot.copy" />
        </Row>
        <Row :label="t('settings.popMain')">
          <NSwitch v-model:value="settings.s.screenshot.popMainWindow" />
        </Row>
        <Row :label="t('settings.hideWindow')" :hint="t('settings.hideWindowHint')">
          <NSwitch v-model:value="settings.s.screenshot.hideWindow" />
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
      </Group>

      <Group :title="t('update.group')" :footer="t('update.hint')">
        <Row :label="t('settings.version')" :hint="updateStatus">
          <span class="muted">{{ info?.version ?? "—" }}</span>
          <NButton
            size="small"
            :loading="updater.status === 'checking'"
            :disabled="updater.status === 'downloading'"
            @click="manualCheck"
          >
            {{ t("update.check") }}
          </NButton>
        </Row>
        <Row :label="t('update.auto')">
          <NSwitch v-model:value="settings.s.update.autoCheck" />
        </Row>
        <Row
          v-if="settings.s.update.ignoredVersion"
          :label="t('update.ignored', { v: settings.s.update.ignoredVersion })"
        >
          <NButton size="small" quaternary @click="settings.s.update.ignoredVersion = ''">{{
            t("update.unignore")
          }}</NButton>
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

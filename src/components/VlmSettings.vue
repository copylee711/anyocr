<script setup lang="ts">
// 设置页：视觉大模型服务（地址、模型、密钥、测试）与并行复核选项。
import { computed, onMounted, ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { NButton, NInput, NInputNumber, NSelect, NSwitch, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";

import { secretSet, secretStatus } from "../api/online";
import { type VlmPreset, vlmPresets, vlmSecretId, vlmTest, type VlmTestResult } from "../api/vlm";
import { useSettings } from "../stores/settings";
import AppIcon from "./AppIcon.vue";
import Group from "./ui/Group.vue";
import Row from "./ui/Row.vue";

const { t } = useI18n();
const message = useMessage();
const settings = useSettings();
const v = computed(() => settings.s.vlm);

const presets = ref<VlmPreset[]>([]);
const preset = computed(() => presets.value.find((p) => p.id === v.value.provider));
const savedTail = ref<string | null>(null);
const draft = ref("");
const testing = ref(false);
const result = ref<VlmTestResult | null>(null);

async function refreshKey() {
  savedTail.value = null;
  if (!isTauri()) return;
  const s = await secretStatus(vlmSecretId(v.value.provider));
  savedTail.value = s.api_key ?? null;
}

onMounted(async () => {
  if (!isTauri()) return;
  presets.value = await vlmPresets();
  await refreshKey();
});

/** 用户切换服务：模型与地址回到该服务的默认值（只响应界面操作，避免加载设置时被清空）。 */
function changeProvider(id: string) {
  v.value.provider = id;
  v.value.model = "";
  v.value.baseUrl = "";
  draft.value = "";
  result.value = null;
  refreshKey();
}

async function saveKey() {
  if (!draft.value.trim()) return;
  try {
    await secretSet(vlmSecretId(v.value.provider), "api_key", draft.value.trim());
    draft.value = "";
    await refreshKey();
    message.success(t("online.saved"));
  } catch (e) {
    message.error(String(e));
  }
}

async function clearKey() {
  await secretSet(vlmSecretId(v.value.provider), "api_key", "");
  await refreshKey();
}

const ready = computed(() => !!preset.value && (!preset.value.needsKey || !!savedTail.value));

async function test() {
  testing.value = true;
  try {
    result.value = await vlmTest();
  } catch (e) {
    result.value = { ok: false, elapsedMs: 0, fixed: 0, model: "", error: String(e) };
  } finally {
    testing.value = false;
  }
}

const modelOptions = computed(() => (preset.value?.models ?? []).map((m) => ({ label: m, value: m })));
</script>

<template>
  <Group :title="t('vlm.title')" :footer="preset?.note || t('vlm.hint')">
    <Row :label="t('vlm.provider')">
      <span v-if="preset && !preset.verified" class="tag">{{ t("online.unverified") }}</span>
      <NSelect
        :value="v.provider"
        size="small"
        style="width: 220px"
        :options="presets.map((p) => ({ label: p.name, value: p.id }))"
        @update:value="changeProvider"
      />
    </Row>
    <Row v-if="preset?.needsKey" :label="'API Key'">
      <NInput
        v-model:value="draft"
        type="password"
        show-password-on="click"
        size="small"
        style="width: 260px"
        :placeholder="savedTail ? t('online.savedTail', { tail: savedTail }) : t('online.enter')"
        @keyup.enter="saveKey"
        @blur="saveKey"
      />
      <NButton v-if="savedTail" size="small" quaternary @click="clearKey">{{ t("online.clear") }}</NButton>
    </Row>
    <Row :label="t('vlm.model')" :hint="t('vlm.modelHint')">
      <NSelect
        :value="v.model || null"
        size="small"
        style="width: 260px"
        filterable
        tag
        clearable
        :placeholder="preset?.model || t('vlm.modelRequired')"
        :options="modelOptions"
        @update:value="(m: string | null) => (v.model = m ?? '')"
      />
    </Row>
    <Row :label="t('vlm.baseUrl')" :hint="t('vlm.baseUrlHint')">
      <NInput
        v-model:value="v.baseUrl"
        size="small"
        style="width: 260px"
        :placeholder="preset?.baseUrl || 'https://…/v1'"
      />
    </Row>
    <div class="actions">
      <NButton v-if="preset?.keyUrl" size="small" secondary @click="openUrl(preset.keyUrl)">
        {{ preset.needsKey ? t("online.getKey") : t("vlm.docs") }}
        <template #icon><AppIcon name="external" :size="15" /></template>
      </NButton>
      <NButton size="small" type="primary" secondary :loading="testing" :disabled="!ready" @click="test">
        {{ t("online.test") }}
      </NButton>
      <span v-if="result" class="result" :class="{ ok: result.ok && result.fixed > 0 }">
        <template v-if="result.ok">{{
          t("vlm.testOk", { fixed: result.fixed, ms: result.elapsedMs, model: result.model })
        }}</template>
        <template v-else>{{ result.error }}</template>
      </span>
    </div>
  </Group>

  <Group :title="t('vlm.review')" :footer="t('vlm.reviewHint')">
    <Row :label="t('vlm.reviewDocs')" :hint="t('vlm.reviewDocsHint')">
      <NSwitch v-model:value="v.review.enabled" :disabled="!ready && !v.review.enabled" />
    </Row>
    <Row :label="t('vlm.reviewShot')" :hint="t('vlm.reviewShotHint')">
      <NSwitch v-model:value="v.review.screenshot" :disabled="!ready && !v.review.screenshot" />
    </Row>
    <Row :label="t('vlm.grace')" :hint="t('vlm.graceHint')">
      <NInputNumber v-model:value="v.review.grace" size="small" :min="0" :max="600" style="width: 110px">
        <template #suffix>s</template>
      </NInputNumber>
    </Row>
    <Row :label="t('vlm.concurrency')">
      <NInputNumber v-model:value="v.review.concurrency" size="small" :min="1" :max="16" style="width: 110px" />
    </Row>
    <Row :label="t('vlm.lowConf')" :hint="t('vlm.lowConfHint')">
      <NInputNumber v-model:value="v.review.lowConf" size="small" :min="0" :max="1" :step="0.05" style="width: 110px" />
    </Row>
  </Group>
</template>

<style scoped>
.tag {
  font-size: 11px;
  padding: 1px 7px;
  border-radius: 999px;
  color: var(--c-text-2);
  background: color-mix(in srgb, var(--c-text) 6%, transparent);
}
.actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 8px 14px 12px;
}
.result {
  font-size: 12px;
  color: #d70015;
  word-break: break-all;
}
.result.ok {
  color: #248a3d;
}
</style>

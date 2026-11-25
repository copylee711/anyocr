<script setup lang="ts">
// 输出设置：输出位置、导出格式、文件名模板、忽略空白。
import { open } from "@tauri-apps/plugin-dialog";
import { NButton, NCheckbox, NCheckboxGroup, NInput, NSwitch } from "naive-ui";
import { useI18n } from "vue-i18n";

import { OUTPUT_FORMATS, type OutputArgs } from "../api/app";
import Group from "./ui/Group.vue";
import Row from "./ui/Row.vue";
import Segmented from "./ui/Segmented.vue";

defineProps<{ title: string; formatsTitle: string; disabled?: boolean }>();
const out = defineModel<OutputArgs>({ required: true });
const { t } = useI18n();

async function chooseDir() {
  const dir = await open({ directory: true, multiple: false });
  if (typeof dir === "string") {
    out.value.dir = dir;
    out.value.dirType = "custom";
  }
}
</script>

<template>
  <Group :title="title">
    <Row :label="t('output.location')">
      <Segmented
        v-model="out.dirType"
        :disabled="disabled"
        :options="[
          { label: t('output.source'), value: 'source' },
          { label: t('output.custom'), value: 'custom' },
        ]"
      />
    </Row>
    <Row v-if="out.dirType === 'custom'" :label="t('output.folder')" stacked>
      <div class="dir">
        <NInput v-model:value="out.dir" size="small" :placeholder="t('output.choose')" :disabled="disabled" />
        <NButton size="small" :disabled="disabled" @click="chooseDir">{{ t("output.browse") }}</NButton>
      </div>
    </Row>
  </Group>
  <Group :title="formatsTitle" :footer="t('output.templateHint')">
    <Row :label="t('output.formats')" stacked>
      <NCheckboxGroup v-model:value="out.formats" :disabled="disabled">
        <div class="formats">
          <NCheckbox v-for="f in OUTPUT_FORMATS" :key="f" :value="f" :label="t(`format.${f}`)" />
        </div>
      </NCheckboxGroup>
    </Row>
    <Row :label="t('output.fileName')" stacked>
      <NInput v-model:value="out.fileNameTemplate" size="small" :disabled="disabled" />
    </Row>
    <Row :label="t('output.ignoreBlank')">
      <NSwitch v-model:value="out.ignoreBlank" :disabled="disabled" />
    </Row>
  </Group>
</template>

<style scoped>
.dir {
  display: flex;
  gap: 8px;
  width: 100%;
}
.formats {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 6px 12px;
}
</style>

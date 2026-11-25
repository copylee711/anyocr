<script setup lang="ts">
// 自定义标题栏：整条可拖动（双击最大化），右侧为窗口控制按钮。
import { onBeforeUnmount, onMounted, ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const maximized = ref(false);
const inTauri = isTauri();
let unlisten: (() => void) | undefined;

async function refresh() {
  if (inTauri) maximized.value = await getCurrentWindow().isMaximized();
}

onMounted(async () => {
  if (!inTauri) return;
  await refresh();
  unlisten = await getCurrentWindow().onResized(refresh);
});
onBeforeUnmount(() => unlisten?.());

const minimize = () => inTauri && getCurrentWindow().minimize();
const toggleMaximize = () => inTauri && getCurrentWindow().toggleMaximize();
const close = () => inTauri && getCurrentWindow().close();
</script>

<template>
  <header class="titlebar" data-tauri-drag-region>
    <slot />
    <div class="controls">
      <button class="ctl" :title="t('window.minimize')" @click="minimize">
        <svg viewBox="0 0 12 12"><path d="M2 6h8" /></svg>
      </button>
      <button class="ctl" :title="maximized ? t('window.restore') : t('window.maximize')" @click="toggleMaximize">
        <svg v-if="maximized" viewBox="0 0 12 12"><path d="M3.5 4.5h5v5h-5zM4.5 4.5V2.5h5v5h-1" /></svg>
        <svg v-else viewBox="0 0 12 12"><rect x="2.5" y="2.5" width="7" height="7" rx="1.2" /></svg>
      </button>
      <button class="ctl close" :title="t('window.close')" @click="close">
        <svg viewBox="0 0 12 12"><path d="M3 3l6 6M9 3l-6 6" /></svg>
      </button>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  height: 40px;
  flex: none;
  display: flex;
  align-items: center;
  padding-left: 16px;
  -webkit-app-region: drag;
}
.controls {
  margin-left: auto;
  display: flex;
  height: 100%;
  -webkit-app-region: no-drag;
}
.ctl {
  width: 46px;
  height: 100%;
  display: grid;
  place-items: center;
  border: 0;
  background: transparent;
  color: var(--c-text-2);
  cursor: default;
  transition:
    background 0.12s,
    color 0.12s;
}
.ctl svg {
  width: 11px;
  height: 11px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.2;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.ctl:hover {
  background: color-mix(in srgb, var(--c-text) 7%, transparent);
  color: var(--c-text);
}
.ctl.close:hover {
  background: #e81123;
  color: #fff;
}
</style>

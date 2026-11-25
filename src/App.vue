<script setup lang="ts">
import { computed, watchEffect } from "vue";
import { darkTheme, NConfigProvider, NMessageProvider, useOsTheme } from "naive-ui";
import { useI18n } from "vue-i18n";

import AppIcon from "./components/AppIcon.vue";
import ContextMenu from "./components/ContextMenu.vue";
import GlobalListeners from "./components/GlobalListeners.vue";
import TitleBar from "./components/TitleBar.vue";
import { bottomNavItems, navItems } from "./router";
import { useSettings } from "./stores/settings";
import { darkOverrides, lightOverrides } from "./theme";

const { t, locale } = useI18n();
const osTheme = useOsTheme();
const settings = useSettings();
// 截图遮罩窗口只渲染遮罩页，不加载主界面
const isOverlay = window.location.hash.startsWith("#/overlay");

const isDark = computed(() => {
  const pref = settings.s.ui.theme;
  return pref === "dark" || (pref === "system" && osTheme.value === "dark");
});

// 主题与语言跟随设置
watchEffect(() => {
  document.documentElement.dataset.theme = isDark.value ? "dark" : "light";
  const lang = settings.s.ui.language;
  locale.value = lang === "auto" ? (navigator.language.startsWith("zh") ? "zh-CN" : "en") : lang;
});
</script>

<template>
  <RouterView v-if="isOverlay" />
  <NConfigProvider
    v-else
    style="height: 100%"
    :theme="isDark ? darkTheme : null"
    :theme-overrides="isDark ? darkOverrides : lightOverrides"
  >
    <NMessageProvider>
      <GlobalListeners />
      <div class="shell">
        <!-- iPadOS 风格侧栏：毛玻璃材质、图标+文字、胶囊形选中态 -->
        <aside class="sidebar" data-tauri-drag-region>
          <div class="brand" data-tauri-drag-region>
            <img src="/logo.svg" alt="" class="logo" />
            <div class="brand-text">
              <strong>{{ t("app.name") }}</strong>
              <small>{{ t("app.tagline") }}</small>
            </div>
          </div>
          <nav class="nav">
            <RouterLink v-for="item in navItems" :key="item.path" :to="item.path" class="nav-item">
              <AppIcon :name="item.icon" :size="19" />
              <span>{{ t(item.label) }}</span>
            </RouterLink>
          </nav>
          <nav class="nav nav-bottom">
            <RouterLink v-for="item in bottomNavItems" :key="item.path" :to="item.path" class="nav-item">
              <AppIcon :name="item.icon" :size="19" />
              <span>{{ t(item.label) }}</span>
            </RouterLink>
          </nav>
        </aside>
        <main class="main">
          <TitleBar />
          <div class="content">
            <RouterView v-slot="{ Component }">
              <Transition name="fade" mode="out-in">
                <component :is="Component" />
              </Transition>
            </RouterView>
          </div>
        </main>
        <ContextMenu />
      </div>
    </NMessageProvider>
  </NConfigProvider>
</template>

<style scoped>
.shell {
  display: flex;
  height: 100%;
}

.sidebar {
  width: 224px;
  flex: none;
  display: flex;
  flex-direction: column;
  padding: 22px 12px 14px;
  gap: 14px;
  background: var(--c-sidebar);
  border-right: 0.5px solid var(--c-border);
  backdrop-filter: saturate(180%) blur(24px);
}

.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 2px 6px 8px;
}
.logo {
  width: 36px;
  height: 36px;
  border-radius: 9px;
  box-shadow: 0 4px 14px -6px rgba(0, 0, 0, 0.45);
}
.brand-text {
  display: flex;
  flex-direction: column;
  line-height: 1.2;
  min-width: 0;
}
.brand-text strong {
  font-size: 16px;
  font-weight: 700;
  letter-spacing: -0.02em;
}
.brand-text small {
  font-size: 11px;
  color: var(--c-text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.nav-bottom {
  margin-top: auto;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 11px;
  height: 36px;
  padding: 0 11px;
  border-radius: 10px;
  font-size: 14px;
  color: var(--c-text);
  text-decoration: none;
  transition:
    background 0.15s,
    color 0.15s;
}
.nav-item svg {
  color: var(--c-primary);
}
.nav-item:hover {
  background: color-mix(in srgb, var(--c-text) 6%, transparent);
}
.nav-item.router-link-active {
  background: var(--c-primary);
  color: #fff;
  font-weight: 600;
}
.nav-item.router-link-active svg {
  color: #fff;
}

.main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.content {
  flex: 1;
  min-height: 0;
  /* 切换动画的位移不能撑出外层滚动条，滚动只发生在页面内部 */
  overflow: hidden;
  position: relative;
}

.fade-enter-active,
.fade-leave-active {
  transition:
    opacity 0.16s ease,
    transform 0.16s ease;
}
.fade-enter-from {
  opacity: 0;
  transform: translateY(4px);
}
.fade-leave-to {
  opacity: 0;
}
</style>

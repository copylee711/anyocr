<script setup lang="ts">
// 全局自定义右键菜单：替换 WebView 默认菜单（去掉“检查”“刷新”等浏览器项）。
// 根据右键位置提供编辑类操作（复制 / 剪切 / 粘贴 / 全选）与快捷导航。
import { computed, h, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { type DropdownOption, NDropdown } from "naive-ui";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

import AppIcon from "./AppIcon.vue";

const { t } = useI18n();
const router = useRouter();

const show = ref(false);
const x = ref(0);
const y = ref(0);
const hasSelection = ref(false);
let editable: HTMLInputElement | HTMLTextAreaElement | HTMLElement | null = null;

function isEditable(el: EventTarget | null): el is HTMLElement {
  if (!(el instanceof HTMLElement)) return false;
  if (el instanceof HTMLInputElement) return !el.readOnly && !el.disabled;
  if (el instanceof HTMLTextAreaElement) return !el.readOnly && !el.disabled;
  return el.isContentEditable;
}

const icon = (name: string) => () => h(AppIcon, { name, size: 15 });

const options = computed<DropdownOption[]>(() => {
  const edit: DropdownOption[] = [];
  if (editable) edit.push({ key: "cut", label: t("menu.cut"), disabled: !hasSelection.value, icon: icon("cut") });
  edit.push({ key: "copy", label: t("menu.copy"), disabled: !hasSelection.value, icon: icon("copy") });
  if (editable) edit.push({ key: "paste", label: t("menu.paste"), icon: icon("paste") });
  edit.push({ key: "selectAll", label: t("menu.selectAll"), icon: icon("selectAll") });
  return [
    ...edit,
    { type: "divider", key: "d1" },
    { key: "nav:/screenshot", label: t("nav.screenshot"), icon: icon("scan") },
    { key: "nav:/batch-doc", label: t("nav.batchDoc"), icon: icon("document") },
    { key: "nav:/settings", label: t("nav.settings"), icon: icon("settings") },
  ];
});

function onContextMenu(e: MouseEvent) {
  e.preventDefault();
  editable = isEditable(e.target) ? (e.target as HTMLElement) : null;
  hasSelection.value = (window.getSelection()?.toString().length ?? 0) > 0 || hasInputSelection(editable);
  show.value = false;
  nextTick(() => {
    x.value = e.clientX;
    y.value = e.clientY;
    show.value = true;
  });
}

function hasInputSelection(el: HTMLElement | null) {
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    return (el.selectionEnd ?? 0) > (el.selectionStart ?? 0);
  }
  return false;
}

async function onSelect(key: string) {
  show.value = false;
  if (key.startsWith("nav:")) {
    await router.push(key.slice(4));
    return;
  }
  editable?.focus();
  switch (key) {
    case "copy":
    case "cut": {
      const text =
        editable instanceof HTMLInputElement || editable instanceof HTMLTextAreaElement
          ? editable.value.slice(editable.selectionStart ?? 0, editable.selectionEnd ?? 0)
          : (window.getSelection()?.toString() ?? "");
      await navigator.clipboard.writeText(text);
      if (key === "cut") document.execCommand("delete");
      break;
    }
    case "paste": {
      const text = await navigator.clipboard.readText();
      document.execCommand("insertText", false, text);
      break;
    }
    case "selectAll":
      if (editable instanceof HTMLInputElement || editable instanceof HTMLTextAreaElement) editable.select();
      else document.execCommand("selectAll");
      break;
  }
}

onMounted(() => document.addEventListener("contextmenu", onContextMenu));
onBeforeUnmount(() => document.removeEventListener("contextmenu", onContextMenu));
</script>

<template>
  <NDropdown
    placement="bottom-start"
    trigger="manual"
    size="small"
    :x="x"
    :y="y"
    :show="show"
    :options="options"
    @select="onSelect"
    @clickoutside="show = false"
  />
</template>

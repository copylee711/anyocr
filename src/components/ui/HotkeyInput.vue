<script setup lang="ts">
// 快捷键录制：点击后按下组合键即录入，Esc 取消，Backspace / Delete 清除。
// 输出格式与 Tauri global-shortcut 一致，如 "Ctrl+Shift+A"、"Alt+F4"、"F4"。
import { ref } from "vue";
import { useI18n } from "vue-i18n";

const model = defineModel<string>({ required: true });
const { t } = useI18n();
const recording = ref(false);

function keyName(e: KeyboardEvent): string | null {
  const c = e.code;
  if (/^Key[A-Z]$/.test(c)) return c.slice(3);
  if (/^Digit\d$/.test(c)) return c.slice(5);
  if (/^F\d{1,2}$/.test(c)) return c;
  if (/^Numpad\d$/.test(c)) return `Num${c.slice(6)}`;
  const map: Record<string, string> = {
    Space: "Space",
    Enter: "Enter",
    Tab: "Tab",
    Insert: "Insert",
    Home: "Home",
    End: "End",
    PageUp: "PageUp",
    PageDown: "PageDown",
    ArrowUp: "Up",
    ArrowDown: "Down",
    ArrowLeft: "Left",
    ArrowRight: "Right",
    Backquote: "`",
    Minus: "-",
    Equal: "=",
    BracketLeft: "[",
    BracketRight: "]",
    Semicolon: ";",
    Quote: "'",
    Comma: ",",
    Period: ".",
    Slash: "/",
    Backslash: "\\",
    PrintScreen: "PrintScreen",
  };
  return map[c] ?? null;
}

function onKey(e: KeyboardEvent) {
  if (!recording.value) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") {
    recording.value = false;
    return;
  }
  if (e.key === "Backspace" || e.key === "Delete") {
    model.value = "";
    recording.value = false;
    return;
  }
  const key = keyName(e);
  if (!key) return; // 只按了修饰键，继续等待
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
  // 单独的字母 / 数字键会干扰正常输入，要求带修饰键；功能键可以单独使用
  if (!mods.length && !/^F\d{1,2}$|^PrintScreen$|^Num/.test(key)) return;
  model.value = [...mods, key].join("+");
  recording.value = false;
}
</script>

<template>
  <button
    type="button"
    class="hk"
    :class="{ rec: recording }"
    @click="recording = !recording"
    @keydown="onKey"
    @blur="recording = false"
  >
    <template v-if="recording">{{ t("hotkey.press") }}</template>
    <template v-else-if="model">
      <kbd v-for="k in model.split('+')" :key="k">{{ k }}</kbd>
    </template>
    <span v-else class="none">{{ t("hotkey.none") }}</span>
  </button>
</template>

<style scoped>
.hk {
  min-width: 132px;
  height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 0 10px;
  border: 0.5px solid var(--c-border);
  border-radius: 8px;
  background: var(--c-surface-2);
  color: var(--c-text);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}
.hk.rec {
  border-color: var(--c-primary);
  color: var(--c-primary);
  box-shadow: 0 0 0 3px var(--c-primary-soft);
}
kbd {
  font-family: var(--font-ui);
  font-size: 12px;
  padding: 1px 6px;
  border-radius: 5px;
  background: var(--c-surface);
  border: 0.5px solid var(--c-border);
  box-shadow: 0 1px 0 var(--c-border);
}
.none {
  color: var(--c-text-3);
}
</style>

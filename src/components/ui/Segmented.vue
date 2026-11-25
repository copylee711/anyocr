<script setup lang="ts" generic="T extends string | number">
// iOS 风格分段控件。
defineProps<{ options: { label: string; value: T }[]; disabled?: boolean }>();
const model = defineModel<T>({ required: true });
</script>

<template>
  <div class="seg" :class="{ disabled }" role="radiogroup">
    <button
      v-for="o in options"
      :key="String(o.value)"
      type="button"
      role="radio"
      :aria-checked="model === o.value"
      :class="{ on: model === o.value }"
      :disabled="disabled"
      @click="model = o.value"
    >
      {{ o.label }}
    </button>
  </div>
</template>

<style scoped>
.seg {
  display: inline-flex;
  padding: 2px;
  gap: 2px;
  border-radius: 9px;
  background: color-mix(in srgb, var(--c-text) 7%, transparent);
}
.seg button {
  border: 0;
  background: transparent;
  color: var(--c-text);
  font: inherit;
  font-size: 13px;
  padding: 4px 12px;
  border-radius: 7px;
  cursor: pointer;
  white-space: nowrap;
  transition:
    background 0.15s,
    box-shadow 0.15s;
}
.seg button.on {
  background: var(--c-surface);
  font-weight: 600;
  box-shadow:
    0 1px 3px rgba(0, 0, 0, 0.12),
    0 0 0 0.5px rgba(0, 0, 0, 0.04);
}
.seg.disabled {
  opacity: 0.5;
}
</style>

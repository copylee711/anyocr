<script setup lang="ts">
// 分组内的一行：左侧标题（可带说明），右侧控件。
defineProps<{ label: string; hint?: string; stacked?: boolean }>();
</script>

<template>
  <div class="row" :class="{ stacked }">
    <div class="label">
      <span>{{ label }}</span>
      <small v-if="hint">{{ hint }}</small>
    </div>
    <div class="control"><slot /></div>
  </div>
</template>

<style scoped>
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  min-height: 44px;
  padding: 7px 14px;
}
.row.stacked {
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
  padding: 10px 14px 12px;
}
.label {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.label span {
  font-size: 14px;
}
.label small {
  font-size: 12px;
  color: var(--c-text-2);
  margin-top: 1px;
}
.control {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.row.stacked .control {
  flex-wrap: wrap;
}
</style>

<script setup lang="ts">
// iOS Inset Grouped 列表分组：小标题 + 圆角卡片 + 底部说明。
defineProps<{ title?: string; footer?: string }>();
</script>

<template>
  <section class="group">
    <h3 v-if="title">{{ title }}</h3>
    <div class="rows"><slot /></div>
    <p v-if="footer" class="footer">{{ footer }}</p>
  </section>
</template>

<style scoped>
.group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
h3 {
  margin: 0 0 0 14px;
  font-size: 12px;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.02em;
  color: var(--c-text-2);
}
.rows {
  background: var(--c-surface);
  border-radius: 12px;
  overflow: hidden;
  border: 0.5px solid var(--c-border);
}
.rows > :deep(.row + .row) {
  border-top: 0.5px solid var(--c-border);
}
.footer {
  margin: 0 14px;
  font-size: 12px;
  color: var(--c-text-2);
  line-height: 1.5;
}
</style>

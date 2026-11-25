<script setup lang="ts">
import { useI18n } from "vue-i18n";

import AppIcon from "./AppIcon.vue";

defineProps<{ titleKey: string; descKey: string; icon: string; milestone: string }>();
const { t } = useI18n();
</script>

<template>
  <section class="page">
    <header class="page-head">
      <h1>{{ t(titleKey) }}</h1>
      <p>{{ t(descKey) }}</p>
    </header>
    <div class="card empty">
      <div class="badge"><AppIcon :name="icon" :size="28" /></div>
      <strong>{{ t("common.comingSoon") }}</strong>
      <span>{{ t("common.milestone", { m: milestone }) }}</span>
    </div>
  </section>
</template>

<style scoped>
.empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--c-text-2);
}
.badge {
  display: grid;
  place-items: center;
  width: 64px;
  height: 64px;
  border-radius: 20px;
  color: var(--c-primary);
  background: var(--c-primary-soft);
  margin-bottom: 6px;
}
strong {
  color: var(--c-text);
  font-size: 15px;
}
</style>

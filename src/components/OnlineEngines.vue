<script setup lang="ts">
// 设置页：识别引擎链（fallback 顺序）与各在线服务的密钥、参数、测试连接。
import { computed, onMounted, reactive, ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { NButton, NInput, NInputNumber, NSelect, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";

import { onlinePresets, onlineTest, type Preset, secretSet, secretStatus, type TestResult } from "../api/online";
import { useSettings } from "../stores/settings";
import AppIcon from "./AppIcon.vue";
import Group from "./ui/Group.vue";
import Row from "./ui/Row.vue";

const { t } = useI18n();
const message = useMessage();
const settings = useSettings();
const online = computed(() => settings.s.online);

const presets = ref<Preset[]>([]);
const mineru = ref<Preset | null>(null);
/** 引擎 id → 字段 → 已保存密钥的末 4 位 */
const saved = reactive<Record<string, Record<string, string | null>>>({});
/** 正在编辑但尚未保存的密钥 */
const drafts = reactive<Record<string, string>>({});
const expanded = ref<string | null>(null);
const testing = ref<string | null>(null);
const results = reactive<Record<string, TestResult>>({});

const all = computed(() => (mineru.value ? [...presets.value, mineru.value] : presets.value));
const byId = (id: string) => all.value.find((p) => p.info.id === id);

async function refresh(id: string) {
  saved[id] = await secretStatus(id);
}

onMounted(async () => {
  if (!isTauri()) return;
  const r = await onlinePresets();
  presets.value = r.engines;
  mineru.value = r.mineru;
  await Promise.all(all.value.map((p) => refresh(p.info.id)));
});

/** 密钥是否齐全 */
function configured(id: string) {
  const p = byId(id);
  if (!p) return id === "local" || id === "vlm";
  return p.fields.filter((f) => f.secret).every((f) => saved[id]?.[f.key]);
}

// ---------- 引擎链 ----------
function nameOf(id: string) {
  if (id === "local") return t("online.local");
  if (id === "vlm") return t("online.vlmEngine");
  return byId(id)?.info.name ?? id;
}

function move(i: number, d: -1 | 1) {
  const c = online.value.chain;
  const j = i + d;
  if (j < 0 || j >= c.length) return;
  [c[i], c[j]] = [c[j], c[i]];
}

function remove(i: number) {
  if (online.value.chain.length <= 1) return;
  online.value.chain.splice(i, 1);
}

const addOptions = computed(() =>
  ["local", "vlm", ...presets.value.map((p) => p.info.id)]
    .filter((id) => !online.value.chain.includes(id))
    .map((id) => ({ label: nameOf(id), value: id })),
);

function add(id: string | null) {
  if (id && !online.value.chain.includes(id)) online.value.chain.push(id);
}

// ---------- 字段 ----------
function fieldValue(id: string, key: string, def = "") {
  return online.value.engines[id]?.[key] ?? def;
}

function setField(id: string, key: string, v: string) {
  online.value.engines[id] = { ...(online.value.engines[id] ?? {}), [key]: v };
}

async function saveSecret(id: string, key: string) {
  const k = `${id}/${key}`;
  const v = drafts[k];
  if (v === undefined || !v.trim()) return;
  try {
    await secretSet(id, key, v.trim());
    delete drafts[k];
    await refresh(id);
    message.success(t("online.saved"));
  } catch (e) {
    message.error(String(e));
  }
}

async function clearSecret(id: string, key: string) {
  await secretSet(id, key, "");
  delete drafts[`${id}/${key}`];
  await refresh(id);
}

async function test(id: string) {
  testing.value = id;
  try {
    results[id] = await onlineTest(id);
  } catch (e) {
    results[id] = { ok: false, elapsedMs: 0, text: "", error: String(e) };
  } finally {
    testing.value = null;
  }
}

const preview = (s: string) => s.replace(/\s+/g, " ").trim().slice(0, 80);
</script>

<template>
  <Group :title="t('online.chain')" :footer="t('online.chainHint')">
    <div v-for="(id, i) in online.chain" :key="id" class="chain-row">
      <span class="badge">{{ i + 1 }}</span>
      <AppIcon :name="id === 'local' ? 'chip' : id === 'vlm' ? 'sparkle' : 'cloud'" :size="18" class="kind" />
      <span class="name">{{ nameOf(id) }}</span>
      <span v-if="!configured(id)" class="tag warn">{{ t("online.noKey") }}</span>
      <span v-else-if="id !== 'local' && byId(id) && !byId(id)!.info.has_boxes" class="tag">{{
        t("online.noBoxes")
      }}</span>
      <span class="spacer" />
      <NButton quaternary circle size="small" :disabled="i === 0" :title="t('online.up')" @click="move(i, -1)">
        <template #icon><AppIcon name="up" :size="16" /></template>
      </NButton>
      <NButton
        quaternary
        circle
        size="small"
        :disabled="i === online.chain.length - 1"
        :title="t('online.down')"
        @click="move(i, 1)"
      >
        <template #icon><AppIcon name="down" :size="16" /></template>
      </NButton>
      <NButton
        quaternary
        circle
        size="small"
        :disabled="online.chain.length <= 1"
        :title="t('online.remove')"
        @click="remove(i)"
      >
        <template #icon><AppIcon name="close" :size="16" /></template>
      </NButton>
    </div>
    <Row v-if="addOptions.length" :label="t('online.add')">
      <NSelect
        :value="null"
        size="small"
        style="width: 220px"
        :placeholder="t('online.addPlaceholder')"
        :options="addOptions"
        @update:value="add"
      />
    </Row>
    <Row :label="t('online.timeout')" :hint="t('online.timeoutHint')">
      <NInputNumber v-model:value="online.timeout" size="small" :min="5" :max="600" style="width: 110px">
        <template #suffix>s</template>
      </NInputNumber>
    </Row>
    <Row :label="t('online.concurrency')" :hint="t('online.concurrencyHint')">
      <NInputNumber v-model:value="online.concurrency" size="small" :min="1" :max="16" style="width: 110px" />
    </Row>
  </Group>

  <Group :title="t('online.services')" :footer="t('online.servicesHint')">
    <div v-for="p in all" :key="p.info.id" class="svc" :class="{ open: expanded === p.info.id }">
      <button class="svc-head" @click="expanded = expanded === p.info.id ? null : p.info.id">
        <AppIcon name="cloud" :size="18" class="kind" />
        <span class="name">{{ p.info.name }}</span>
        <span v-if="p.info.id === 'mineru'" class="tag">{{ t("online.docOnly") }}</span>
        <span v-if="!p.verified" class="tag dim">{{ t("online.unverified") }}</span>
        <span class="spacer" />
        <span class="state" :class="{ ok: configured(p.info.id) }">
          {{ configured(p.info.id) ? t("online.ready") : t("online.noKey") }}
        </span>
        <AppIcon name="chevron" :size="16" class="chev" />
      </button>

      <div v-if="expanded === p.info.id" class="svc-body">
        <p class="note">{{ p.note }}</p>
        <div v-for="f in p.fields" :key="f.key" class="field">
          <label>
            {{ f.label }}
            <small v-if="f.hint">{{ f.hint }}</small>
          </label>
          <template v-if="f.secret">
            <div class="secret">
              <NInput
                v-model:value="drafts[`${p.info.id}/${f.key}`]"
                type="password"
                show-password-on="click"
                size="small"
                :placeholder="
                  saved[p.info.id]?.[f.key]
                    ? t('online.savedTail', { tail: saved[p.info.id][f.key] })
                    : t('online.enter')
                "
                @keyup.enter="saveSecret(p.info.id, f.key)"
                @blur="saveSecret(p.info.id, f.key)"
              />
              <NButton v-if="saved[p.info.id]?.[f.key]" size="small" quaternary @click="clearSecret(p.info.id, f.key)">
                {{ t("online.clear") }}
              </NButton>
            </div>
          </template>
          <NSelect
            v-else-if="f.options?.length"
            size="small"
            :value="fieldValue(p.info.id, f.key, f.default)"
            :options="f.options.map((o) => ({ label: o, value: o }))"
            @update:value="(v: string) => setField(p.info.id, f.key, v)"
          />
          <NInput
            v-else
            size="small"
            :value="fieldValue(p.info.id, f.key)"
            :placeholder="f.default || ''"
            @update:value="(v: string) => setField(p.info.id, f.key, v)"
          />
        </div>
        <div class="actions">
          <NButton v-if="p.info.key_url" size="small" secondary @click="openUrl(p.info.key_url!)">
            {{ t("online.getKey") }}
            <template #icon><AppIcon name="external" :size="15" /></template>
          </NButton>
          <NButton
            size="small"
            type="primary"
            secondary
            :loading="testing === p.info.id"
            :disabled="!configured(p.info.id) || (testing !== null && testing !== p.info.id)"
            @click="test(p.info.id)"
          >
            {{ t("online.test") }}
          </NButton>
          <NButton
            v-if="p.info.id !== 'mineru' && !online.chain.includes(p.info.id)"
            size="small"
            quaternary
            @click="add(p.info.id)"
          >
            <template #icon><AppIcon name="plus" :size="15" /></template>
            {{ t("online.addToChain") }}
          </NButton>
        </div>
        <div v-if="results[p.info.id]" class="result" :class="{ ok: results[p.info.id].ok }">
          <template v-if="results[p.info.id].ok">
            {{ t("online.testOk", { ms: results[p.info.id].elapsedMs }) }}
            <code>{{ preview(results[p.info.id].text) }}</code>
          </template>
          <template v-else>{{ results[p.info.id].error }}</template>
        </div>
      </div>
    </div>
  </Group>
</template>

<style scoped>
.chain-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 44px;
  padding: 4px 8px 4px 14px;
}
.badge {
  flex: none;
  width: 20px;
  height: 20px;
  display: grid;
  place-items: center;
  border-radius: 6px;
  font: 600 11px/1 var(--font-mono);
  color: var(--c-primary);
  background: var(--c-primary-soft);
}
.kind {
  flex: none;
  color: var(--c-text-2);
}
.name {
  font-size: 14px;
  white-space: nowrap;
}
.spacer {
  flex: 1;
}
.tag {
  flex: none;
  font-size: 11px;
  padding: 1px 7px;
  border-radius: 999px;
  color: var(--c-primary);
  background: var(--c-primary-soft);
}
.tag.warn {
  color: #c77700;
  background: color-mix(in srgb, #ff9f0a 16%, transparent);
}
.tag.dim {
  color: var(--c-text-2);
  background: color-mix(in srgb, var(--c-text) 6%, transparent);
}
.svc + .svc {
  border-top: 1px solid var(--c-separator, color-mix(in srgb, var(--c-text) 8%, transparent));
}
.svc-head {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 44px;
  padding: 0 14px;
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.svc-head:hover {
  background: color-mix(in srgb, var(--c-text) 3%, transparent);
}
.state {
  font-size: 12px;
  color: var(--c-text-3, var(--c-text-2));
}
.state.ok {
  color: #248a3d;
}
.chev {
  color: var(--c-text-3, var(--c-text-2));
  transition: transform 0.18s ease;
}
.svc.open .chev {
  transform: rotate(90deg);
}
.svc-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 2px 14px 14px 42px;
}
.note {
  margin: 0;
  font-size: 12px;
  color: var(--c-text-2);
}
.field {
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
}
.field label {
  display: flex;
  flex-direction: column;
  font-size: 13px;
}
.field label small {
  font-size: 11px;
  color: var(--c-text-2);
}
.secret {
  display: flex;
  gap: 6px;
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 2px;
}
.result {
  font-size: 12px;
  line-height: 1.5;
  padding: 8px 10px;
  border-radius: 8px;
  color: #d70015;
  background: color-mix(in srgb, #ff3b30 10%, transparent);
  word-break: break-all;
}
.result.ok {
  color: #248a3d;
  background: color-mix(in srgb, #34c759 12%, transparent);
}
.result code {
  display: block;
  margin-top: 3px;
  color: var(--c-text);
  font: 12px/1.5 var(--font-mono);
}
</style>

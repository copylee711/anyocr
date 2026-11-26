<script setup lang="ts">
// 主窗口的全局监听：批量任务进度、截图结果、快捷键注册。放在 NMessageProvider 内以便弹出提示。
import { onBeforeUnmount, onMounted, watch } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

import { onJobEvent } from "../api/app";
import { useCapture } from "../stores/capture";
import { useSettings } from "../stores/settings";
import { useTasks } from "../stores/tasks";
import { useUpdater } from "../stores/updater";

const { t } = useI18n();
const message = useMessage();
const router = useRouter();
const settings = useSettings();
const tasks = useTasks();
const capture = useCapture();
const updater = useUpdater();
const unlisteners: (() => void)[] = [];

async function applyHotkeys() {
  if (!isTauri()) return;
  const errors = await invoke<{ name: string; error: string }[]>("apply_hotkeys", {
    hotkeys: { ...settings.s.hotkeys },
  });
  for (const e of errors) message.error(t("hotkey.failed", { name: t(`hotkey.names.${e.name}`), error: e.error }));
}

onMounted(async () => {
  await settings.load();
  unlisteners.push(await onJobEvent(tasks.handle));
  unlisteners.push(
    ...(await capture.listenAll(
      (r) => {
        if (r.error) message.error(r.error);
        else if (!r.text) message.info(r.mode === "ocr" ? t("screenshot.noText") : t("qrcode.none"));
        else if (r.mode === "ocr" && settings.s.vlm.review.screenshot)
          capture.startReview(settings.s.screenshot.copy ? r.text : null);
        router.push(r.mode === "ocr" ? "/screenshot" : "/qrcode");
      },
      (e) => message.warning(e),
    )),
  );
  // 设置加载后立即注册一次（首次运行时设置文件里还没有快捷键，需要用默认值），之后随修改重新注册
  await applyHotkeys();
  watch(() => JSON.stringify(settings.s.hotkeys), applyHotkeys);
  // 启动后稍等再检查更新，避免与模型加载、首屏渲染抢资源；失败时静默
  if (settings.s.update.autoCheck) setTimeout(() => updater.checkNow(true), 4000);
});
onBeforeUnmount(() => unlisteners.forEach((u) => u()));
</script>

<template>
  <span hidden />
</template>

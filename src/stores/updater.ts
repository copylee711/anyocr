// 检查更新：读取 GitHub 最新 Release 的 latest.json（Tauri updater），一键下载安装并重启。
import { defineStore } from "pinia";
import { isTauri } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { ref, shallowRef } from "vue";

import { useSettings } from "./settings";

export type UpdateStatus = "idle" | "checking" | "latest" | "available" | "downloading" | "ready" | "error";

export const useUpdater = defineStore("updater", () => {
  const settings = useSettings();
  const status = ref<UpdateStatus>("idle");
  const update = shallowRef<Update | null>(null);
  const error = ref<string | null>(null);
  /** 下载进度：已下载 / 总字节数（总数未知时为 0） */
  const downloaded = ref(0);
  const total = ref(0);
  const dialog = ref(false);
  const checkedAt = ref<number | null>(null);

  /**
   * 检查更新。`auto` 为启动时的自动检查：失败时不提示，用户忽略过的版本不弹窗。
   * 返回是否有新版本。
   */
  async function checkNow(auto = false): Promise<boolean> {
    if (!isTauri() || status.value === "checking" || status.value === "downloading") return false;
    status.value = "checking";
    error.value = null;
    try {
      const u = await check({ timeout: 20_000 });
      checkedAt.value = Date.now();
      update.value = u;
      if (!u) {
        status.value = "latest";
        return false;
      }
      status.value = "available";
      if (!auto || u.version !== settings.s.update.ignoredVersion) dialog.value = true;
      return true;
    } catch (e) {
      status.value = auto ? "idle" : "error";
      error.value = String(e);
      return false;
    }
  }

  /** 下载并安装，完成后重启应用。Windows 上安装程序会接管并自动关闭本应用。 */
  async function install() {
    const u = update.value;
    if (!u) return;
    status.value = "downloading";
    downloaded.value = 0;
    total.value = 0;
    try {
      await u.downloadAndInstall((ev) => {
        if (ev.event === "Started") total.value = ev.data.contentLength ?? 0;
        else if (ev.event === "Progress") downloaded.value += ev.data.chunkLength;
      });
      status.value = "ready";
      await relaunch();
    } catch (e) {
      status.value = "error";
      error.value = String(e);
    }
  }

  /** 忽略这个版本：之后启动时不再为它弹窗（手动检查仍会显示）。 */
  function ignore() {
    if (update.value) settings.s.update.ignoredVersion = update.value.version;
    dialog.value = false;
  }

  return { status, update, error, downloaded, total, dialog, checkedAt, checkNow, install, ignore };
});

// 监听窗口拖放文件（Tauri WebView 事件），仅在组件挂载期间生效。
import { onBeforeUnmount, onMounted, ref } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";

export function useDropFiles(onDrop: (paths: string[]) => void) {
  const hovering = ref(false);
  let unlisten: (() => void) | undefined;

  onMounted(async () => {
    if (!isTauri()) return;
    unlisten = await getCurrentWebview().onDragDropEvent((e) => {
      const p = e.payload;
      if (p.type === "enter" || p.type === "over") hovering.value = true;
      else if (p.type === "leave") hovering.value = false;
      else if (p.type === "drop") {
        hovering.value = false;
        if (p.paths.length) onDrop(p.paths);
      }
    });
  });
  onBeforeUnmount(() => unlisten?.());

  return { hovering };
}

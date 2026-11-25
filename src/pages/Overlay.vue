<script setup lang="ts">
// 截图遮罩：显示冻结的屏幕画面，拖动框选区域。Esc / 右键取消。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useRoute } from "vue-router";

const route = useRoute();
const monitor = Number(route.query.i ?? 0);
const canvas = ref<HTMLCanvasElement | null>(null);

const start = ref<{ x: number; y: number } | null>(null);
const cur = ref<{ x: number; y: number }>({ x: -1, y: -1 });
const dragging = ref(false);
const done = ref(false);

const rect = computed(() => {
  if (!start.value) return null;
  const x = Math.min(start.value.x, cur.value.x);
  const y = Math.min(start.value.y, cur.value.y);
  return { x, y, w: Math.abs(cur.value.x - start.value.x), h: Math.abs(cur.value.y - start.value.y) };
});
const dpr = window.devicePixelRatio || 1;

/** 取截图原始像素（前 8 字节为宽高）画到 canvas，画好后再显示窗口，避免白屏闪烁。 */
async function draw() {
  const buf = await invoke<ArrayBuffer>("overlay_pixels", { monitor });
  const view = new DataView(buf);
  const w = view.getUint32(0, true);
  const h = view.getUint32(4, true);
  const el = canvas.value;
  if (!el) return;
  el.width = w;
  el.height = h;
  el.getContext("2d")?.putImageData(new ImageData(new Uint8ClampedArray(buf, 8, w * h * 4), w, h), 0, 0);
  await invoke("overlay_ready");
}

function down(e: MouseEvent) {
  if (e.button !== 0 || done.value) return;
  start.value = { x: e.clientX, y: e.clientY };
  cur.value = { x: e.clientX, y: e.clientY };
  dragging.value = true;
}

function move(e: MouseEvent) {
  cur.value = { x: e.clientX, y: e.clientY };
}

async function up(e: MouseEvent) {
  if (e.button !== 0 || !dragging.value) return;
  dragging.value = false;
  const r = rect.value;
  if (!r || r.w < 4 || r.h < 4) {
    start.value = null;
    return;
  }
  done.value = true;
  await invoke("capture_finish", {
    selection: {
      monitor,
      x: r.x,
      y: r.y,
      width: r.w,
      height: r.h,
      viewWidth: window.innerWidth,
      viewHeight: window.innerHeight,
    },
  });
}

function cancel() {
  if (done.value) return;
  done.value = true;
  invoke("capture_cancel");
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") cancel();
}

onMounted(() => {
  document.documentElement.classList.add("overlay-mode");
  window.addEventListener("keydown", onKey);
  draw().catch(() => invoke("capture_cancel"));
});
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="overlay" @mousedown="down" @mousemove="move" @mouseup="up" @contextmenu.prevent.stop="cancel">
    <canvas ref="canvas" />
    <div class="dim" :class="{ clear: rect }" />
    <template v-if="rect">
      <div
        class="sel"
        :style="{ left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.w}px`, height: `${rect.h}px` }"
      />
      <div class="size" :style="{ left: `${rect.x}px`, top: `${Math.max(rect.y - 30, 4)}px` }">
        {{ Math.round(rect.w * dpr) }} × {{ Math.round(rect.h * dpr) }}
      </div>
    </template>
    <template v-else-if="cur.x >= 0">
      <div class="cross-h" :style="{ top: `${cur.y}px` }" />
      <div class="cross-v" :style="{ left: `${cur.x}px` }" />
    </template>
    <div class="tip">拖动选择识别区域 · Esc 或右键取消</div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  cursor: crosshair;
  overflow: hidden;
  user-select: none;
  background: #000;
}
canvas {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
}
.dim {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.35);
}
.dim.clear {
  background: transparent;
}
.sel {
  position: absolute;
  outline: 1.5px solid #0a84ff;
  /* 选区外暗化 */
  box-shadow: 0 0 0 100000px rgba(0, 0, 0, 0.45);
}
.size {
  position: absolute;
  padding: 3px 8px;
  border-radius: 6px;
  font:
    600 12px/1.4 -apple-system,
    "Segoe UI",
    sans-serif;
  color: #fff;
  background: rgba(10, 132, 255, 0.92);
  pointer-events: none;
}
.cross-h,
.cross-v {
  position: absolute;
  background: rgba(10, 132, 255, 0.7);
  pointer-events: none;
}
.cross-h {
  left: 0;
  right: 0;
  height: 1px;
}
.cross-v {
  top: 0;
  bottom: 0;
  width: 1px;
}
.tip {
  position: absolute;
  top: 18px;
  left: 50%;
  transform: translateX(-50%);
  padding: 6px 14px;
  border-radius: 999px;
  font:
    500 13px/1.4 -apple-system,
    "Segoe UI",
    "Microsoft YaHei UI",
    sans-serif;
  color: #fff;
  background: rgba(30, 30, 32, 0.72);
  backdrop-filter: blur(12px);
  pointer-events: none;
}
</style>

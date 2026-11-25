/// <reference types="vitest/config" />
import process from "node:process";

import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vite";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [vue()],
  // Tauri 开发：不清屏以免遮住 Rust 报错；固定端口
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/crates/**"] },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
}));

#!/usr/bin/env node
// 启动器：找到当前平台的预编译 anyocr 二进制并原样转发参数、标准输入输出（MCP 走 stdio）。
// 保持 CommonJS，任何 Node 18+ 与 npx 都能直接运行。
/* eslint-disable @typescript-eslint/no-require-imports */
"use strict";

const { spawn } = require("node:child_process");
const path = require("node:path");

const PLATFORMS = {
  "win32-x64": "@copylee/anyocr-win32-x64",
};

const key = `${process.platform}-${process.arch}`;
const pkg = PLATFORMS[key];
if (!pkg) {
  console.error(
    `anyocr 暂未提供 ${key} 平台的预编译版本，目前支持：${Object.keys(PLATFORMS).join(", ")}。\n` +
      "可以从源码安装：https://github.com/copylee711/anyocr/blob/main/docs/AGENTS-INTEGRATION.md",
  );
  process.exit(1);
}

let dir;
try {
  dir = path.dirname(require.resolve(`${pkg}/package.json`));
} catch {
  console.error(`没有找到平台包 ${pkg}。请重新安装（不要使用 --no-optional / --omit=optional）。`);
  process.exit(1);
}

const exe = path.join(dir, process.platform === "win32" ? "anyocr.exe" : "anyocr");
const child = spawn(exe, process.argv.slice(2), {
  stdio: "inherit",
  // PDFium 动态库与二进制放在同一目录
  env: { ...process.env, ANYOCR_PDFIUM_DIR: process.env.ANYOCR_PDFIUM_DIR || dir },
  windowsHide: true,
});
for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => child.kill(sig));
child.on("error", (e) => {
  console.error(`无法启动 ${exe}：${e.message}`);
  process.exit(1);
});
child.on("exit", (code, signal) => process.exit(signal ? 1 : (code ?? 0)));

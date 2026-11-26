# AGENTS.md

写给在本仓库工作的 AI 编码代理（Claude Code、Codex 等）和人类贡献者。

## 项目概述

AnyOCR：本地优先的 OCR 桌面应用。它用 Tauri 2 + Vue 3 重写 Umi-OCR，并新增三项能力：在线 OCR 引擎、视觉大模型（VLM）识别与并行复核、扫描件 PDF 加可搜索文本层（参考 OCRmyPDF）。

- 开发计划与里程碑：[docs/PLAN.md](docs/PLAN.md)。开工前先看当前处于哪个里程碑。
- 调研记录与外部 API 事实：[docs/RESEARCH.md](docs/RESEARCH.md)。
- 目标平台：Windows 10/11 优先，代码保持跨平台可编译。

## 目录结构（规划，按里程碑逐步落地）

```
src/                Vue 3 + TypeScript 前端（Naive UI、Pinia、vue-i18n）
src-tauri/          Tauri 2 应用壳：窗口、托盘、快捷键、截图遮罩、命令层
crates/
  anyocr-core       数据模型、Engine trait、任务调度、tbpu 排版解析、配置
  anyocr-engines    本地引擎（oar-ocr/ort）、PaddleOCR AI Studio、MinerU、VLM
  anyocr-review     VLM 并行复核流水线
  anyocr-pdf        pdfium 渲染、GlyphLess 文字层写入（lopdf）
  anyocr-output     各种输出格式
  anyocr-qrcode     二维码读写
  anyocr-server     兼容 Umi-OCR 的 HTTP API（axum，默认 1224 端口）
  anyocr-cli        命令行
docs/               计划、调研、使用文档
scripts/            验收、模型下载等脚本
tests/fixtures/     可公开的小样张
```

## 常用命令

```powershell
pwsh scripts/fetch-pdfium.ps1   # 首次：下载 PDFium 到 vendor/pdfium/（固定 chromium/8179）
pnpm install                    # 安装前端依赖
pnpm tauri dev                  # 启动桌面开发模式（Rust 包名 anyocr-desktop）
pnpm tauri build                # 打包 NSIS 安装包
cargo test --workspace          # Rust 测试
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
pnpm lint; pnpm typecheck; pnpm test; pnpm format

# CLI（crates/anyocr-cli，二进制名 anyocr）
cargo run --release -p anyocr-cli -- ocr a.png --parser multi_para --format json
cargo run --release -p anyocr-cli -- bench book.pdf --pages 1-20 --device directml
cargo run -p anyocr-cli -- mcp                      # stdio MCP 服务（智能体接入，见 docs/AGENTS-INTEGRATION.md）

# 发布：见 docs/RELEASE.md（推送 v* 标签触发 release.yml；更新签名私钥不在仓库里）
pwsh scripts/pack-npm.ps1          # 组装 npm 包（anyocr + @copylee/anyocr-win32-x64）
```

- 模型首次使用时自动下载到 `%APPDATA%/anyocr/models`（桌面端为 Tauri 的 app_data_dir/models）。
- `Cargo.toml` 中设置了 `[profile.dev.package."*"] opt-level = 3`，依赖在 debug 下也会优化，否则推理慢到无法使用。
- 前端依赖是较新的大版本（pinia 4、vue-router 5、eslint 10、vite 8）。写代码前用 Context7 查当前 API。

## 架构约束

1. **OCR 与输出解耦**：所有引擎统一产出 `anyocr-core` 的 `PageResult` / `TextBlock`（文字、四点框、置信度、来源）。输出格式和 PDF 文字层只依赖这个结构，不得直接依赖某个具体引擎。
2. **核心逻辑不依赖 Tauri**：crates 中不引用 `tauri`。`src-tauri` 只做薄的命令和事件转发。CLI 和 HTTP 服务复用同一份核心逻辑。
3. **前端不做重计算**：图像处理、OCR、PDF 读写都在 Rust 侧完成。前端通过 `invoke` 调用，通过事件接收进度。
4. **长任务可取消、可暂停**：使用 tokio + `CancellationToken`；进度以事件节流推送（≤ 10 次/秒）。
5. **PDF 文字层**：使用 GlyphLessFont + `3 Tr` 不可见文字 + `Tz` 宽度适配，原页面图像对象不得重新编码。修改坐标变换逻辑时，必须跑旋转页 / CropBox 偏移的测试。
6. **VLM 复核**：只能修改已有行的文字，不能新增或删除行，也不能改坐标；修改必须通过编辑距离阈值校验并记录日志。复核不得阻塞 OCR 流水线，超出截止时间就保留原结果。

## 编码规范

- Rust：edition 2024，`thiserror` 定义库错误，`anyhow` 只用于二进制入口；禁止在库代码里 `unwrap()`/`expect()` 处理可恢复错误；公共 API 写文档注释。
- TypeScript：`strict`；Vue 使用 `<script setup lang="ts">` + Composition API；组件 PascalCase；与 Rust 交互的类型集中定义在 `src/api/`。
- 界面文字全部走 i18n，不硬编码。中文为源语言（`zh-CN`）。
- 从 Umi-OCR 移植的算法，在文件头注明原始文件路径和 MIT 来源。
- 注释和文档用中文；标识符用英文。

## 安全与隐私

- **任何 API Key / Token 都不得写入仓库**，包括测试、示例、日志和快照。应用内的密钥存进系统凭据管理器（`keyring`）；开发测试和 CLI 从环境变量读取：`ANYOCR_<引擎ID>_<字段>`（如 `ANYOCR_BAIDU_API_KEY`，`anyocr engines` 会列出全部变量名），视觉大模型用 `ANYOCR_VLM_<服务>_API_KEY`（DeepSeek 也认 `DEEPSEEK_API_KEY`），以及兼容的 `ANYOCR_PADDLE_TOKEN`、`ANYOCR_MINERU_TOKEN`、`DEEPSEEK_API_KEY`。
- 日志中出现密钥时只保留末 4 位。
- 用户的本地文件（如课本扫描件）只在本机验收时引用，不得复制进仓库或上传到 CI。
- 在线引擎默认关闭；用户显式启用前，不得向外部发送图片。

## 测试

- 新功能必须带测试：纯逻辑写单元测试，引擎和 PDF 写集成测试（使用 `tests/fixtures`）。
- 依赖网络或密钥的测试标记为 `#[ignore]`，或在没有环境变量时跳过。
- PDF 文本层改动后，运行验收脚本 `scripts/acceptance.ps1`（落地后提供），确认可搜索率和坐标重叠率。

## Git 约定

- 直接在 `main` 上小步提交并推送；较大的实验性改动开分支。
- 提交信息用中文，带 Conventional Commits 前缀：`feat:`、`fix:`、`docs:`、`refactor:`、`test:`、`chore:`、`ci:`。
- 提交作者只署仓库所有者本人。不要添加 `Co-Authored-By`，也不要在提交信息或 PR 中写入任何 AI 工具署名。
- 提交前确保 `cargo fmt`、`clippy`、前端 lint 和类型检查通过。

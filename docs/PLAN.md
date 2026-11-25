# AnyOCR 开发计划

AnyOCR 是一个本地优先的 OCR 桌面应用。它用 Tauri 2 + Vue 3 重写 [Umi-OCR](https://github.com/hiroi-sora/Umi-OCR)（MIT），复刻其全部功能，并在此基础上新增三项能力：

1. **在线 OCR 引擎**：PaddleOCR（AI Studio）、MinerU，以及可扩展的其他服务，API Key 由用户在设置页填写。
2. **视觉大模型（VLM）**：既可以单独作为 OCR 引擎，也可以与 OCR 引擎并行运行，对识别结果做二次复核和修正，并尽量不拉长总用时。
3. **扫描件 PDF 加文本层**（参考 OCRmyPDF）：原页面图像不重新编码，生成可搜索、可复制的 PDF。

调研结论见 [RESEARCH.md](RESEARCH.md)。

---

## 1. 总体架构

```
┌──────────────── Vue 3 前端（src/）────────────────┐
│ 标签页：截图OCR │ 批量OCR │ 批量文档/PDF文本层 │ 二维码 │ 设置 │ 关于 │
│ Pinia 状态 · vue-i18n · Naive UI · 图片/PDF 预览与文本框叠加 │
└───────────────▲ Tauri invoke / event ▼────────────┘
┌──────────────── src-tauri（应用壳）────────────────┐
│ 窗口/托盘/全局快捷键/截图遮罩窗口/单实例/自启/剪贴板       │
│ 命令层：把前端调用转发给 crates，把进度以事件推回前端      │
└───────────────▲ Rust API ▼─────────────────────────┘
 crates/
  anyocr-core     数据模型（TextBlock/PageResult）、Engine trait、任务调度、
                  tbpu 排版解析（移植 Umi-OCR 的 gap_tree 等）、忽略区域、配置
  anyocr-engines  local（oar-ocr + ort，PP-OCRv5/v6 ONNX）
                  paddle_aistudio · mineru · vlm（OpenAI 兼容）
  anyocr-review   VLM 并行复核流水线（调度、提示词、差异合并、防幻觉校验）
  anyocr-pdf      pdfium 渲染/读取、GlyphLess 文字层写入（lopdf）、跳过已有文字页
  anyocr-output   txt / txtPlain / txtIndividual / md / csv / jsonl / pdfLayered / pdfOneLayer
  anyocr-qrcode   rxing 读写 + 预处理
  anyocr-server   axum HTTP 服务（兼容 Umi-OCR /api/ocr /api/doc /api/qrcode）
  anyocr-cli      命令行：Umi 兼容指令 + `anyocr pdf in.pdf out.pdf`（类 ocrmypdf）
```

设计原则：

- **OCR 与输出解耦**。所有引擎（本地、在线、VLM）统一输出 `PageResult { blocks: [TextBlock{ text, box: [[x,y];4], score, source }] , width, height }`。文字层和各输出格式只消费这个结构，因此任何引擎都能生成双层 PDF。
- **核心逻辑放在 crates 里，不依赖 Tauri**。CLI、HTTP 服务、桌面端共用同一份核心逻辑，也可以脱离 GUI 跑测试和批量验收。
- **本地优先**。默认使用本地引擎，离线可用；只有用户显式启用时，才把图片发给在线服务。
- **密钥安全**。API Key 用 `keyring` 存进 Windows 凭据管理器，不写入明文配置，也不进仓库。

## 2. 关键技术方案

### 2.1 本地 OCR 引擎
- 使用 `oar-ocr`（基于 ort）加载 PP-OCRv5 mobile 的检测和识别 ONNX 模型；可选文本行方向分类、文档方向分类。
- 模型首次运行时下载到 `%APPDATA%/anyocr/models`，也可以随安装包附带一套中英文 mobile 模型（约 15 MB）。
- 执行后端：CPU 默认可用，可选 DirectML（覆盖任意 Windows GPU）和 CUDA（RTX 5060），在设置页切换。
- 并发：页级并行，worker 数可配置；检测和识别共享 session。
- 先做性能 spike：在 `常微分方程` 前 20 页上比较 CPU 和 DirectML 的速度与准确率，再决定默认配置。

### 2.2 PDF 文本层（OCRmyPDF 思路）
- 渲染：`pdfium-render` 按目标 DPI 渲染（默认 300，不低于 Umi-OCR 的 1080 px 下限），同时读取页面已有文字，用于判断是否跳过。
- 写入：`lopdf` 打开原 PDF，给每页追加一个内容流：
  - 字体：GlyphLessFont（Type0 + CIDFontType2，`Identity-H`，CID = UTF-16 码元，`CIDToGIDMap` 全部映射到 1，附 `ToUnicode` CMap）。全文档只嵌入一次，体积约 1 KB。
  - 每行：`BT /F 1 Tf 3 Tr` + 按框高设字号 + 按框宽计算 `Tz` 水平缩放 + 旋转矩阵（支持竖排和倾斜框）+ `ET`。
  - 处理页面 `/Rotate`、`MediaBox` 与 `CropBox` 偏移、图片坐标到 PDF 坐标的换算。
  - 原始图像对象不动，增量保存或整体重写可选。
- 模式：`skip-text`（默认，有文字的页跳过）、`force`（强制 OCR 整页）、`redo`（删除旧的不可见文字后重做），对应 Umi-OCR 的 mixed / fullPage / imageOnly / textOnly 提取模式。
- 选项：页范围、密码、可选纠偏/旋转（只影响识别，不改原图）、输出单层纯文字 PDF（pdfOneLayer）。PDF/A 列为后续阶段。
- 验收方式：用 pdfium 重新提取文字层，与 OCR 结果逐字比对；同时检查每个字符的提取坐标与 OCR 框的重叠率 ≥ 0.9；再到 Edge/SumatraPDF 里人工 Ctrl+F 抽查。

### 2.3 在线 OCR 引擎
- `paddle_aistudio`：异步 jobs API（PP-OCRv5 / PP-StructureV3 / PaddleOCR-VL-1.5）。整份 PDF 提交（≤ 50 MB 时本地上传，更大的按页范围切分），结果 JSONL 按页映射成 `PageResult`。适合批量文档；排队延迟较大，截图场景默认不用。
- `mineru`：v4 batch 上传 → 轮询 → 解压 zip，读取 `layout.json` 块级 bbox 和 `full.md`。用于“文档转 Markdown（含公式 LaTeX、表格）”输出，也可以用块级 bbox 生成精度较粗的文字层。
- 统一的 `OnlineEngine` 适配层负责限流、重试、超时、配额错误提示、进度回传。后续可增加百度智能云、腾讯云等服务。
- 设置页：每个引擎一张卡片，包括启用开关、API Key、模型选择、“测试连接”按钮。

### 2.4 视觉大模型：VLM OCR 与并行复核
**Provider**：OpenAI Chat Completions 兼容协议，可配置 base_url、model、key、并发数、超时。预置 DeepSeek `deepseek-flash`（已实测支持图片），以及通义千问 VL、硅基流动、智谱、OpenAI、Ollama 的模板。

**模式 A：VLM 作为 OCR 引擎**
- 先用本地检测模型得到行框；再把整页图片和带编号的行框一起交给 VLM 转写，要求返回 `[{id, text}]`。这样 VLM 给出文字，坐标仍来自检测框，所以照样能生成文字层。
- 没有检测框时（纯 VLM 模式）只输出文本 / Markdown，不生成文字层。

**模式 B：并行复核（重点）**
目标：总用时 ≈ 仅本地 OCR 的用时 + 很短的尾部等待。
1. 流水线：渲染 → OCR → 复核 → 写出，阶段之间用有界通道连接。第 N 页在复核时，第 N+1 页已经在做 OCR，复核与 OCR 在时间上重叠。
2. 复核并发 K（默认 4，受 API 限流约束），按页提交。请求内容是页面图片（缩放到长边 ≤ 1600 px）加 OCR 行列表 `{id, text, score}`，要求只返回需要修改的行 `[{id, text}]`，以减少输出 token 和延迟。
3. 选择性复核：可选“只复核含低置信度行（score < 阈值）的页”，或者只送低置信度行的裁剪图，进一步省时省钱。
4. 截止时间：OCR 全部完成后，最多再等 `review_grace`（默认 10 s，可配置）。超时没有返回的页保留原 OCR 结果并标记“未复核”；之后用户可以单独补复核。
5. 截图 OCR 场景：OCR 结果立即显示并复制；VLM 返回后在界面上以差异高亮的方式就地更新，可撤销。
6. 防幻觉：VLM 的修改和原文的编辑距离比例超过阈值（默认 0.5），或行数、长度明显不一致时拒绝该修改；保留 OCR 坐标，只替换文字；每页记录修改日志，可导出。
7. 计时埋点：分别记录 OCR 用时、复核用时、尾部等待用时，在任务面板显示，作为验收数据。

### 2.5 复刻 Umi-OCR 的功能清单
- **截图 OCR**：全局快捷键截图（多屏、冻结屏幕选区窗口）、粘贴图片识别、重复上次截图、识别后自动复制 / 弹出主窗口、结果可编辑、图片预览叠加文本框、排版解析方案选择、截图前隐藏窗口。
- **批量 OCR**：拖入文件或文件夹（可递归）、文件表（状态 / 耗时）、忽略区域编辑器、输出目录（原目录 / 指定）、文件名模板 + 日期格式、输出格式 txt / txtPlain / txtIndividual / md / csv / jsonl、忽略空白、暂停 / 继续 / 停止、任务完成后关机 / 休眠。
- **批量文档**：PDF（以及 cbz、图片集）、页范围、密码、提取模式、忽略区域、输出 pdfLayered / pdfOneLayer / txt / txtPlain / csv / jsonl / md（MinerU）、预览。
- **二维码**：识别（多码制、预处理：中值滤波、锐度、对比度、灰度、阈值）、生成（码制、宽高、静区、纠错等级）。
- **全局设置**：主题（明 / 暗 / 跟随系统）、界面与数据字体、缩放、语言（zh-CN、zh-TW、en、ja 起步）、桌面 / 开始菜单快捷方式、开机自启、启动时隐藏、窗口置顶、托盘、通知、日志级别、HTTP 服务（开关 / 主机 / 端口）、引擎管理。
- **多标签页**：可增删的功能标签页、导航页、关于页（附 Umi-OCR 致谢与许可）。
- **HTTP API**：兼容 Umi-OCR v2 的 `/api/ocr`、`/api/ocr/get_options`、`/api/doc/*`、`/api/qrcode`，默认端口 1224，现有脚本可以直接迁移。
- **CLI**：兼容 `--screenshot --clipboard --path --qrcode_read --qrcode_create --clip --output --show --hide --quit`，未运行时自动拉起或转发给运行中的实例；另加 `anyocr pdf` 子命令。
- 与原版的差异：xps / epub / mobi / fb2 依赖 MuPDF（AGPL），不支持，会在文档中说明。

## 3. 里程碑

每个里程碑结束时都应可运行、可测试，并提交推送。

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **M0 脚手架** | Cargo workspace + crates 骨架；Tauri 2 + Vue 3 + TS + Vite + pnpm；Naive UI、Pinia、vue-i18n；ESLint/Prettier、rustfmt/clippy；GitHub Actions（Windows：test + build） | `pnpm tauri dev` 能打开空壳窗口；`cargo test` 通过；CI 绿 |
| **M1 核心 + 本地引擎** | anyocr-core 数据模型；oar-ocr 接入；模型下载 / 管理；tbpu 解析器移植（附 Umi 用例对照测试）；`anyocr ocr <img>` | 测试图片识别正确；在 `常微分方程` 前 20 页上给出 CPU / DirectML 速度基准 |
| **M2 PDF 文本层** | anyocr-pdf：渲染、GlyphLess 字体、文字层写入、skip / force / redo、页范围、密码；`anyocr pdf` | 4 页和 7 页的扫描件生成可搜索 PDF；提取比对通过；体积增量 < 2%；`应坚刚 概率论` 被正确跳过 |
| **M3 桌面主界面** | 应用壳（标签页、主题、i18n、托盘、单实例、设置持久化）；批量文档页（含 PDF 文本层）；批量 OCR 页；全部输出格式 | GUI 中完成整本 `常微分方程`（264 页）文本层，记录用时；输出格式与 Umi 样例一致 |
| **M4 截图与二维码** | 截图选区窗口、全局快捷键、粘贴识别、结果叠加预览；二维码页 | 快捷键截图 → 识别 → 复制全流程可用；二维码读写通过 |
| **M5 在线引擎** | 设置页 API Key（keyring）；PaddleOCR AI Studio、MinerU 适配；测试连接；Markdown 输出 | 用用户的令牌跑通两份扫描件；MinerU 产出 Markdown（含公式） |
| **M6 VLM** | Provider 配置；模式 A（VLM OCR）；模式 B（并行复核流水线、截止时间、防幻觉、差异高亮）；计时面板 | 7 页扫描件：开启复核后总用时 ≤ 仅 OCR 用时 + 10 s；抽样修正准确率人工核查 |
| **M7 兼容与发布** | HTTP API、CLI 兼容层；忽略区域编辑器；任务完成后动作；NSIS 安装包；README 与使用文档 | Umi-OCR 官方 API 示例脚本改端口即可运行；安装包在干净环境可用 |

## 4. 测试与验收

- 单元测试：tbpu 解析器（用 Umi-OCR 行为构造的固定输入 / 输出）、坐标变换、GlyphLess 字体与 ToUnicode 生成、差异合并与防幻觉规则。
- 集成测试：`tests/fixtures` 放少量可公开的小样张（自制渲染图片和小 PDF）；用户的课本扫描件只在本机验收时引用，**不进仓库**。
- 验收脚本：`scripts/acceptance.ps1` 依次跑 4 页、7 页、264 页三档，输出用时、页数、可搜索率、复核修改数，写成 Markdown 报告。
- 在线服务和 VLM 的测试只在显式设置环境变量时运行，默认跳过。环境变量：`ANYOCR_PADDLE_TOKEN`、`ANYOCR_MINERU_TOKEN`、`DEEPSEEK_API_KEY`。

## 5. 风险与对策

| 风险 | 对策 |
|---|---|
| oar-ocr API 变动或 Windows 构建问题 | M1 先做 spike；以 `paddle-ocr-rs` 或自写 ort 推理（DB 后处理 + CTC 解码）作为备选 |
| pdfium.dll 分发 | 用 bblanchon/pdfium-binaries 固定版本，作为 Tauri resource 打包，启动时校验 |
| 复杂 PDF（旋转页、CropBox 偏移、对象流、加密） | 坐标变换单测覆盖；失败页降级为跳过并记录日志，不中断整份文档 |
| 在线服务排队慢 / 配额 | 任务面板显示排队状态；超时后可回退到本地引擎 |
| VLM 幻觉或改错 | 只修改不新增行，编辑距离阈值，记录修改日志，可一键撤销 |
| 数学公式识别差 | 文字层场景接受；需要 LaTeX 时走 MinerU 或 VLM 的 Markdown 输出 |

## 6. 仓库与提交约定

- 本仓库 `copylee711/anyocr` 是 Umi-OCR 的重写版：不直接 fork 原仓库代码（技术栈不同），在 `NOTICE` 与关于页保留 Umi-OCR 的 MIT 版权声明。移植算法（如 tbpu）时，在文件头注明来源。
- 每完成一个可运行的步骤就提交并推送到 `main`；提交信息用中文、Conventional Commits 前缀（`feat:` / `fix:` / `docs:` …）；作者只署用户本人。

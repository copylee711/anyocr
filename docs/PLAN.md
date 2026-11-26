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
- 统一的 `OnlineEngine` 适配层负责限流、重试、超时、配额错误提示、进度回传。
- **预设更多在线引擎**，每个预设都带官方的 API Key 获取页链接（`key_url`），设置页提供“获取 API Key ↗”按钮，可直接跳转。链接在 M5 实现时逐个核实：

  | 引擎                    | 类型               | 获取 Key                                                 |
  | ----------------------- | ------------------ | -------------------------------------------------------- |
  | PaddleOCR（AI Studio）  | 文字行 / 版面 / VL | https://aistudio.baidu.com/account/accessToken           |
  | MinerU                  | 文档解析           | https://mineru.net/apiManage                             |
  | 百度智能云 OCR          | 通用文字（含位置） | https://console.bce.baidu.com/ai/#/ai/ocr/overview/index |
  | 腾讯云 OCR              | 通用印刷体         | https://console.cloud.tencent.com/cam/capi               |
  | 阿里云 OCR（读光）      | 通用文字           | https://ram.console.aliyun.com/manage/ak                 |
  | 讯飞 OCR                | 通用文字           | https://console.xfyun.cn/services                        |
  | 有道智云 OCR            | 通用文字           | https://ai.youdao.com/console/                           |
  | Google Cloud Vision     | 文字 / 文档        | https://console.cloud.google.com/apis/credentials        |
  | Azure AI Vision（Read） | 文字 / 文档        | https://portal.azure.com/                                |
  | OCR.space               | 免费额度           | https://ocr.space/ocrapi/freekey                         |
  | Mistral OCR             | 文档 OCR           | https://console.mistral.ai/api-keys                      |

  VLM 预设同样带链接：DeepSeek https://platform.deepseek.com/api_keys 、阿里云百炼（通义千问 VL）https://bailian.console.aliyun.com/ 、硅基流动 https://cloud.siliconflow.cn/account/ak 、智谱 https://open.bigmodel.cn/usercenter/apikeys 、火山方舟（豆包）https://console.volcengine.com/ark 、OpenAI https://platform.openai.com/api-keys 、Gemini https://aistudio.google.com/apikey 。

- **Fallback 链**：用户在设置页拖动排序，形成引擎优先级列表，例如“本地 → 百度 → 腾讯 → PaddleOCR”。某个引擎遇到以下情况时，自动切到下一个：未配置 Key、鉴权失败、配额用尽（如 Paddle 12001）、限流、超时、网络错误。每一页都记录最终实际使用的引擎，并在任务面板显示。同一任务里已触发配额错误的引擎，在本任务剩余部分直接跳过。
- 设置页：每个引擎一张卡片，包括启用开关、API Key（以及 Secret 等附加字段）、模型选择、“获取 API Key ↗”、“测试连接”按钮。

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

## 2.6 界面设计方向

- **仿 iOS / iPadOS**（用户 2026-11-25 确认）：系统蓝 `#007AFF`/`#0A84FF`、分组背景 `#F2F2F7`、毛玻璃侧栏、胶囊形选中态、Inset Grouped 设置列表、分段控件、SF 风格字体栈；Logo 为深色圆角方块 + 扫描框四角 + 字母 A + 扫描线（`assets/logo.svg`），标语 “Recognize anything. Own your data.”。
- 风格：现代、精致、克制，**冷色调**。主色在青蓝到靛蓝之间，中性色偏冷灰（slate）；明暗两套主题，默认跟随系统。
- 质感：大圆角卡片、细描边、轻阴影、毛玻璃侧栏（Windows 11 Mica / Acrylic 效果，`window-vibrancy`）、统一的 8px 间距网格、克制的过渡动画。
- 字体：界面用系统字体栈（Segoe UI Variable / 微软雅黑 / PingFang），识别结果用可配置的等宽或衬线字体。
- 布局：左侧图标导航 + 顶部可增删的功能标签页；任务类页面使用“文件列表 / 预览 / 结果”三栏，可拖动分隔。
- 流程：M3 动工前先出设计稿（色板、组件、关键页面）请用户确认，再铺开全部页面。

## 3. 里程碑

每个里程碑结束时都应可运行、可测试，并提交推送。

| 阶段                   | 内容                                                                                                                                                                   | 验收标准                                                                                   |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| **M0 脚手架**          | Cargo workspace + crates 骨架；Tauri 2 + Vue 3 + TS + Vite + pnpm；Naive UI、Pinia、vue-i18n；ESLint/Prettier、rustfmt/clippy；GitHub Actions（Windows：test + build） | `pnpm tauri dev` 能打开空壳窗口；`cargo test` 通过；CI 绿                                  |
| **M1 核心 + 本地引擎** | anyocr-core 数据模型；oar-ocr 接入；模型下载 / 管理；tbpu 解析器移植（附 Umi 用例对照测试）；`anyocr ocr <img>`                                                        | 测试图片识别正确；在 `常微分方程` 前 20 页上给出 CPU / DirectML 速度基准                   |
| **M2 PDF 文本层**      | anyocr-pdf：渲染、GlyphLess 字体、文字层写入、skip / force / redo、页范围、密码；`anyocr pdf`                                                                          | 4 页和 7 页的扫描件生成可搜索 PDF；提取比对通过；体积增量 < 2%；`应坚刚 概率论` 被正确跳过 |
| **M3 桌面主界面**      | 应用壳（标签页、主题、i18n、托盘、单实例、设置持久化）；批量文档页（含 PDF 文本层）；批量 OCR 页；全部输出格式                                                         | GUI 中完成整本 `常微分方程`（264 页）文本层，记录用时；输出格式与 Umi 样例一致             |
| **M4 截图与二维码**    | 截图选区窗口、全局快捷键、粘贴识别、结果叠加预览；二维码页                                                                                                             | 快捷键截图 → 识别 → 复制全流程可用；二维码读写通过                                         |
| **M5 在线引擎**        | 设置页 API Key（keyring）；PaddleOCR AI Studio、MinerU 适配；测试连接；Markdown 输出                                                                                   | 用用户的令牌跑通两份扫描件；MinerU 产出 Markdown（含公式）                                 |
| **M6 VLM**             | Provider 配置；模式 A（VLM OCR）；模式 B（并行复核流水线、截止时间、防幻觉、差异高亮）；计时面板                                                                       | 7 页扫描件：开启复核后总用时 ≤ 仅 OCR 用时 + 10 s；抽样修正准确率人工核查                  |
| **M7 兼容与发布**      | HTTP API、CLI 兼容层；忽略区域编辑器；任务完成后动作；NSIS 安装包；README 与使用文档                                                                                   | Umi-OCR 官方 API 示例脚本改端口即可运行；安装包在干净环境可用                              |

### 进度

- [x] **M0 脚手架**（2026-11-25）：Cargo workspace（core / engines / pdf / cli + src-tauri）、Tauri 2 + Vue 3 + Naive UI + Pinia + vue-i18n、仿 iOS 冷色调外壳与 Logo/图标、ESLint/Prettier/Vitest、rustfmt/clippy、GitHub Actions。
  - 待验证：真实窗口中的“打开图片 → 识别”点击流程（命令与 CLI 共用同一代码路径，CLI 已实测）；CI 首次运行结果。
- [x] **M1 核心 + 本地引擎**（2026-11-25）：统一数据模型；tbpu 全部 8 种排版解析的移植与 14 个单测；oar-ocr PP-OCRv6/v5 本地引擎（模型自动下载、SHA-256 校验、CPU/DirectML/CUDA）；PDFium 渲染；`anyocr ocr` / `anyocr bench`。基准见 [BENCHMARKS.md](BENCHMARKS.md)。
- [x] **M2 PDF 文本层**（2026-11-25）：GlyphLessFont 文字层（自生成 640 字节字体；Identity-H + ToUnicode，含 BMP 以外字符）、lopdf 增量更新（原文件字节原样保留）、加密文档解密重写、skip-text / force / redo、`anyocr pdf … --verify`、`scripts/acceptance.ps1`。端到端坐标测试覆盖 0/90/180/270 旋转、CropBox 偏移、非零原点 MediaBox、倾斜文字。
  - 发现并修复：空轮廓字形会导致 PDFium（Chrome / Edge）丢弃倾斜行的文字，字形改为不可见的整格矩形轮廓。
  - 后续：PDF/A 输出、纠偏 / 旋转页检测（文档方向分类模型已可用，待接入）、单层纯文字 PDF（pdfOneLayer）放到 M3 输出格式中实现。
- [x] **M3 桌面主界面（主体）**（2026-11-25）：
  - `anyocr-output`：txt / 纯文本 / 单独 txt / md / csv / jsonl，规则与 Umi-OCR 一致。
  - 设置持久化（自动保存）、本地引擎池、批量任务（文档、图片）及 `job` 事件、取消。
  - 文档 / PDF 页、批量图片页、设置页；主题（跟随系统 / 浅色 / 深色）与语言切换；托盘与单实例。
  - 任务流水线集成测试（使用假引擎）。
  - 未完成，挪到后续里程碑：
    - pdfOneLayer（单层纯文字 PDF）、忽略区域编辑器、任务完成后关机 / 休眠 → M7；
    - Umi 的“可增删标签页”改为固定侧栏导航（同一功能可同时开多个任务的需求待评估）；
    - 真实窗口中的拖放、开始和取消流程，需用户实测确认。
- [x] **M4 截图与二维码**（2026-11-25，用户 2026-11-26 实机确认）：
  - 多屏截图：截屏结果留在内存，经二进制 IPC 交给遮罩窗口用 canvas 绘制，省去编码和写盘；截屏本身约 40 ms。
  - 遮罩窗口：框选、选区外暗化、尺寸提示，Esc 或右键取消。
  - 粘贴识别、重复上次截图；识别后自动复制、弹出主窗口、截图前隐藏主窗口（均可设置）。
  - 全局快捷键：可录制，默认 F4 截图；注册失败会提示。
  - `anyocr-qrcode`：rxing 多码识别，预处理含中值滤波、锐度、对比度、阈值；生成支持 10 种码制、尺寸、静区和纠错等级，一维码自动保证静区。
  - 二维码页支持截图、粘贴、打开图片识别，以及生成后保存。
- [x] **M5 在线引擎**（2026-11-26，桌面端待用户实机确认）：
  - `anyocr-engines::online`：8 个逐页引擎（PaddleOCR AI Studio、百度、腾讯 TC3 签名、有道 v3 签名、Google Vision、Azure Read 4.0、OCR.space、Mistral OCR）和 MinerU 文档转 Markdown。HTTP 用 ureq，错误统一归类为 `EngineError`。
  - `ChainEngine` 实现 fallback 链：缺密钥、鉴权失败、额度用完的引擎在本任务里不再尝试；超时、限流、网络错误只换下一个引擎重试当前页；每页记录实际使用的引擎。
  - 实测（用户令牌，2026-11-26）：
    - PaddleOCR 当天不再排队，单张约 3 s。4 页扫描件以 4 并发处理 5.2 s，字数一致率 100%，搜索抽样 10/12；7 页扫描件搜索抽样 21/21。
    - MinerU（vlm）：4 页 16 s，7 页 33 s，产出 Markdown，公式为 LaTeX，图片单独导出。
    - OCR.space 用公开演示 Key 跑通。百度、腾讯、有道、Google、Azure、Mistral 只有单元测试（签名、返回解析、错误码分类），界面标注“未实测”。
  - 桌面端：
    - 密钥存进 Windows 凭据管理器（keyring），设置文件里只放模型、地域等非密钥字段。
    - 设置页新增“识别引擎顺序”（排序、增删、超时、并发）和“在线服务”卡片（密钥、参数、获取 API Key、测试连接，测试用内置样张）。
    - 截图、批量、PDF 都走引擎链。文档页新增“MinerU 转 Markdown”模式，显示排队和解析进度。
  - CLI：`--engine paddle_aistudio,local` 组成 fallback 链，密钥从环境变量 `ANYOCR_<ID>_<FIELD>` 读取。`anyocr engines` 列出全部引擎与变量名；`anyocr md` 调用 MinerU。
  - 未做，挪到后续：阿里云、讯飞（签名复杂且没有密钥可测）；用 MinerU 的块级坐标生成文字层（精度为块级，暂不做）。
  - 开发体验：dev 配置去掉依赖的调试信息，`target` 目录明显变小。需要时用 `cargo clean` 清理（2026-11-26 清掉 29.7 GB）。
- [x] **M6 视觉大模型**（2026-11-26，桌面端待用户实机确认）：
  - `anyocr-engines::vlm`：
    - OpenAI 兼容客户端，预设 DeepSeek、通义千问 VL、硅基流动、智谱、火山方舟、OpenAI、Gemini、Ollama、自定义。
    - 模式 A：`VlmEngine`，本地检测行框后由 VLM 转写。
    - 模式 B：`VlmReviewer`，只返回需要修改的行。
  - **关键发现**：`deepseek-flash` 默认先思考，一页复核要 12~40 s。传 `thinking: {type: disabled}` 后约 1~3 s，改正结果相同，所以预设默认关闭思考。
  - `anyocr-core::review`：
    - `ReviewPool`：复核线程与后续页的 OCR 并行；OCR 全部完成后最多再等 `grace`，超时的页保留原结果。
    - 防幻觉校验（`Guard`），以下修改会被拒绝：
      - 编辑距离超过阈值；
      - 引入 `$ \ _ ^ { }` 等排版标记；
      - 数字位数不变而数值变化，或新增了数字（形近字母 l/I/O 按数字计）；
      - 改成本页另一行的原文（行号错位）。错位占比高时整页修改作废。
    - 只有空白差异的修改忽略，不算改动。
  - 桌面端：
    - 设置页“视觉大模型”：服务、密钥（凭据管理器）、模型、地址、测试（用样张故意写错 2 处）。
    - 设置页“并行复核”：文档与批量开关、截图开关、截止等待、并发、只复核低置信度页。
    - 引擎链可加入“视觉大模型识别”。
    - 文档和批量任务的列表显示“复核修正 N 处”，并在输出旁生成 `.review.jsonl`。
    - 截图页先显示 OCR 结果，复核后就地更新。修正的行绿色高亮，悬停显示原文，可撤销。剪贴板内容没变时，自动换成修正后的文本。
  - CLI：
    - `anyocr pdf --review deepseek [--review-grace 10 --review-concurrency 4 --review-low-conf 0.9 --review-log x.jsonl]`。
    - `--engine vlm_deepseek` 用 VLM 识别。
  - 验收（7 页《光的等厚干涉》，本机 DirectML）：
    - 用时：仅 OCR 3.6~4.2 s，开启复核 6.9~7.5 s。尾部等待约 4 s，满足“≤ 仅 OCR + 10 s”。7 页全部按时复核，字数一致率 100%。
    - 人工核查（对照扫描页）：第三轮采纳 34 处，绝大多数正确，包括错字（代人→代入、人射→入射、牛頓→牛顿、画痕→划痕）和 OCR 丢失的下标（r→rk、rn→rm）。
    - 确认错误 1 处：“读数显微器→显微镜”，原书就是“显微器”。模型会把原书的用字改成常见写法，提示词约束无效。
    - 被规则拦下的错误：“10⁻²mm→10-3mm”、下标 xᵢ 被猜成 x1 / x10、表格行号错位。规则偏保守，部分正确的公式修正（如 Dm2-Dn2）也被拒绝。
    - 模式 A：2 页用时 9.6 s（逐页串行），适合少量高要求页面。
  - 未做：截图结果的差异逐字高亮（目前按行高亮）；只送低置信度行裁剪图的复核方式。
- [x] **Agent 集成**（2026-11-26，用户尚未在自己的 Claude Code / Codex 中实测）：
  - `anyocr mcp`：基于 rmcp 3 的 stdio MCP 服务，提供 5 个工具：
    - `ocr_image`：识别图片。
    - `ocr_pdf`：按页读取。有文字层的页直接提取，扫描页现场 OCR；单次最多 30 页，并按 `max_chars` 在页边界截断。
    - `search_pdf`：搜索时忽略空白和大小写，并说明哪些扫描页尚未识别。
    - `make_searchable_pdf`：后台任务，立即返回任务编号。
    - `job_status`：查询进度，给出预计剩余时间。
  - 扫描页文字缓存在 `%LOCALAPPDATA%\anyocr\pages\`，以路径、大小、修改时间为键；输出的 `.ocr.pdf` 也记一份。
  - `integrations/claude-plugin`：Claude Code 插件，含 `.mcp.json`、`skills/anyocr/SKILL.md`（何时使用，以及“先搜索定位、再按页读取”的流程）和图标。仓库根目录的 `.claude-plugin/marketplace.json` 让仓库本身可以作为插件市场添加。`claude plugin validate` 通过。
  - 接入文档：`docs/AGENTS-INTEGRATION.md`，覆盖 Claude Code、Codex 和其他 MCP 客户端。
  - 验收（用最小 MCP 客户端走 JSON-RPC stdio 完整流程，《常微分方程》264 页扫描件）：
    1. 首次 `search_pdf` 正确提示“264 页尚未识别”。
    2. `make_searchable_pdf` 在后台用 358 s 识别完全书，失败 0 页。
    3. 再搜“朗斯基行列式”，0.1 s 命中 17 处，首处在第 86 页。
    4. `ocr_pdf` 读取第 86 页，原文正确。再次读取命中缓存，用时 0.0 s。
  - 待做：发布阶段做 npm 包装（`npx anyocr-mcp`，按平台提供预编译二进制并带上 pdfium.dll）；在用户真实的 Claude Code / Codex 中试用。
- [x] **检查更新与发布流程**（2026-11-26，用户提出）：
  - Tauri updater 插件读取 `releases/latest/download/latest.json`，更新包用 minisign 签名校验。私钥在本机 `%USERPROFILE%\.tauri\`，不进仓库。
  - 设置页“更新”：显示当前版本，可手动检查，可开关“启动时自动检查”，可取消忽略。
  - 启动 4 s 后自动检查。有新版本时弹窗显示版本号、发布日期和更新说明，提供“立即更新”（显示下载进度，装完自动重启）、“稍后”和“忽略此版本”。忽略的版本不再自动弹窗，手动检查时仍会显示。
  - `release.yml`：推送 `v*` 标签后先检查三处版本号与标签一致，再用 tauri-action 发布安装包、更新包和 `latest.json`，上传命令行压缩包；配置了 `NPM_TOKEN` 时同时发布 npm 包。
  - CI 构建用 `tauri.nosign.conf.json` 关闭更新包，避免因缺少私钥失败。
  - npm 分发：
    - `anyocr` 是 JS 启动器，`@copylee/anyocr-win32-x64` 带 `anyocr.exe` 和 `pdfium.dll`（16 MB）。
    - 在干净目录从 tarball 安装后，`npx anyocr ocr` 和 `npx anyocr mcp`（stdio 干净，握手与调用正常）都已验证。
  - 待用户在 GitHub 仓库设置 Secrets 后才能走正式发布（见 `docs/RELEASE.md`）。
- [ ] M7 ← 下一步

### 新增里程碑：Agent 集成（用户 2026-11-25 提出，排在 M3 之后，可按需提前）

| 内容                                                                                                                                                                                                                                   | 验收标准                                                                    |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `anyocr mcp`：stdio MCP 服务（Rust `rmcp`），工具 `ocr_image`、`ocr_pdf`（页范围，返回带 `<!-- page N -->` 标记的文本 / Markdown）、`make_searchable_pdf`、`search_pdf`（关键词 → 页码与上下文）；大结果分页返回，避免撑爆智能体上下文 | Claude Code 与 Codex 配置后能调用，在《常微分方程》中按关键词定位并读取原文 |
| `SKILL.md` + Claude Code 插件清单：说明何时使用（扫描件、图片、无文字层 PDF）、推荐流程（先 search 定位页码，再按页取文本）                                                                                                            | 智能体在遇到扫描件时会主动调用                                              |
| 分发：GitHub Release 提供 exe；发布阶段再做 npm 包装（按平台的预编译二进制，`npx anyocr-mcp` 一键安装）                                                                                                                                | 干净环境一条命令完成安装与配置                                              |

## 4. 测试与验收

- 单元测试：tbpu 解析器（用 Umi-OCR 行为构造的固定输入 / 输出）、坐标变换、GlyphLess 字体与 ToUnicode 生成、差异合并与防幻觉规则。
- 集成测试：`tests/fixtures` 放少量可公开的小样张（自制渲染图片和小 PDF）；用户的课本扫描件只在本机验收时引用，**不进仓库**。
- 验收脚本：`scripts/acceptance.ps1` 依次跑 4 页、7 页、264 页三档，输出用时、页数、可搜索率、复核修改数，写成 Markdown 报告。
- 在线服务和 VLM 的测试只在显式设置环境变量时运行，默认跳过。环境变量：`ANYOCR_PADDLE_TOKEN`、`ANYOCR_MINERU_TOKEN`、`DEEPSEEK_API_KEY`。

## 5. 风险与对策

| 风险                                           | 对策                                                                              |
| ---------------------------------------------- | --------------------------------------------------------------------------------- |
| oar-ocr API 变动或 Windows 构建问题            | M1 先做 spike；以 `paddle-ocr-rs` 或自写 ort 推理（DB 后处理 + CTC 解码）作为备选 |
| pdfium.dll 分发                                | 用 bblanchon/pdfium-binaries 固定版本，作为 Tauri resource 打包，启动时校验       |
| 复杂 PDF（旋转页、CropBox 偏移、对象流、加密） | 坐标变换单测覆盖；失败页降级为跳过并记录日志，不中断整份文档                      |
| 在线服务排队慢 / 配额                          | 任务面板显示排队状态；超时后可回退到本地引擎                                      |
| VLM 幻觉或改错                                 | 只修改不新增行，编辑距离阈值，记录修改日志，可一键撤销                            |
| 数学公式识别差                                 | 文字层场景接受；需要 LaTeX 时走 MinerU 或 VLM 的 Markdown 输出                    |

## 6. 仓库与提交约定

- 本仓库 `copylee711/anyocr` 是 Umi-OCR 的重写版：不直接 fork 原仓库代码（技术栈不同），在 `NOTICE` 与关于页保留 Umi-OCR 的 MIT 版权声明。移植算法（如 tbpu）时，在文件头注明来源。
- 每完成一个可运行的步骤就提交并推送到 `main`；提交信息用中文、Conventional Commits 前缀（`feat:` / `fix:` / `docs:` …）；作者只署用户本人。

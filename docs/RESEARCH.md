# 调研记录

调研日期：2026-11-25。本文件只记录事实与结论，方案见 [PLAN.md](PLAN.md)。

## 1. Umi-OCR（被复刻对象）

- 仓库：https://github.com/hiroi-sora/Umi-OCR ，MIT，最后提交 2025-11-20（v2.1.5）。
- 架构：Python + PySide6/QML 界面；OCR 引擎以插件形式存在于另一个仓库（Umi-OCR_plugins：PaddleOCR-json / RapidOCR-json，C++ 进程，stdin/stdout JSON 通信）。
- 源码要点（`UmiOCR-data/py_src`）：
  - `ocr/tbpu/`：文本块后处理（排版解析）。解析器：`multi_para` 多栏按自然段换行、`multi_line` 多栏总是换行、`multi_none` 多栏无换行、`single_para`、`single_line`、`single_none`、`single_code` 单栏保留缩进、`none` 不处理；另有 `ignore_area` 忽略区域（模式：inside 优先内部 / onlyInside / onlyOutside）。核心算法在 `parser_tools/gap_tree.py`（间隙树分栏）、`line_preprocessing.py`、`paragraph_parse.py`。
  - `ocr/output/`：输出格式 `txt`、`txtPlain`、`txtIndividual`（每图单独 txt）、`md`、`csv`、`jsonl`、`pdfLayered`（双层可搜索 PDF）、`pdfOneLayer`（仅文字的单层 PDF）。
  - `output_pdf_layered.py`：用 PyMuPDF `page.insert_text(..., fill_opacity=0)` 写透明 CJK 字体文字，按框宽二分逼近字号，最后 `subset_fonts()`。缺点：嵌入真实 CJK 字体子集、体积偏大、只按字号适配宽度。
  - `mission/mission_doc.py`：文档识别，支持 `.pdf .xps .epub .mobi .fb2 .cbz`；最小渲染分辨率 1080；提取模式 `mixed` 混合OCR/原文本、`fullPage` 整页强制OCR、`imageOnly` 仅OCR图片、`textOnly` 仅拷贝原有文本；页范围、密码。
  - `server/`：HTTP 服务（bottle），默认 `127.0.0.1:1224`；接口 `/api/ocr`、`/api/ocr/get_options`、`/api/doc/*`（upload → result 轮询 → download → clear）、`/api/qrcode`。
  - CLI（`docs/README_CLI.md`）：`--show --hide --quit --reload --screenshot [screen= rect=] --clipboard --path ... --qrcode_read --qrcode_create --clip --output --output_append --all_pages --add_page --del_page` 等，支持前缀简写。
- 界面页面（`qt_res/qml/TabPages`）：截图OCR、批量OCR、批量文档、二维码、全局设置、导航页、关于。多标签页可增删。
- 各页配置项（摘自 `*Configs.qml`）：
  - 截图OCR：tbpu、快捷键（截图 / 粘贴 / 重复截图）、识别后动作（复制、弹出主窗口）。
  - 批量OCR：tbpu、忽略区域、递归子目录、输出目录（原目录 / 指定）、文件名格式 + 日期格式、输出类型、忽略空白、任务完成后动作（关机 / 休眠）。
  - 批量文档：同上 + 提取模式、页范围、输出 pdfLayered / pdfOneLayer / txt / txtPlain / csv / jsonl。
  - 二维码：快捷键、预处理（中值滤波、锐度、对比度、灰度、二值化阈值）、生成（格式、宽高、静区、纠错等级）。
  - 全局：快捷方式（桌面 / 开始菜单 / 开机自启）、主题、界面字体与数据字体、缩放、关闭硬件加速、窗口（启动隐藏、置顶、锁定标签栏、隐藏托盘）、截图前隐藏窗口及延时、HTTP 服务（开关 / 主机 / 端口）、日志、通知、多语言。

## 2. OCRmyPDF（文本层方案参考）

- 原理：渲染页面 → 预处理（旋转、纠偏）→ Tesseract → 生成文字层（hOCR/内置渲染器）→ 用 pikepdf/QPDF 把文字层叠加回原页面，原图像不重编码。
- 关键技巧：**GlyphLessFont**（源自 Tesseract PDF 渲染器）——一个只有一个空字形的极小字体，Type0/CIDFontType2 + `Identity-H` 编码，CID 直接等于 UTF-16 码元，`CIDToGIDMap` 全部映射到同一字形，`ToUnicode` CMap 负责还原文字；文字渲染模式 `3 Tr`（不可见）；每行用 `Tz` 水平缩放精确匹配框宽。好处：不嵌入真实字体、体积极小、选区与原图对齐。
- 行为选项：`--skip-text` 跳过已有文字页、`--force-ocr`、`--redo-ocr`、`--deskew`、`--rotate-pages`、`-O` 优化级别、PDF/A 输出。
- v17 起 Ghostscript 非必需，栅格化改用 pypdfium2。

## 3. 可复用的 Rust 生态（crates.io 实测版本）

| 用途                    | crate           | 版本   | 许可       | 备注                                                                                                        |
| ----------------------- | --------------- | ------ | ---------- | ----------------------------------------------------------------------------------------------------------- |
| 本地 OCR（PP-OCR ONNX） | `oar-ocr`       | 0.11.0 | Apache-2.0 | 基于 `ort`；支持 PP-OCRv5/v6、文档方向、文本行方向、版面分析；有 `cuda` feature；备选 `paddle-ocr-rs` 0.6.1 |
| ONNX Runtime            | `ort`           | 2.0 rc | MIT/Apache | Windows 可用 DirectML / CUDA EP                                                                             |
| PDF 渲染与读取          | `pdfium-render` | 0.9.6  | MIT/Apache | 需分发 `pdfium.dll`（bblanchon/pdfium-binaries）                                                            |
| PDF 底层写入            | `lopdf`         | 0.47.0 | MIT        | 用于追加文字层内容流、增量保存                                                                              |
| 截屏                    | `xcap`          | 0.9.10 | Apache-2.0 | 多屏截图                                                                                                    |
| 二维码 / 条码           | `rxing`         | 0.9.4  | Apache-2.0 | ZXing 的 Rust 移植，读写多种码制                                                                            |
| HTTP 服务               | `axum`          | 0.8.9  | MIT        | 兼容 Umi-OCR API                                                                                            |
| 桌面框架                | `tauri`         | 2.12.7 | MIT/Apache | 插件：global-shortcut、clipboard-manager、autostart、single-instance、dialog、fs、store、updater 等         |

注意：pdfium 只能打开 PDF。Umi-OCR 支持的 xps/epub/mobi/fb2 依赖 MuPDF（AGPL），本项目不引入；cbz（图片压缩包）可自行解包支持。

## 4. 同类项目（界面 / 交互参考）

- [pot-desktop](https://github.com/pot-app/pot-desktop)：Tauri 的翻译 + OCR 应用，截图选区窗口、托盘、全局快捷键、本地 HTTP 接口、插件化识别服务，最值得参考。
- [TrayOCR](https://github.com/ospoon/trayocr)：Tauri 2 + Vue 3，调用系统 OCR（Windows.Media.Ocr / macOS Vision）。
- [screen_inu](https://github.com/iml1s/screen_inu)：Tauri，冻结屏幕后框选识别。
- [pdf-ocrer](https://github.com/leon80148/pdf-ocrer)：PP-OCR + 双层 PDF + 跳过已有文字页，Windows 优先。
- [pdf-textlayer](https://github.com/patelvivekdev/pdf-textlayer)：OCR 与文字层解耦，统一 JSON（文字 + 框 + 置信度）→ 文字层。本项目采用同样的解耦思路。
- [DeepOCR](https://github.com/deeplabua/deepocr)：纯 Rust 的 OCRmyPDF 替代品（ocrs 引擎），可参考其 PDF 写入实现。
- [BigOcrPDF](https://github.com/biglinux/bigocrpdf)：透视校正、去噪、PDF/A 等预处理思路。
- [markitdown](https://github.com/microsoft/markitdown)（微软，Python，MIT）：把 docx / pptx / xlsx / pdf / html 等转成面向大模型的 Markdown。与 OCR 核心和文本层无直接关系，可参考两点：一是 Markdown 输出的结构约定（标题、列表、表格）；二是后续支持 Office 文档输入，即提取 docx / pptx 中嵌入的图片做 OCR，再与原有文字合并。不作为依赖。
- [oar-ocr](https://github.com/greatv/oar-ocr)（Apache-2.0）：已选为本地引擎。支持 PP-OCRv6 tiny / small / medium（2026 新一代）以及 PP-StructureV3、PaddleOCR-VL 原生推理（`oar-ocr-vl`，基于 Candle），后续可作为离线 VLM 选项。

## 5. 在线 OCR 服务（已用用户密钥实测连通）

### PaddleOCR（百度 AI Studio）

- 令牌：https://aistudio.baidu.com/account/accessToken
- 异步任务接口：`POST https://paddleocr.aistudio-app.com/api/v2/ocr/jobs`，请求头 `Authorization: bearer <token>`；multipart 字段 `file` / `model` / `optionalPayload`(JSON 字符串)，或 JSON `fileUrl`；可选 `pageRanges`、`batchId`。
- 模型：`PP-OCRv5`、`PP-StructureV3`、`PaddleOCR-VL`、`PaddleOCR-VL-1.5`。
- 轮询：`GET /api/v2/ocr/jobs/{jobId}`，`data.state` ∈ pending/running/done/failed；完成后 `data.resultUrl.jsonUrl`（JSONL，每行一页的 `result`）和 `markdownUrl`。
- 限制：单次最多 1000 页 PDF，URL 文件 200 MB，本地上传 50 MB，有每日页数配额（错误码 12001）、限流 12002。
- 同步接口（`/ocr`、`/layout-parsing`）的 URL 需要在 https://aistudio.baidu.com/paddleocr/task 控制台复制，头为 `Authorization: token <token>`。
- 实测（2026-11-25）：提交成功（code 0），但一张小图排队超过 10 分钟仍是 `pending`。不适合截图即时识别；批量文档使用时需显示排队状态，并设置超时后 fallback。

### MinerU（mineru.net 精准解析 API v4）

- 头 `Authorization: Bearer <token>`。
- 本地文件：`POST /api/v4/file-urls/batch`（`files[{name,data_id,is_ocr,page_ranges}]`、`model_version` = pipeline/vlm）→ 对返回的预签名 URL 做 `PUT` 上传（不带 Content-Type）→ 轮询 `GET /api/v4/extract-results/batch/{batch_id}` → 下载 `full_zip_url`。
- URL 文件：`POST /api/v4/extract/task`，轮询 `GET /api/v4/extract/task/{task_id}`。
- 结果 zip：`full.md`、`*_content_list.json`、`layout.json`（middle.json，含块级 bbox）、`*_model.json`；可选 `extra_formats` docx/html/latex。
- 限制：单文件 200 MB / 600 页，单批 200 个文件，每日 2000 页高优先级。
- 定位：版面 / 公式（LaTeX）/ 表格解析，产出 Markdown；块级坐标可用于文字层，但精度为块而非行。
- 实测：令牌有效（`file-urls/batch` 返回 code 0）。

## 6. 视觉大模型

- DeepSeek：`GET https://api.deepseek.com/models` 返回 `deepseek-flash`、`deepseek-v4-pro`。
  - `deepseek-flash` 支持图片输入（OpenAI 兼容 `image_url`，base64 data URL）；实测一张小图中英混排识别正确，耗时约 1.6 s。
  - `deepseek-v4-pro` 不接受图片。
- 统一按 OpenAI Chat Completions 兼容协议接入，可覆盖 DeepSeek、通义千问 VL（DashScope 兼容模式）、硅基流动、智谱、OpenAI、Gemini 兼容端点、本地 Ollama / LM Studio。
- 注意：环境变量更新后，已启动进程拿到的仍是旧值；开发测试时要从用户级环境变量读取，或重启终端。

## 7. 测试素材（`D:\MyFiles\大学课程\大二上`）

已确认是纯扫描件（前 5 页无文字层，每页一张图）：

| 文件                                                     | 页数 | 用途                                 |
| -------------------------------------------------------- | ---- | ------------------------------------ |
| 大学物理实验\实验5.3B 用拉伸法测杨氏模量\…课本扫描件.pdf | 4    | 冒烟测试                             |
| 大学物理实验\实验3.19 光的等厚干涉\…课本扫描件.pdf       | 7    | 冒烟测试（每页 2 图）                |
| 常微分方程\常微分方程（第四版）王高雄.pdf                | 264  | 整本验收，公式多                     |
| 数学分析三\《数学分析 第4版 下册》复旦.pdf               | 294  | 整本验收                             |
| 离散数学\离散数学（第六版）耿素云.pdf                    | 273  | 整本验收                             |
| 概率论\课外参考书合集\应坚刚 概率论.pdf                  | 206  | 已有文字层，用于测试“跳过已有文字页” |

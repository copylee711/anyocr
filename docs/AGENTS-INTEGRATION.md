# 让 AI 智能体使用 AnyOCR（MCP / Skill）

`anyocr mcp` 以 stdio MCP 服务运行。Claude Code、Codex、Cursor 等支持 MCP 的智能体接入后，可以识别图片、读取和搜索扫描件 PDF。识别全部在本机完成，文件不会上传。

## 1. 安装命令行

暂未发布安装包时，从源码安装（需要 Rust 1.95+）：

```powershell
git clone https://github.com/copylee711/anyocr; cd anyocr
pwsh scripts/fetch-pdfium.ps1
cargo install --path crates/anyocr-cli
```

`anyocr` 运行时需要 PDFium 动态库（Windows 上是 `pdfium.dll`），按以下顺序查找：

1. 环境变量 `ANYOCR_PDFIUM_DIR` 指定的目录；
2. `anyocr.exe` 所在目录；
3. 源码仓库的 `vendor/pdfium/`。

用 `cargo install` 安装后，把 `vendor/pdfium/<平台>/bin/pdfium.dll` 复制到 `%USERPROFILE%\.cargo\bin\`，或者设置 `ANYOCR_PDFIUM_DIR` 指向它所在的目录。

首次识别时会自动下载本地模型（约 100 MB）到 `%APPDATA%\anyocr\models`。

## 2. 接入智能体

### Claude Code

方式一，安装插件（MCP 服务和 Skill 一起装好）：

```text
/plugin marketplace add copylee711/anyocr
/plugin install anyocr@anyocr
```

方式二，只添加 MCP 服务：

```powershell
claude mcp add anyocr -- anyocr mcp
```

Skill 位于 `integrations/claude-plugin/skills/anyocr/SKILL.md`。它告诉智能体什么时候用 AnyOCR，以及长文档“先搜索定位、再按页读取”的流程。

### Codex

```powershell
codex mcp add anyocr -- anyocr mcp
```

也可以直接写进 `~/.codex/config.toml`：

```toml
[mcp_servers.anyocr]
command = "anyocr"
args = ["mcp"]
# 识别整本书时单次调用可能较久；make_searchable_pdf 本身会立即返回
tool_timeout_sec = 120
```

### 其他客户端

凡是支持 stdio MCP 的客户端，配置命令 `anyocr`、参数 `["mcp"]` 即可。

常用参数：

- `anyocr mcp --device cpu`：不用显卡。
- `anyocr mcp --model pp-ocrv6-medium`：使用更准、更慢的模型。
- `anyocr mcp --engine paddle_aistudio,local`：先用在线引擎，失败时回退本地。密钥从环境变量读取，变量名见 `anyocr engines`。

## 3. 工具

| 工具                  | 参数                                                                  | 说明                                                                                       |
| --------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `ocr_image`           | `path`、`parser`                                                      | 识别图片                                                                                   |
| `ocr_pdf`             | `path`、`pages`（默认 `1-10`）、`max_chars`（默认 20000）、`password` | 按页读取，每页以 `<!-- page N (文字层/OCR) -->` 开头；单次最多 30 页，超出时提示从哪页继续 |
| `search_pdf`          | `path`、`query`、`max_results`、`context`                             | 搜索文字层和已识别的扫描页，忽略空白和大小写；会说明哪些扫描页尚未识别                     |
| `make_searchable_pdf` | `path`、`output`、`pages`、`password`                                 | 后台识别并生成带文字层的 PDF，立即返回任务编号                                             |
| `job_status`          | `job_id`                                                              | 查询后台任务进度和预计剩余时间                                                             |

扫描页的识别结果缓存在 `%LOCALAPPDATA%\anyocr\pages\`，以文件路径、大小和修改时间为键。同一文件再次读取或搜索时不会重复识别。

## 4. 验收记录

2026-11-26，用一个最小 MCP 客户端跑完整流程。测试书是《常微分方程（第四版）》扫描件，264 页，没有文字层。完整数据见 `docs/PLAN.md` 的 Agent 集成一节。

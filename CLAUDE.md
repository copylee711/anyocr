# CLAUDE.md

本仓库的通用约定写在 AGENTS.md，这里通过导入引用，不再重复：

@AGENTS.md

## Claude Code 专用补充

- 回复用户一律使用中文。
- 开始一项工作前先读 `docs/PLAN.md`，确认当前所处的里程碑和验收标准；完成后在 PLAN.md 中勾选或更新进度。
- 查询第三方库（Tauri、Vue、oar-ocr、ort、pdfium-render、lopdf、axum 等）的用法时，优先用 Context7 获取当前文档，不要凭记忆写 API。
- Windows 环境：默认 shell 是 PowerShell 7。用户级环境变量更新后，当前进程里可能还是旧值，需要用 `[Environment]::GetEnvironmentVariable('NAME','User')` 读取。
- 本机验收素材位于 `D:\MyFiles\大学课程\大二上`（只读，不得修改，也不得复制进仓库），清单见 `docs/RESEARCH.md` 第 7 节。
- 提交与推送时机由代理自行决定，不必询问用户；提交不加任何 AI 署名或 Co-Authored-By。

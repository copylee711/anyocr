# 发布流程

## 一次性准备

1. **更新签名密钥**：已在本机生成，位于 `%USERPROFILE%\.tauri\`（不在仓库里）：
   - `anyocr.key`：私钥。丢失后，已安装的版本将无法再接收更新，请另外备份。
   - `anyocr.key.password`：私钥密码。
   - `anyocr.key.pub`：公钥，已写入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。
2. **在 GitHub 仓库里设置 Secrets**（Settings → Secrets and variables → Actions）。在仓库根目录用 `gh` 设置：

   ```powershell
   Get-Content "$env:USERPROFILE\.tauri\anyocr.key" -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY
   Get-Content "$env:USERPROFILE\.tauri\anyocr.key.password" -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
   # 可选：在 CI 中自动发布 npm 包（npmjs.com → Access Tokens → Granular，授予 anyocr 与 @copylee 的发布权限）
   gh secret set NPM_TOKEN
   ```

## 每次发布

1. 同步修改三处版本号：`package.json`、`src-tauri/tauri.conf.json`、`Cargo.toml` 的 `[workspace.package]`。
2. 提交并打标签：

   ```powershell
   git commit -am "chore: 发布 v0.2.0"
   git tag v0.2.0
   git push; git push --tags
   ```

3. `.github/workflows/release.yml` 会自动完成：
   - 构建 NSIS 安装包和签名的更新包，生成 `latest.json`，创建 GitHub Release；
   - 构建命令行，上传 `anyocr-cli-<版本>-win-x64.zip`（含 `anyocr.exe`、`pdfium.dll`）；
   - 如果配置了 `NPM_TOKEN`，发布 npm 包 `anyocr` 和 `@copylee/anyocr-win32-x64`。

已安装的应用启动时会读取 `https://github.com/copylee711/anyocr/releases/latest/download/latest.json`，发现新版本就弹窗提醒，用户可以一键更新。

## 本机手动发布 npm

```powershell
cargo build --release -p anyocr-cli
pwsh scripts/pack-npm.ps1             # 组装到 target/npm 并检查包内容
pwsh scripts/pack-npm.ps1 -Publish    # 发布（需要已 npm login）
```

## 本机构建安装包

没有签名私钥时，不生成更新包：

```powershell
pnpm tauri build --config src-tauri/tauri.nosign.conf.json
```

有私钥时，先设置环境变量 `TAURI_SIGNING_PRIVATE_KEY`（私钥文件内容或路径）和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，再运行 `pnpm tauri build`。

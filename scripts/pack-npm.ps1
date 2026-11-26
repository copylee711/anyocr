# 组装 npm 包：把 release 版 anyocr.exe 和 pdfium.dll 放进平台包，同步版本号；可选发布。
#
#   pwsh scripts/pack-npm.ps1                    # 只组装并检查包内容，产物在 target/npm/
#   pwsh scripts/pack-npm.ps1 -Publish           # 组装后发布（需要已登录 npm）
#   pwsh scripts/pack-npm.ps1 -Version 0.2.0     # 指定版本，默认取 Cargo 工作区版本
#
# 先执行：cargo build --release -p anyocr-cli；pwsh scripts/fetch-pdfium.ps1
param(
    [string]$Version = "",
    [switch]$Publish
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $Version) {
    $m = Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1
    $Version = $m.Matches[0].Groups[1].Value
}
$exe = Join-Path $root "target/release/anyocr.exe"
$dll = Join-Path $root "vendor/pdfium/win-x64/pdfium.dll"
foreach ($f in @($exe, $dll)) {
    if (-not (Test-Path $f)) { throw "缺少 $f，请先编译 release 版 CLI 并下载 PDFium" }
}

$out = Join-Path $root "target/npm"
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
Copy-Item -Recurse npm $out

# 平台包：二进制 + PDFium + 第三方许可
$plat = Join-Path $out "anyocr-win32-x64"
Copy-Item $exe, $dll $plat
$pdfiumVer = (Get-Content vendor/pdfium/win-x64/VERSION -ErrorAction SilentlyContinue) -join " "
$pdfiumLicense = Get-Content vendor/pdfium/win-x64/LICENSE -Raw
@"
# 第三方组件

- **PDFium**（pdfium.dll）：来自 https://github.com/bblanchon/pdfium-binaries（$pdfiumVer），BSD-3-Clause / Apache-2.0，许可原文附后。
- **ONNX Runtime**：静态链接进 anyocr.exe，MIT。
- **PaddleOCR 模型**：首次运行时下载，不随本包分发，Apache-2.0。
- **Umi-OCR**：部分算法移植自 https://github.com/hiroi-sora/Umi-OCR，MIT。

---

$pdfiumLicense
"@ | Set-Content -Encoding utf8 (Join-Path $plat "THIRD_PARTY.md")
Copy-Item README.md (Join-Path $out "anyocr/README.md")

# 同步版本号
foreach ($name in @("anyocr", "anyocr-win32-x64")) {
    $file = Join-Path $out "$name/package.json"
    $j = Get-Content $file -Raw | ConvertFrom-Json
    $j.version = $Version
    if ($j.optionalDependencies) {
        foreach ($k in @($j.optionalDependencies.PSObject.Properties.Name)) { $j.optionalDependencies.$k = $Version }
    }
    $j | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8 $file
}
Write-Host "组装完成：$out（版本 $Version）"

# 在仓库外执行 npm：仓库 package.json 声明了 packageManager=pnpm，目录树内的 npm 会被 Corepack 拦截
Push-Location ([System.IO.Path]::GetTempPath())
try {
    foreach ($name in @("anyocr-win32-x64", "anyocr")) {
        $dir = Join-Path $out $name
        npm pack $dir --dry-run 2>&1 | ForEach-Object { "$_" } | Select-String "name:|version:|package size|unpacked size|total files"
        if ($Publish) {
            npm publish $dir --access public
            if ($LASTEXITCODE -ne 0) { throw "发布 $name 失败" }
        }
    }
} finally {
    Pop-Location
}

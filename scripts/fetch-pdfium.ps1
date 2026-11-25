# 下载 PDFium 预编译库（bblanchon/pdfium-binaries）到 vendor/pdfium/<平台>/。
# 用法：pwsh scripts/fetch-pdfium.ps1 [-Version chromium/7543] [-Platform win-x64]
param(
    [string]$Version = "chromium/8179",
    [string]$Platform = "win-x64"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "vendor/pdfium/$Platform"
$url = if ($Version -eq "latest") {
    "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-$Platform.tgz"
} else {
    "https://github.com/bblanchon/pdfium-binaries/releases/download/$Version/pdfium-$Platform.tgz"
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) "pdfium-$([guid]::NewGuid()).tgz"
Write-Host "下载 $url"
Invoke-WebRequest $url -OutFile $tmp
$extract = Join-Path ([IO.Path]::GetTempPath()) "pdfium-$([guid]::NewGuid())"
New-Item -ItemType Directory -Force $extract | Out-Null
tar -xzf $tmp -C $extract
New-Item -ItemType Directory -Force $dest | Out-Null
$lib = Get-ChildItem -Recurse $extract -Include pdfium.dll, libpdfium.so, libpdfium.dylib | Select-Object -First 1
if (-not $lib) { throw "压缩包中未找到 PDFium 动态库" }
Copy-Item $lib.FullName $dest -Force
Get-Content (Join-Path $extract "VERSION") -ErrorAction SilentlyContinue | Set-Content (Join-Path $dest "VERSION")
Copy-Item (Join-Path $extract "LICENSE") (Join-Path $dest "LICENSE") -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $tmp, $extract
Write-Host "已安装到 $dest"

# PDF 文本层验收：对本机课本扫描件生成可搜索 PDF 并校验，输出 Markdown 汇总。
# 素材只在本机使用，不进仓库。用法：
#   pwsh scripts/acceptance.ps1 [-Root "D:\MyFiles\大学课程\大二上"] [-Out acceptance-out] [-Full]
# 默认只跑小样本（4 页、7 页）；加 -Full 再跑整本《常微分方程》（264 页）。
param(
    [string]$Root = "D:\MyFiles\大学课程\大二上",
    [string]$Out = "acceptance-out",
    [switch]$Full
)
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    cargo build --release -p anyocr-cli
    if ($LASTEXITCODE) { throw "构建失败" }
    $exe = Join-Path $repo "target/release/anyocr.exe"
    New-Item -ItemType Directory -Force $Out | Out-Null

    $cases = @(
        @{ Name = "杨氏模量（4 页）"; Path = "大学物理实验\实验5.3B 用拉伸法测杨氏模量\实验5.3B 用拉伸法测杨氏模量课本扫描件.pdf" },
        @{ Name = "等厚干涉（7 页）"; Path = "大学物理实验\实验3.19 光的等厚干涉\实验3.19 光的等厚干涉课本扫描件.pdf" },
        @{ Name = "应坚刚 概率论（已有文字，应全部跳过）"; Path = "概率论\课外参考书合集\应坚刚 概率论.pdf"; Args = @("--pages", "1-20") }
    )
    if ($Full) {
        $cases += @{ Name = "常微分方程（264 页）"; Path = "常微分方程\常微分方程 （第四版） (王高雄,周之铭,朱思铭,王寿松) .pdf" }
    }

    $rows = @()
    $i = 0
    foreach ($c in $cases) {
        $i++
        $src = Join-Path $Root $c.Path
        $dst = Join-Path $Out "case$i.pdf"
        $rep = Join-Path $Out "case$i.json"
        Write-Host "==> $($c.Name)"
        $log = & $exe pdf $src $dst --verify --report $rep @($c.Args) 2>$null
        $ok = $LASTEXITCODE -eq 0
        $summary = ($log | Select-String "^完成").Line
        $check = ($log | Select-String "^校验").Line
        $rows += "| $($c.Name) | $(if ($ok) { '通过' } else { '失败' }) | $summary | $check |"
    }
    $md = @("# PDF 文本层验收（$(Get-Date -Format 'yyyy-MM-dd HH:mm')）", "", "| 样本 | 结果 | 处理 | 校验 |", "|---|---|---|---|") + $rows
    $md | Set-Content -Encoding utf8 (Join-Path $Out "summary.md")
    $md | Write-Host
}
finally {
    Pop-Location
}

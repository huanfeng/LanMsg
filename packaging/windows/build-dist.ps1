#Requires -Version 5.1
<#
.SYNOPSIS
    Build the Windows installer and the scoop zip for LanMsg.
.DESCRIPTION
    cargo build -> target/dist/windows-<arch>/ -> lanmsg-windows-<arch>.zip (scoop)
    -> temporary manifest from app.toml -> wind-packer build.
.PARAMETER Arch
    x64 or arm64.
.PARAMETER Version
    Version string; defaults to the workspace version in Cargo.toml.
.PARAMETER WindInstallerDir
    Local wind-installer checkout whose target/release holds wind-installer.exe,
    wind-uninstaller.exe and wind-packer.exe (development). Without it the tools are
    downloaded from the wind-installer release $WindInstallerTag.
.PARAMETER WindInstallerTag
    wind-installer release tag to download (CI). Must contain the firewall and sendto features.
.PARAMETER SkipBuild
    Reuse existing release binaries.
#>
param(
    [Parameter(Mandatory = $true)][ValidateSet("x64", "arm64")][string]$Arch,
    [string]$Version = "",
    [string]$WindInstallerDir = "",
    [string]$WindInstallerTag = "",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

# Pinned in CI by the release workflow; overridable with -WindInstallerTag.
$WindInstallerRepo = "huanfeng/wind-installer"

function Write-Step { param([string]$m) Write-Host ">>> $m" }
function Fail { param([string]$m) Write-Host "ERROR: $m"; exit 1 }

$Root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$Dist = Join-Path $Root "target\dist"
$Stage = Join-Path $Dist "windows-$Arch"
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }

if ($Version -eq "") {
    $cargoToml = Get-Content (Join-Path $Root "Cargo.toml") -Raw
    if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') { Fail "cannot read workspace version from Cargo.toml" }
    $Version = $Matches[1]
}
$Version = $Version.TrimStart("v")
if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z.+-]*$') { Fail "invalid version: $Version" }

$triple = if ($Arch -eq "x64") { "x86_64-pc-windows-msvc" } else { "aarch64-pc-windows-msvc" }

# --- wind-installer tools ---
$tools = Join-Path $Dist "wind-installer-tools"
if ($WindInstallerDir -ne "") {
    $rel = Join-Path $WindInstallerDir "target\release"
    $Stub = Join-Path $rel "wind-installer.exe"
    $Uninstaller = Join-Path $rel "wind-uninstaller.exe"
    $Packer = Join-Path $rel "wind-packer.exe"
} else {
    if ($WindInstallerTag -eq "") { Fail "give -WindInstallerDir (local build) or -WindInstallerTag (release download)" }
    New-Item -ItemType Directory -Force -Path $tools | Out-Null
    $base = "https://github.com/$WindInstallerRepo/releases/download/$WindInstallerTag"
    Write-Step "Downloading wind-installer $WindInstallerTag"
    # Only an x64 stub is published; it also runs on arm64 Windows through emulation.
    $assets = @("wind-installer-windows-x64.exe", "wind-uninstaller-windows-x64.exe", "wind-packer-windows-x64.exe")
    Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile (Join-Path $tools "SHA256SUMS")
    $sums = Get-Content (Join-Path $tools "SHA256SUMS")
    foreach ($a in $assets) {
        $dest = Join-Path $tools $a
        Invoke-WebRequest -Uri "$base/$a" -OutFile $dest
        $line = $sums | Where-Object { $_ -match "\s\*?$([regex]::Escape($a))$" } | Select-Object -First 1
        if (-not $line) { Fail "$a is not listed in SHA256SUMS" }
        $expected = ($line -split "\s+")[0].ToLower()
        $actual = (Get-FileHash -Algorithm SHA256 $dest).Hash.ToLower()
        if ($expected -ne $actual) { Fail "checksum mismatch for $a" }
    }
    $Stub = Join-Path $tools "wind-installer-windows-x64.exe"
    $Uninstaller = Join-Path $tools "wind-uninstaller-windows-x64.exe"
    $Packer = Join-Path $tools "wind-packer-windows-x64.exe"
}
foreach ($f in @($Stub, $Uninstaller, $Packer)) {
    if (-not (Test-Path $f)) { Fail "missing wind-installer tool: $f" }
}

# --- build ---
if (-not $SkipBuild) {
    Write-Step "Building LanMsg ($triple)"
    Push-Location $Root
    try {
        cargo build --release --target $triple -p lanmsgd -p lanmsg-desktop -p lanmsg-cli -p lanmsg-media
        if ($LASTEXITCODE -ne 0) { Fail "cargo build failed" }
    } finally { Pop-Location }
}

# --- stage ---
Write-Step "Staging $Stage"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
$binDir = Join-Path $TargetDir "$triple\release"
foreach ($b in @("lanmsg.exe", "lanmsgd.exe", "lanmsg-cli.exe", "lanmsg-thumb.exe")) {
    $src = Join-Path $binDir $b
    if (-not (Test-Path $src)) { Fail "missing binary: $src" }
    Copy-Item $src $Stage
}
Copy-Item (Join-Path $Root "LICENSE") $Stage

# --- scoop zip (made before the uninstaller is injected) ---
$zip = Join-Path $Dist "lanmsg-windows-$Arch.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $zip
Write-Step "Built $zip"

# --- temporary manifest next to app.toml (relative paths in it resolve from there) ---
$manifest = Join-Path $PSScriptRoot "app.generated.toml"
(Get-Content (Join-Path $PSScriptRoot "app.toml") -Raw).Replace("@VERSION@", $Version).Replace("@ARCH@", $Arch) |
    Set-Content -Encoding UTF8 -NoNewline $manifest

$uninstallDest = Join-Path $Stage "uninstall.exe"
try {
    Copy-Item $Uninstaller $uninstallDest -Force
    Write-Step "Preparing uninstaller"
    & $Packer prep-uninstaller --config $manifest
    if ($LASTEXITCODE -ne 0) { Fail "prep-uninstaller failed" }
    Write-Step "Packing installer"
    & $Packer build --config $manifest --stub $Stub
    if ($LASTEXITCODE -ne 0) { Fail "wind-packer build failed" }
} finally {
    Remove-Item -Force -ErrorAction SilentlyContinue $uninstallDest
    Remove-Item -Force -ErrorAction SilentlyContinue $manifest
}
Write-Step "Done: $Dist"

#!/bin/bash
# 打包相关的静态与可在 Linux 上跑的检查（CI ubuntu 运行）。
# 用法：bash scripts/check-packaging.sh [--fast]   --fast 跳过耗时的 Linux 安装端到端测试
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM
fail() { echo "FAIL: $*" >&2; exit 1; }

fast=0
[ "${1:-}" = "--fast" ] && fast=1

echo "== shellcheck"
shellcheck packaging/linux/*.sh packaging/macos/*.sh packaging/scoop/*.sh scripts/check-packaging.sh

echo "== windows app.toml"
for arch in x64 arm64; do
    sed -e 's|@VERSION@|1.2.3|g' -e "s|@ARCH@|$arch|g" packaging/windows/app.toml > "$tmp/app-$arch.toml"
    if grep -q '@' "$tmp/app-$arch.toml"; then fail "placeholder left in app.toml ($arch)"; fi
    python3 -I - "$tmp/app-$arch.toml" "$arch" <<'PY'
import sys, tomllib
d = tomllib.load(open(sys.argv[1], "rb"))
arch = sys.argv[2]
assert d["app"]["version"] == "1.2.3"
assert d["app"]["main_exe"] == "lanmsg.exe"
assert "url_protocol" not in d["app"]
assert d["autostart"]["exe"] == "lanmsgd.exe"
assert d["firewall"][0]["program"] == "lanmsgd.exe"
assert any(s["location"] == "sendto" and s["args"] == "--station-put" for s in d["shortcut"])
assert d["package"]["source_dir"].endswith("windows-" + arch)
assert d["paths"]["data"] == r"%APPDATA%\lanmsg\LanMsg\data"
assert "localdata" not in d
assert d["package"]["output_name"] == "lanmsg-windows-%s-setup" % arch
PY
done
# 清单引用的图标文件必须存在（路径相对 app.toml 所在目录）
for rel in ../assets/lanmsg-256.png ../../crates/lanmsg-desktop/assets/lanmsg.ico; do
    [ -f "packaging/windows/$rel" ] || fail "missing asset: $rel"
done

echo "== build-dist.ps1 placeholders"
# 脚本替换的占位符必须是模板里实际出现的
grep -q '@VERSION@' packaging/windows/app.toml || fail "app.toml lacks @VERSION@"
grep -q '@ARCH@' packaging/windows/app.toml || fail "app.toml lacks @ARCH@"
grep -q 'Replace("@VERSION@"' packaging/windows/build-dist.ps1 || fail "build-dist.ps1 does not replace @VERSION@"
if command -v pwsh >/dev/null 2>&1; then
    pwsh -NoProfile -Command '[void][ScriptBlock]::Create((Get-Content -Raw packaging/windows/build-dist.ps1))' \
        || fail "build-dist.ps1 syntax error"
else
    echo "pwsh not found; skipping PowerShell syntax check"
fi

echo "== scoop manifest"
jq . packaging/scoop/lanmsg.json >/dev/null
h64=$(printf 'a%.0s' $(seq 64))
harm=$(printf 'B%.0s' $(seq 64))
printf '%s  lanmsg-windows-x64.zip\n%s *lanmsg-windows-arm64.zip\n' "$h64" "$harm" > "$tmp/SHA256SUMS"
OUT_DIR="$tmp/out" bash packaging/scoop/update-manifest.sh v1.2.3 "$tmp/SHA256SUMS" >/dev/null
if grep -q '@' "$tmp/out/lanmsg.json"; then fail "placeholder left in lanmsg.json"; fi
jq -e --arg h64 "$h64" --arg harm "$(echo "$harm" | tr 'B' 'b')" '
    .version == "1.2.3"
    and .architecture."64bit".hash == $h64
    and .architecture.arm64.hash == $harm
    and .autoupdate.hash.url == "$baseurl/SHA256SUMS"
    and (.architecture."64bit".url | endswith("/v1.2.3/lanmsg-windows-x64.zip"))' \
    "$tmp/out/lanmsg.json" >/dev/null || fail "lanmsg.json content wrong"
printf '%s  lanmsg-windows-x64.zip\n' "$h64" > "$tmp/bad"
if OUT_DIR="$tmp/out2" bash packaging/scoop/update-manifest.sh 1.2.3 "$tmp/bad" >/dev/null 2>&1; then
    fail "missing arm64 hash accepted"
fi

echo "== macOS dry run"
bash packaging/macos/test-build-dmg.sh >/dev/null

if [ "$fast" = 0 ]; then
    echo "== linux install"
    sh packaging/linux/test-install.sh >/dev/null
fi

echo "OK"

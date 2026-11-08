#!/bin/bash
# build-dmg.sh 的 Linux 侧测试：参数解析、plist、目录布局、签名开关（均为 --dry-run）。
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM
fail() { echo "FAIL: $*" >&2; exit 1; }

export OUT_DIR="$tmp/dist"
unset APPLE_DEVELOPER_ID APPLE_ID APPLE_TEAM_ID APPLE_APP_PASSWORD

echo "== argument parsing"
if bash "$here/build-dmg.sh" >/dev/null 2>&1; then fail "no args accepted"; fi
if bash "$here/build-dmg.sh" --dry-run 'a"b' >/dev/null 2>&1; then fail "unsafe version accepted"; fi
if bash "$here/build-dmg.sh" --dry-run 'x|y' >/dev/null 2>&1; then fail "unsafe version accepted"; fi

echo "== unsigned dry run"
out=$(bash "$here/build-dmg.sh" --dry-run v1.2.3-ci) || fail "dry run failed"
echo "$out" | grep -q "building an unsigned dmg" || fail "missing unsigned notice"
if echo "$out" | grep -q "codesign\|notarytool"; then fail "signing ran without credentials"; fi
echo "$out" | grep -q "hdiutil create -volname LanMsg" || fail "missing hdiutil"
app="$OUT_DIR/dmgroot/LanMsg.app"
for b in lanmsg lanmsgd lanmsg-cli lanmsg-thumb; do
    [ -f "$app/Contents/MacOS/$b" ] || fail "missing MacOS/$b"
done
[ -f "$app/Contents/Resources/lanmsg.icns" ] || fail "missing icns"
[ -f "$app/Contents/Resources/zh-Hans.lproj/InfoPlist.strings" ] || fail "missing lproj"
[ -L "$OUT_DIR/dmgroot/Applications" ] || fail "missing Applications symlink"
if grep -q "@VERSION@" "$app/Contents/Info.plist"; then fail "placeholder left in plist"; fi
python3 - "$app/Contents/Info.plist" <<'PY'
import plistlib, sys
d = plistlib.load(open(sys.argv[1], "rb"))
assert d["CFBundleIdentifier"] == "name.apk.lanmsg"
assert d["CFBundleExecutable"] == "lanmsg"
assert d["CFBundleShortVersionString"] == "1.2.3-ci", d["CFBundleShortVersionString"]
assert d["LSMinimumSystemVersion"] == "11.0"
assert d["CFBundleVersion"] == "1.2.3", d["CFBundleVersion"]
assert d["CFBundleInfoDictionaryVersion"] == "6.0"
assert "CFBundleURLTypes" not in d
assert d["NSLocalNetworkUsageDescription"]
assert "NSBonjourServices" not in d
PY

echo "== partial credentials do not sign"
out=$(APPLE_ID=a APPLE_TEAM_ID=b bash "$here/build-dmg.sh" --dry-run 0.0.0) || fail "partial run failed"
echo "$out" | grep -q "missing: APPLE_DEVELOPER_ID APPLE_APP_PASSWORD" || fail "missing list wrong"
if echo "$out" | grep -q "codesign"; then fail "signed with partial credentials"; fi

echo "== full credentials sign and notarize"
out=$(APPLE_DEVELOPER_ID=id APPLE_ID=a APPLE_TEAM_ID=b APPLE_APP_PASSWORD='s3/cr*t' \
    bash "$here/build-dmg.sh" --dry-run 0.0.0) || fail "signed run failed"
for s in "codesign --force --options runtime --timestamp -s id" "MacOS/lanmsgd" "notarytool store-credentials lanmsg-ci" \
    "--keychain-profile lanmsg-ci" "stapler staple" "security create-keychain"; do
    echo "$out" | grep -q "$s" || fail "missing step: $s"
done

if echo "$out" | grep -q -e '--deep'; then fail "--deep used"; fi
echo "$out" | grep -qF -e '--password ***' || fail "password not redacted"
if echo "$out" | grep -qF 's3/cr*t'; then fail "password leaked into output"; fi

echo "OK"

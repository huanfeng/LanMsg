#!/bin/bash
# 生成 macOS 通用二进制 LanMsg.app 与 dmg。
# 用法：build-dmg.sh [--dry-run] <version>
#   --dry-run：只在 OUT_DIR 下组装 .app 目录布局（二进制为空占位），
#              macOS 专用命令（cargo/lipo/sips/iconutil/codesign/hdiutil/xcrun）只打印不执行。
# 环境变量：OUT_DIR（默认 ${CARGO_TARGET_DIR:-target}/dist）；
#   APPLE_DEVELOPER_ID、APPLE_ID、APPLE_TEAM_ID、APPLE_APP_PASSWORD 全部存在才签名并公证。
# 开机自启的 LaunchAgent（app.lanmsg.daemon）由应用内 autostart.rs 写入，
# 其 ProgramArguments 指向与 lanmsg 同目录的 lanmsgd，因此二者必须同在 Contents/MacOS。
set -euo pipefail

dry_run=0
if [ "${1:-}" = "--dry-run" ]; then
    dry_run=1
    shift
fi
if [ $# -ne 1 ]; then
    echo "usage: build-dmg.sh [--dry-run] <version>" >&2
    exit 2
fi
version=${1#v}
kc=""
kc_password=""
# 版本号会被写入 plist，只接受安全字符
if ! [[ $version =~ ^[0-9A-Za-z][0-9A-Za-z.+-]*$ ]]; then
    echo "error: invalid version: $1" >&2
    exit 2
fi

# CFBundleVersion 只允许点分数字：取版本号开头的数字与点（1.2.3-ci -> 1.2.3）
bundle_version=${version%%[!0-9.]*}
if [ -z "$bundle_version" ]; then
    echo "error: version must start with a number: $1" >&2
    exit 2
fi

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
tgt=${CARGO_TARGET_DIR:-$root/target}
out_dir=${OUT_DIR:-$tgt/dist}
dmg_root="$out_dir/dmgroot"
app="$dmg_root/LanMsg.app"
dmg="$out_dir/lanmsg-macos-universal.dmg"
bins=(lanmsg lanmsgd lanmsg-cli lanmsg-thumb)
targets=(x86_64-apple-darwin aarch64-apple-darwin)

# 临时 keychain 只在公证时创建，退出时一定删除
cleanup() {
    if [ -n "$kc" ] && [ "$dry_run" = 0 ]; then
        security delete-keychain "$kc" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# 打印并（非 dry-run 时）执行命令；逐参数精确比较后脱敏（密码含 * 或 / 也安全）
run() {
    local a shown="+"
    for a in "$@"; do
        if { [ -n "${APPLE_APP_PASSWORD:-}" ] && [ "$a" = "$APPLE_APP_PASSWORD" ]; } \
            || { [ -n "${kc_password:-}" ] && [ "$a" = "$kc_password" ]; }; then
            a='***'
        fi
        shown="$shown $a"
    done
    echo "$shown"
    if [ "$dry_run" = 0 ]; then
        "$@"
    fi
}

cd "$root"
rm -rf "$dmg_root"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$out_dir"

# 1. 构建两个架构并合成通用二进制
for t in "${targets[@]}"; do
    run cargo build --release --target "$t" -p lanmsgd -p lanmsg-desktop -p lanmsg-cli -p lanmsg-media
done
for b in "${bins[@]}"; do
    if [ "$dry_run" = 1 ]; then
        echo "+ lipo -create -output $app/Contents/MacOS/$b <per-arch $b>"
        : > "$app/Contents/MacOS/$b"
    else
        run lipo -create -output "$app/Contents/MacOS/$b" \
            "$tgt/x86_64-apple-darwin/release/$b" "$tgt/aarch64-apple-darwin/release/$b"
    fi
done

# 2. Info.plist 与本地化字符串
sed -e "s|@VERSION@|$version|g" -e "s|@BUNDLE_VERSION@|$bundle_version|g" "$here/Info.plist" > "$app/Contents/Info.plist"
mkdir -p "$app/Contents/Resources/zh-Hans.lproj"
cp "$here/zh-Hans.lproj/InfoPlist.strings" "$app/Contents/Resources/zh-Hans.lproj/InfoPlist.strings"

# 3. 图标：256px png -> iconset -> icns
src_png="$root/packaging/assets/lanmsg-256.png"
iconset="$out_dir/lanmsg.iconset"
rm -rf "$iconset"
mkdir -p "$iconset"
for spec in 16:icon_16x16 32:icon_16x16@2x 32:icon_32x32 64:icon_32x32@2x \
    128:icon_128x128 256:icon_128x128@2x 256:icon_256x256; do
    run sips -z "${spec%%:*}" "${spec%%:*}" "$src_png" --out "$iconset/${spec#*:}.png"
done
run iconutil -c icns "$iconset" -o "$app/Contents/Resources/lanmsg.icns"
if [ "$dry_run" = 1 ]; then
    : > "$app/Contents/Resources/lanmsg.icns"
fi

ln -s /Applications "$dmg_root/Applications"

# 4. 签名与公证（四个变量全部存在才执行）
signing=1
missing=""
for v in APPLE_DEVELOPER_ID APPLE_ID APPLE_TEAM_ID APPLE_APP_PASSWORD; do
    if [ -z "${!v:-}" ]; then
        signing=0
        missing="$missing $v"
    fi
done

if [ "$signing" = 1 ]; then
    # 不用 --deep：先逐个签内部可执行文件（hardened runtime），最后签 app 本身
    for b in "${bins[@]}"; do
        run codesign --force --options runtime --timestamp -s "$APPLE_DEVELOPER_ID" "$app/Contents/MacOS/$b"
    done
    run codesign --force --options runtime --timestamp -s "$APPLE_DEVELOPER_ID" "$app"
else
    echo "Apple signing not configured (missing:$missing); building an unsigned dmg"
fi

run hdiutil create -volname LanMsg -srcfolder "$dmg_root" -ov -format UDZO "$dmg"

if [ "$signing" = 1 ]; then
    run codesign --force --timestamp -s "$APPLE_DEVELOPER_ID" "$dmg"
    # 公证凭据：store-credentials 需要一次性在命令行带上 app 专用密码（短暂可见于进程列表，
    # CI runner 为单租户一次性环境，可接受）。凭据只存进构建时创建、退出时删除的临时 keychain，
    # 之后的 submit 都用 --keychain-profile，不再带密码。
    kc="$out_dir/lanmsg-ci.keychain-db"
    kc_password=$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')
    run security create-keychain -p "$kc_password" "$kc"
    run security set-keychain-settings -lut 3600 "$kc"
    run security unlock-keychain -p "$kc_password" "$kc"
    run xcrun notarytool store-credentials lanmsg-ci --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --keychain "$kc"
    run xcrun notarytool submit "$dmg" --keychain-profile lanmsg-ci --keychain "$kc" --wait
    run xcrun stapler staple "$dmg"
fi

echo "built $dmg"

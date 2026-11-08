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
# 版本号会被写入 plist，只接受安全字符
if ! [[ $version =~ ^[0-9A-Za-z][0-9A-Za-z.+-]*$ ]]; then
    echo "error: invalid version: $1" >&2
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

# 打印并（非 dry-run 时）执行命令
run() {
    local line="$*"
    echo "+ ${line//"${APPLE_APP_PASSWORD:-}"/***}"
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
sed "s|@VERSION@|$version|g" "$here/Info.plist" > "$app/Contents/Info.plist"
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
    run codesign --force --deep --options runtime --timestamp -s "$APPLE_DEVELOPER_ID" "$app"
else
    echo "Apple signing not configured (missing:$missing); building an unsigned dmg"
fi

run hdiutil create -volname LanMsg -srcfolder "$dmg_root" -ov -format UDZO "$dmg"

if [ "$signing" = 1 ]; then
    run codesign --force --timestamp -s "$APPLE_DEVELOPER_ID" "$dmg"
    run xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --wait
    run xcrun stapler staple "$dmg"
fi

echo "built $dmg"

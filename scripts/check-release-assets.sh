#!/bin/bash
# 核对发布目录里是否包含约定的全部发布资产（SHA256SUMS 由发布步骤随后生成，不在此列）。
# 用法：scripts/check-release-assets.sh <目录>
# 少任何一个都以退出码 1 失败，并逐个列出缺失项。
#
# 背景：v0.1.0 漏发了 Windows 安装器（打包器默认输出带版本号的文件名，找不到约定名），而
# actions/upload-artifact 的 if-no-files-found 只在所有路径都缺失时才报错，缺一个会静默通过。
# 清单必须与 .github/workflows/release.yml 文件头注释一致（scripts/check-packaging.sh 校验）。
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "usage: check-release-assets.sh <dir>" >&2
    exit 2
fi
dir=$1

ASSETS=(
    lanmsg-windows-x64-setup.exe
    lanmsg-windows-arm64-setup.exe
    lanmsg-windows-x64.zip
    lanmsg-windows-arm64.zip
    lanmsg-linux-x86_64.tar.gz
    lanmsg-linux-aarch64.tar.gz
    lanmsgd-linux-x86_64-musl.tar.gz
    lanmsgd-linux-aarch64-musl.tar.gz
    lanmsg-macos-universal.dmg
    lanmsg-android.apk
    install.sh
    uninstall.sh
)

if [ "${1:-}" = "--list" ]; then
    printf '%s\n' "${ASSETS[@]}"
    exit 0
fi

missing=0
for f in "${ASSETS[@]}"; do
    if [ ! -s "$dir/$f" ]; then
        echo "::error::missing or empty release asset: $f" >&2
        missing=1
    fi
done
[ "$missing" = 0 ] || exit 1
echo "release assets complete (${#ASSETS[@]} files)"

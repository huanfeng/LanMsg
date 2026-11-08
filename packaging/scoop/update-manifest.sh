#!/bin/bash
# 由模板生成 scoop 清单：填充版本、仓库与两个架构的 SHA256。
# 用法：update-manifest.sh <version> <SHA256SUMS 路径>
# 环境变量：REPO_SLUG（默认 huanfeng/lanmsg）、OUT_DIR（默认 ${CARGO_TARGET_DIR:-target}/dist）。
set -euo pipefail

REPO_SLUG=${REPO_SLUG:-huanfeng/lanmsg}

if [ $# -ne 2 ]; then
    echo "usage: update-manifest.sh <version> <SHA256SUMS>" >&2
    exit 2
fi
version=${1#v}
sums=$2
if ! [[ $version =~ ^[0-9A-Za-z][0-9A-Za-z.+-]*$ ]]; then
    echo "error: invalid version: $1" >&2
    exit 2
fi
if ! [[ $REPO_SLUG =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]]; then
    echo "error: invalid REPO_SLUG: $REPO_SLUG" >&2
    exit 2
fi
[ -f "$sums" ] || { echo "error: no such file: $sums" >&2; exit 1; }

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out_dir=${OUT_DIR:-${CARGO_TARGET_DIR:-$root/target}/dist}

# 取 SHA256SUMS 中指定文件的哈希（格式：<hash>  <file>，文件名前可带 *）
hash_of() {
    local h
    h=$(awk -v f="$1" '{ n=$2; sub(/^\*/, "", n); if (n == f) { print $1; exit } }' "$sums")
    if ! [[ $h =~ ^[0-9a-fA-F]{64}$ ]]; then
        echo "error: no valid SHA256 for $1 in $sums" >&2
        exit 1
    fi
    echo "$h" | tr 'A-F' 'a-f'
}

x64=$(hash_of lanmsg-windows-x64.zip)
arm64=$(hash_of lanmsg-windows-arm64.zip)

mkdir -p "$out_dir"
sed -e "s|@VERSION@|$version|g" \
    -e "s|@REPO@|$REPO_SLUG|g" \
    -e "s|@SHA256_X64@|$x64|g" \
    -e "s|@SHA256_ARM64@|$arm64|g" \
    "$here/lanmsg.json" > "$out_dir/lanmsg.json"
echo "wrote $out_dir/lanmsg.json"

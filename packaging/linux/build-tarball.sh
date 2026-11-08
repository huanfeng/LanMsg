#!/bin/sh
# 生成 Linux 发布包。
# 用法：build-tarball.sh [--headless] <target-triple> <out-name>
#   桌面版：bin/{lanmsg,lanmsgd,lanmsg-cli,lanmsg-thumb} + .desktop + 图标 + service + LICENSE
#   --headless：bin/{lanmsgd,lanmsg-cli} + service（ExecStart 带 --headless）+ LICENSE，
#               lanmsgd 以 --no-default-features 构建（不链接 windui）
# 环境变量：PROFILE=release|debug（默认 release）、OUT_DIR（默认 ${CARGO_TARGET_DIR:-target}/dist）。
set -eu

headless=0
if [ "${1:-}" = "--headless" ]; then
    headless=1
    shift
fi
if [ $# -ne 2 ]; then
    echo "usage: build-tarball.sh [--headless] <target-triple> <out-name>" >&2
    exit 2
fi
triple=$1
out_name=$2

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
tgt=${CARGO_TARGET_DIR:-$root/target}
profile=${PROFILE:-release}
out_dir=${OUT_DIR:-$tgt/dist}
case $profile in
    release) cargo_profile="--release" ;;
    debug) cargo_profile="" ;;
    *) echo "unknown PROFILE: $profile" >&2; exit 2 ;;
esac

cd "$root"
# cargo_profile 为空或单个标志，需要按单词展开
if [ "$headless" = 1 ]; then
    # shellcheck disable=SC2086
    cargo build $cargo_profile --target "$triple" -p lanmsgd --no-default-features
    # shellcheck disable=SC2086
    cargo build $cargo_profile --target "$triple" -p lanmsg-cli
    bins="lanmsgd lanmsg-cli"
else
    # shellcheck disable=SC2086
    cargo build $cargo_profile --target "$triple" -p lanmsgd -p lanmsg-desktop -p lanmsg-cli -p lanmsg-media
    bins="lanmsg lanmsgd lanmsg-cli lanmsg-thumb"
fi

bin_dir=$tgt/$triple/$profile
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT INT TERM

mkdir -p "$stage/bin" "$stage/lib/systemd/user"
for b in $bins; do
    cp "$bin_dir/$b" "$stage/bin/$b"
done
cp "$root/LICENSE" "$stage/LICENSE"

if [ "$headless" = 1 ]; then
    sed 's|@HEADLESS@|--headless|' "$here/lanmsgd.service" > "$stage/lib/systemd/user/lanmsgd.service"
else
    cp "$here/lanmsgd.service" "$stage/lib/systemd/user/lanmsgd.service"
    mkdir -p "$stage/share/applications" "$stage/share/icons/hicolor/256x256/apps"
    cp "$here/lanmsg.desktop" "$stage/share/applications/lanmsg.desktop"
    cp "$root/packaging/assets/lanmsg-256.png" "$stage/share/icons/hicolor/256x256/apps/lanmsg.png"
fi

mkdir -p "$out_dir"
tar -czf "$out_dir/$out_name" -C "$stage" .
echo "built $out_dir/$out_name"

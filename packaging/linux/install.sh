#!/bin/sh
# LanMsg Linux 安装脚本。
# 用法：install.sh [--headless] [--version vX.Y.Z] [--prefix DIR]
# 环境变量：LANMSG_BASE_URL（覆盖下载地址）、LANMSG_NO_SYSTEMD=1（跳过 systemd 与桌面数据库操作）。
set -eu

headless=0
version=""
prefix="$HOME/.local"

while [ $# -gt 0 ]; do
    case $1 in
        --headless) headless=1 ;;
        --version)
            [ $# -ge 2 ] || { echo "error: --version needs a value" >&2; exit 2; }
            version=$2
            shift
            ;;
        --prefix)
            [ $# -ge 2 ] || { echo "error: --prefix needs a value" >&2; exit 2; }
            prefix=$2
            shift
            ;;
        -h | --help)
            echo "usage: install.sh [--headless] [--version vX.Y.Z] [--prefix DIR]"
            exit 0
            ;;
        *) echo "error: unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

# 1. 架构
case $(uname -m) in
    x86_64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) echo "error: unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

# 2. glibc 检查（桌面版）
if [ "$headless" = 0 ]; then
    ldd_out=$(ldd --version 2>&1 | head -n 1 || true)
    case $ldd_out in
        *musl*)
            echo "error: musl libc detected; use --headless to install the headless build" >&2
            exit 1
            ;;
    esac
    glibc=$(echo "$ldd_out" | sed -n 's/.*[^0-9.]\([0-9][0-9]*\.[0-9][0-9]*\)$/\1/p')
    if [ -n "$glibc" ]; then
        major=${glibc%%.*}
        minor=${glibc#*.}
        if [ "$major" -lt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -lt 31 ]; }; then
            echo "error: system glibc $glibc is older than 2.31; use --headless to install the headless build" >&2
            exit 1
        fi
    fi
fi

# 3. 下载并校验
if [ -n "$version" ]; then
    default_base="https://github.com/huanfeng/lanmsg/releases/download/$version"
else
    default_base="https://github.com/huanfeng/lanmsg/releases/latest/download"
fi
base=${LANMSG_BASE_URL:-$default_base}
if [ "$headless" = 1 ]; then
    tarball="lanmsgd-linux-$arch-musl.tar.gz"
else
    tarball="lanmsg-linux-$arch.tar.gz"
fi

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        echo "error: curl or wget is required" >&2
        return 1
    fi
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

echo "Downloading $tarball"
fetch "$base/$tarball" "$work/$tarball" || { echo "error: failed to download $tarball" >&2; exit 1; }
fetch "$base/SHA256SUMS" "$work/SHA256SUMS" || { echo "error: failed to download SHA256SUMS" >&2; exit 1; }
grep "  $tarball\$" "$work/SHA256SUMS" > "$work/SHA256SUMS.one" \
    || { echo "error: $tarball not listed in SHA256SUMS" >&2; exit 1; }
(cd "$work" && sha256sum -c SHA256SUMS.one >/dev/null) \
    || { echo "error: checksum verification failed" >&2; exit 1; }

# 4. 解压并安装
mkdir "$work/pkg"
tar -xzf "$work/$tarball" -C "$work/pkg"

mkdir -p "$prefix/bin"
cp "$work/pkg/bin/"* "$prefix/bin/"
chmod 755 "$prefix/bin/"*

if [ -d "$work/pkg/share" ]; then
    mkdir -p "$prefix/share"
    cp -R "$work/pkg/share/." "$prefix/share/"
    # 让 .desktop 指向实际安装位置
    if [ -f "$prefix/share/applications/lanmsg.desktop" ]; then
        sed "s|^Exec=lanmsg |Exec=$prefix/bin/lanmsg |" "$prefix/share/applications/lanmsg.desktop" \
            > "$work/lanmsg.desktop"
        cp "$work/lanmsg.desktop" "$prefix/share/applications/lanmsg.desktop"
    fi
fi

unit_dir=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
mkdir -p "$unit_dir"
if [ "$headless" = 1 ]; then
    flag="--headless"
else
    flag=""
fi
sed -e "s|%h/.local/bin|$prefix/bin|" -e "s|@HEADLESS@|$flag|" -e 's| *$||' \
    "$work/pkg/lib/systemd/user/lanmsgd.service" > "$unit_dir/lanmsgd.service"

# 5. systemd 与桌面数据库
if [ "${LANMSG_NO_SYSTEMD:-}" != 1 ] && command -v systemctl >/dev/null 2>&1 \
    && systemctl --user show-environment >/dev/null 2>&1; then
    systemctl --user daemon-reload
    systemctl --user enable --now lanmsgd
else
    echo "systemd user session not available; start the service manually with: $prefix/bin/lanmsgd"
fi
if [ "${LANMSG_NO_SYSTEMD:-}" != 1 ] && [ -d "$prefix/share/applications" ] \
    && command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
fi

# 6. PATH 提示
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "note: $prefix/bin is not in PATH; add it to your shell profile" ;;
esac

# 7. 完成
echo "LanMsg installed"

#!/bin/sh
# LanMsg Linux 安装脚本。
# 用法：install.sh [--headless] [--version vX.Y.Z] [--prefix DIR]
# 环境变量：
#   LANMSG_BASE_URL    发布页根地址（默认 https://github.com/huanfeng/lanmsg/releases），
#                      实际下载目录为 <根>/latest/download 或 <根>/download/<版本>
#   LANMSG_NO_SYSTEMD=1  跳过 systemd 与桌面数据库操作
# 整个脚本包在 main 里，末行才调用：curl | sh 被截断时不会执行半截脚本。
set -eu

work=""
cleanup() {
    [ -n "$work" ] && rm -rf "$work"
    return 0
}
trap cleanup EXIT
trap 'exit 130' INT TERM

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

# 校验 $1 目录下 SHA256SUMS.one 描述的文件
verify_sum() {
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$1" && sha256sum -c SHA256SUMS.one >/dev/null)
    elif command -v shasum >/dev/null 2>&1; then
        (cd "$1" && shasum -a 256 -c SHA256SUMS.one >/dev/null)
    else
        echo "error: sha256sum or shasum is required" >&2
        return 1
    fi
}

main() {
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

    # prefix 会被写进 sed 替换串与 service/.desktop 文件，拒绝空白与 sed 特殊字符
    case $prefix in
        *[[:space:]]* | *'|'* | *'&'* | *\\*)
            printf 'error: --prefix must not contain whitespace, |, & or backslash\n' >&2
            exit 2
            ;;
    esac

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
    root=${LANMSG_BASE_URL:-https://github.com/huanfeng/lanmsg/releases}
    if [ -n "$version" ]; then
        base="$root/download/$version"
    else
        base="$root/latest/download"
    fi
    if [ "$headless" = 1 ]; then
        tarball="lanmsgd-linux-$arch-musl.tar.gz"
    else
        tarball="lanmsg-linux-$arch.tar.gz"
    fi

    work=$(mktemp -d)
    echo "Downloading $tarball"
    fetch "$base/$tarball" "$work/$tarball" || {
        echo "error: failed to download $tarball" >&2
        echo "hint: possible causes: the repository is private (anonymous download returns 404), the network is unreachable, LANMSG_BASE_URL is wrong, or the requested version does not exist. Set LANMSG_BASE_URL to a mirror or a local directory (file:// URL or http server) to install from elsewhere." >&2
        exit 1
    }
    fetch "$base/SHA256SUMS" "$work/SHA256SUMS" || { echo "error: failed to download SHA256SUMS" >&2; exit 1; }
    grep "  $tarball\$" "$work/SHA256SUMS" > "$work/SHA256SUMS.one" \
        || { echo "error: $tarball not listed in SHA256SUMS" >&2; exit 1; }
    verify_sum "$work" || { echo "error: checksum verification failed" >&2; exit 1; }

    # 4. 解压前预检成员路径，再解压并安装
    tar -tzf "$work/$tarball" > "$work/members" || { echo "error: cannot read tarball" >&2; exit 1; }
    if grep -qE '(^/|(^|/)\.\.(/|$))' "$work/members"; then
        echo "error: tarball contains unsafe paths" >&2
        exit 1
    fi
    mkdir "$work/pkg"
    tar -xzf "$work/$tarball" -C "$work/pkg"

    mkdir -p "$prefix/bin"
    # 先复制到同目录临时名再 mv：原子替换，运行中的二进制不会报 Text file busy
    for src in "$work/pkg/bin/"*; do
        name=$(basename "$src")
        cp "$src" "$prefix/bin/.$name.new"
        chmod 755 "$prefix/bin/.$name.new"
        mv -f "$prefix/bin/.$name.new" "$prefix/bin/$name"
    done

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
        # 服务已在运行时 enable --now 不会重启，需显式重启以换上新二进制
        systemctl --user restart lanmsgd
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
}

main "$@"

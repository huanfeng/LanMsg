#!/bin/sh
# LanMsg Linux 卸载脚本。
# 用法：uninstall.sh [--purge] [--yes] [--prefix DIR]
# --purge 额外删除数据目录（需输入 yes 确认，--yes 跳过确认）。
set -eu

purge=0
assume_yes=0
prefix="$HOME/.local"

while [ $# -gt 0 ]; do
    case $1 in
        --purge) purge=1 ;;
        --yes) assume_yes=1 ;;
        --prefix)
            [ $# -ge 2 ] || { echo "error: --prefix needs a value" >&2; exit 2; }
            prefix=$2
            shift
            ;;
        -h | --help)
            echo "usage: uninstall.sh [--purge] [--yes] [--prefix DIR]"
            exit 0
            ;;
        *) echo "error: unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

if [ "${LANMSG_NO_SYSTEMD:-}" != 1 ] && command -v systemctl >/dev/null 2>&1; then
    systemctl --user disable --now lanmsgd >/dev/null 2>&1 || true
fi

unit="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/lanmsgd.service"
rm -f "$unit" \
    "$prefix/bin/lanmsg" "$prefix/bin/lanmsgd" "$prefix/bin/lanmsg-cli" "$prefix/bin/lanmsg-thumb" \
    "$prefix/share/applications/lanmsg.desktop" \
    "$prefix/share/icons/hicolor/256x256/apps/lanmsg.png"

if [ "${LANMSG_NO_SYSTEMD:-}" != 1 ]; then
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload >/dev/null 2>&1 || true
    fi
    if [ -d "$prefix/share/applications" ] && command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
    fi
fi

if [ "$purge" = 1 ]; then
    data="${XDG_DATA_HOME:-$HOME/.local/share}/lanmsg"
    echo "This will delete: $data"
    if [ "$assume_yes" = 0 ]; then
        printf 'Type "yes" to confirm: '
        answer=""
        read -r answer || true
        if [ "$answer" != "yes" ]; then
            echo "Data directory kept"
            echo "LanMsg uninstalled"
            exit 0
        fi
    fi
    rm -rf "$data"
fi

echo "LanMsg uninstalled"

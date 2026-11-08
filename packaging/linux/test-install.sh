#!/bin/sh
# install.sh / uninstall.sh / build-tarball.sh 的端到端测试（本机架构，无 systemd 分支）。
# 用法：sh packaging/linux/test-install.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
tmp=$(mktemp -d)
server_pid=""
daemon_pid=""
cleanup() {
    [ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null || true
    [ -n "$daemon_pid" ] && kill "$daemon_pid" 2>/dev/null || true
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

fail() { echo "FAIL: $*" >&2; exit 1; }

case $(uname -m) in
    x86_64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) fail "unsupported arch" ;;
esac
triple=$(rustc -vV | sed -n 's/^host: //p')

# 使用 debug 产物以缩短构建时间
export PROFILE=debug
export OUT_DIR="$tmp/dist"
export LANMSG_NO_SYSTEMD=1
export HOME="$tmp/home"
export XDG_CONFIG_HOME="$tmp/home/.config"
export XDG_DATA_HOME="$tmp/home/.local/share"
mkdir -p "$HOME"

desktop_name="lanmsg-linux-$arch.tar.gz"
headless_name="lanmsgd-linux-$arch-musl.tar.gz"

echo "== build tarballs"
sh "$here/build-tarball.sh" "$triple" "$desktop_name"
sh "$here/build-tarball.sh" --headless "$triple" "$headless_name"
[ -f "$OUT_DIR/$desktop_name" ] || fail "desktop tarball missing"
[ -f "$OUT_DIR/$headless_name" ] || fail "headless tarball missing"

(cd "$OUT_DIR" && sha256sum "$desktop_name" "$headless_name" > SHA256SUMS)

# 起本地 HTTP 服务
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
(cd "$OUT_DIR" && exec python3 -m http.server "$port" --bind 127.0.0.1 >/dev/null 2>&1) &
server_pid=$!
sleep 1
# 发布页目录结构：<根>/latest/download 与 <根>/download/<版本>
mkdir -p "$OUT_DIR/releases/latest" "$OUT_DIR/releases/download/v9.9.9"
ln -s ../.. "$OUT_DIR/releases/latest/download" 2>/dev/null || true
export LANMSG_BASE_URL="http://127.0.0.1:$port/releases"

echo "== desktop install"
prefix="$tmp/prefix"
out=$(sh "$here/install.sh" --prefix "$prefix") || fail "desktop install failed"
echo "$out" | tail -n 1 | grep -qx "LanMsg installed" || fail "last line is not 'LanMsg installed'"
"$prefix/bin/lanmsgd" --help >/dev/null || fail "lanmsgd --help"
for f in lanmsg lanmsg-cli lanmsg-thumb; do
    [ -x "$prefix/bin/$f" ] || fail "missing bin/$f"
done
[ -f "$prefix/share/applications/lanmsg.desktop" ] || fail "missing .desktop"
grep -q "^Exec=$prefix/bin/lanmsg %U" "$prefix/share/applications/lanmsg.desktop" || fail ".desktop Exec not rewritten"
[ -f "$prefix/share/icons/hicolor/256x256/apps/lanmsg.png" ] || fail "missing icon"
svc="$XDG_CONFIG_HOME/systemd/user/lanmsgd.service"
[ -f "$svc" ] || fail "missing service file"
grep -q "^ExecStart=$prefix/bin/lanmsgd\$" "$svc" || fail "service ExecStart not rewritten"
if grep -q "@HEADLESS@" "$svc"; then fail "placeholder left in service"; fi

echo "== uninstall"
sh "$here/uninstall.sh" --prefix "$prefix" >/dev/null || fail "uninstall failed"
[ ! -e "$prefix/bin/lanmsgd" ] || fail "lanmsgd still present"
[ ! -e "$prefix/share/applications/lanmsg.desktop" ] || fail ".desktop still present"
[ ! -e "$svc" ] || fail "service file still present"

echo "== headless install"
prefix2="$tmp/prefix2"
sh "$here/install.sh" --headless --prefix "$prefix2" >/dev/null || fail "headless install failed"
"$prefix2/bin/lanmsgd" --help >/dev/null || fail "headless lanmsgd --help"
[ -x "$prefix2/bin/lanmsg-cli" ] || fail "headless missing lanmsg-cli"
[ ! -e "$prefix2/bin/lanmsg" ] || fail "headless must not ship lanmsg"
grep -q "^ExecStart=$prefix2/bin/lanmsgd --headless\$" "$svc" || fail "headless service lacks --headless"

echo "== reinstall while lanmsgd is running"
mkdir -p "$tmp/rundata"
"$prefix2/bin/lanmsgd" --headless --data-dir "$tmp/rundata" --port 0 --no-multicast >/dev/null 2>&1 &
daemon_pid=$!
sleep 1
kill -0 "$daemon_pid" 2>/dev/null || fail "test daemon did not start"
sh "$here/install.sh" --headless --prefix "$prefix2" >/dev/null || fail "reinstall over running daemon failed"
kill "$daemon_pid" 2>/dev/null || true
wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""

echo "== --version path"
cp "$OUT_DIR/$headless_name" "$OUT_DIR/SHA256SUMS" "$OUT_DIR/releases/download/v9.9.9/"
prefix4="$tmp/prefix4"
sh "$here/install.sh" --headless --version v9.9.9 --prefix "$prefix4" >/dev/null || fail "--version install failed"
[ -x "$prefix4/bin/lanmsgd" ] || fail "--version install missing lanmsgd"
if sh "$here/install.sh" --headless --version v0.0.0 --prefix "$tmp/prefix5" >/dev/null 2>&1; then
    fail "nonexistent version was accepted"
fi

echo "== unsafe prefix rejected"
for bad in "$tmp/with space" "$tmp/a|b" "$tmp/a&b" "$tmp/a\\b"; do
    if sh "$here/install.sh" --headless --prefix "$bad" >/dev/null 2>&1; then
        fail "unsafe prefix accepted: $bad"
    fi
done

echo "== unsafe tarball members rejected"
python3 - "$OUT_DIR/releases/download/v9.9.9/$headless_name" <<'PY'
import io, sys, tarfile
with tarfile.open(sys.argv[1], "w:gz") as t:
    data = b"x"
    info = tarfile.TarInfo("../evil")
    info.size = len(data)
    t.addfile(info, io.BytesIO(data))
PY
(cd "$OUT_DIR/releases/download/v9.9.9" && sha256sum "$headless_name" > SHA256SUMS)
if sh "$here/install.sh" --headless --version v9.9.9 --prefix "$tmp/prefix6" >/dev/null 2>&1; then
    fail "tarball with .. member accepted"
fi

echo "== purge asks for confirmation"
mkdir -p "$XDG_DATA_HOME/lanmsg"
echo x > "$XDG_DATA_HOME/lanmsg/keep"
echo no | sh "$here/uninstall.sh" --purge --prefix "$prefix2" >/dev/null 2>&1 || true
[ -f "$XDG_DATA_HOME/lanmsg/keep" ] || fail "purge removed data without confirmation"
sh "$here/install.sh" --headless --prefix "$prefix2" >/dev/null
sh "$here/uninstall.sh" --purge --yes --prefix "$prefix2" >/dev/null || fail "purge failed"
[ ! -e "$XDG_DATA_HOME/lanmsg" ] || fail "purge did not remove data dir"

echo "== tampered tarball must fail"
python3 - "$OUT_DIR/$desktop_name" <<'PY'
import sys
p = sys.argv[1]
b = bytearray(open(p, "rb").read())
b[len(b) // 2] ^= 0xFF
open(p, "wb").write(b)
PY
prefix3="$tmp/prefix3"
if sh "$here/install.sh" --prefix "$prefix3" >/dev/null 2>&1; then
    fail "tampered tarball was accepted"
fi
[ ! -e "$prefix3/bin/lanmsgd" ] || fail "tampered install left files"

echo "== headless build has no windui"
cd "$root"
cargo build -p lanmsgd --no-default-features
n=$(cargo tree -p lanmsgd --no-default-features | grep -c windui || true)
[ "$n" = "0" ] || fail "windui still in headless dependency tree ($n)"

echo "OK"

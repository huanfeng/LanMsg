#!/usr/bin/env bash
# P1 阶段 e2e：同一台机器上两个 lanmsgd 经 TCP 互连并收发文字消息。
# 用法：bash scripts/e2e-two-daemons.sh
# 末行输出 E2E OK 表示通过。
set -euo pipefail

ROOT="$(mktemp -d /tmp/lanmsg-e2e.XXXXXX)"
DIR_A="$ROOT/a"
DIR_B="$ROOT/b"
PORT_A=53330
PORT_B=53331

BIN_A="$(dirname "$0")/../target/debug/lanmsgd"
BIN_CLI="$(dirname "$0")/../target/debug/lanmsg-cli"

cleanup() {
    cli_a shutdown >/dev/null 2>&1 || true
    cli_b shutdown >/dev/null 2>&1 || true
}

# CLI 包装：Linux 的 XDG 端点为固定路径，同一台机器跑两个实例
# 需为每个实例隔离 XDG_RUNTIME_DIR，使端点各为 <data_dir>/run/lanmsg/lanmsgd.sock。
cli_a() { XDG_RUNTIME_DIR="$DIR_A/run" "$BIN_CLI" --data-dir "$DIR_A" "$@"; }
cli_b() { XDG_RUNTIME_DIR="$DIR_B/run" "$BIN_CLI" --data-dir "$DIR_B" "$@"; }
trap cleanup EXIT

# 启动两个守护进程（A 无对端；B 静态指向 A），各自隔离 XDG 端点。
XDG_RUNTIME_DIR="$DIR_A/run" "$BIN_A" --no-multicast --port "$PORT_A" --data-dir "$DIR_A" &
PID_A=$!
XDG_RUNTIME_DIR="$DIR_B/run" "$BIN_A" --no-multicast --port "$PORT_B" --data-dir "$DIR_B" --peer "127.0.0.1:$PORT_A" &
PID_B=$!

# 等待 A 的 IPC 就绪。
for _ in $(seq 1 50); do
    if cli_a self >/dev/null 2>&1; then break; fi
    sleep 0.2
done
cli_a self >/dev/null

# 各自初始化。
cli_a init --name 甲 --device-name 甲机
cli_b init --name 乙 --device-name 乙机

# B 等 A 上线（静态拨号 + 连接建立）。
USER_A=""
for _ in $(seq 1 50); do
    if cli_b peers 2>/dev/null | grep -q "甲"; then
        break
    fi
    sleep 0.2
done
cli_b peers | grep -q "甲" || {
    echo "E2E 失败：B 未发现 A"
    exit 1
}

# 取 A 的 user_id。
USER_A="$(cli_a self | grep '^user_id:' | awk '{print $2}')"
[ -n "$USER_A" ] || { echo "E2E 失败：未取得 A 的 user_id"; exit 1; }

# B 打开 DM 并发消息。
CONV="$(cli_b dm "$USER_A")"
[ -n "$CONV" ] || { echo "E2E 失败：未取得会话 ID"; exit 1; }
cli_b send "$CONV" "你好，来自乙"

# A 的 log 看到消息（等同步）。
for _ in $(seq 1 50); do
    if cli_a log "$CONV" 2>/dev/null | grep -q "你好，来自乙"; then
        break
    fi
    sleep 0.2
done
cli_a log "$CONV" | grep -q "你好，来自乙" || {
    echo "E2E 失败：A 未收到消息"
    exit 1
}

# 收尾：两边 shutdown。
cli_b shutdown >/dev/null
cli_a shutdown >/dev/null
wait "$PID_A" 2>/dev/null || true
wait "$PID_B" 2>/dev/null || true

echo "E2E OK"

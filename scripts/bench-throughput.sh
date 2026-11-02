#!/usr/bin/env bash
# P2 阶段吞吐基准（设计 §17）。
# 用法：
#   bash scripts/bench-throughput.sh local [SIZE_MB]     # 默认 2048
#   bash scripts/bench-throughput.sh remote <对端IP>
#
# local：同机起两个 lanmsgd（端口 53340/53341，无组播，静态互指），
# 互加联系人，关自动下载，A 生成 SIZE_MB 随机文件发到 DM，B 等消息出现后
# download --wait，末行打印 BENCH <MB/s>。
#
# remote：本机执行接收侧步骤。假设对端已按「两机手工流程」准备好：
#   对端机器执行：
#     lanmsgd --port 53340 --data-dir ~/bench-a
#     lanmsg-cli init --name 对端 --device-name 对端机
#     lanmsg-cli request-contact <本机 user_id>   # 互换 user_id 后互加
#     （本机 respond add 之后）lanmsg-cli send-files <conv> <SIZE_MB 的文件>
#   本机执行：
#     lanmsgd --port 53341 --data-dir ~/bench-b
#     lanmsg-cli init --name 本机 --device-name 本机机
#     （对端 request-contact 本机后）lanmsg-cli respond <request> add
#     bash scripts/bench-throughput.sh remote <对端IP>
# 本机等对端文件出现在任一会话后 download --wait，末行打印 BENCH <MB/s>。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_D="$ROOT/target/debug/lanmsgd"
BIN_CLI="$ROOT/target/debug/lanmsg-cli"

print_result() {
    # $1 = 平均速度 MB/s
    printf 'BENCH %s\n' "$1"
}

die() { echo "BENCH 失败：$*" >&2; exit 1; }

# CLI 包装：同机多实例需隔离 XDG_RUNTIME_DIR（Linux 端点为固定路径）。
cli_a() { XDG_RUNTIME_DIR="$DIR_A/run" "$BIN_CLI" --data-dir "$DIR_A" "$@"; }
cli_b() { XDG_RUNTIME_DIR="$DIR_B/run" "$BIN_CLI" --data-dir "$DIR_B" "$@"; }

MODE="${1:-}"
case "$MODE" in
local)
    SIZE_MB="${2:-2048}"
    ROOT_DIR="$(mktemp -d /tmp/lanmsg-bench.XXXXXX)"
    DIR_A="$ROOT_DIR/a"
    DIR_B="$ROOT_DIR/b"
    PORT_A=53340
    PORT_B=53341
    trap 'cli_a shutdown >/dev/null 2>&1 || true; cli_b shutdown >/dev/null 2>&1 || true; rm -rf "$ROOT_DIR"' EXIT

    # 启动两个守护进程（B 静态指向 A；日志落各自 data_dir，stdout 丢弃，
    # 保证 BENCH 是脚本末行）。
    XDG_RUNTIME_DIR="$DIR_A/run" "$BIN_D" --no-multicast --port "$PORT_A" --data-dir "$DIR_A" >/dev/null 2>&1 &
    XDG_RUNTIME_DIR="$DIR_B/run" "$BIN_D" --no-multicast --port "$PORT_B" --data-dir "$DIR_B" --peer "127.0.0.1:$PORT_A" >/dev/null 2>&1 &

    # 等 IPC 就绪。
    for _ in $(seq 1 50); do cli_a self >/dev/null 2>&1 && break; sleep 0.2; done
    for _ in $(seq 1 50); do cli_b self >/dev/null 2>&1 && break; sleep 0.2; done

    # 各自初始化。
    cli_a init --name 甲 --device-name 甲机
    cli_b init --name 乙 --device-name 乙机

    USER_A="$(cli_a self | grep '^user_id:' | awk '{print $2}')"
    USER_B="$(cli_b self | grep '^user_id:' | awk '{print $2}')"
    [ -n "$USER_A" ] || die "未取得 A 的 user_id"
    [ -n "$USER_B" ] || die "未取得 B 的 user_id"

    # B 等 A 上线（静态拨号 + 连接建立）。
    for _ in $(seq 1 50); do
        if cli_b peers 2>/dev/null | grep -q "甲"; then break; fi
        sleep 0.2
    done
    cli_b peers | grep -q "甲" || die "B 未发现 A"

    # 互加联系人：B 请求 → A respond add。
    cli_b request-contact "$USER_A"
    REQ=""
    for _ in $(seq 1 50); do
        REQ="$(cli_a requests | awk 'NR==1{print $1}')"
        [ -n "$REQ" ] && break
        sleep 0.2
    done
    [ -n "$REQ" ] || die "A 未收到联系人请求"
    cli_a respond "$REQ" add

    # 关自动下载（bench 只测手动下载吞吐）。
    cli_a settings set auto_file_max 0
    cli_a settings set auto_image_max 0
    cli_b settings set auto_file_max 0
    cli_b settings set auto_image_max 0

    # A 生成随机文件并发到与 B 的 DM。
    SRC="$(mktemp /tmp/lanmsg-bench-src.XXXXXX)"
    trap 'rm -f "$SRC"; cli_a shutdown >/dev/null 2>&1 || true; cli_b shutdown >/dev/null 2>&1 || true; rm -rf "$ROOT_DIR"' EXIT
    head -c "$((SIZE_MB * 1024 * 1024))" /dev/urandom > "$SRC"

    CONV="$(cli_a dm "$USER_B")"
    [ -n "$CONV" ] || die "未取得会话 ID"
    cli_a send-files "$CONV" "$SRC" >/dev/null

    # B 等文件消息出现（轮询 files-of），提取 hash。
    HASH=""
    for _ in $(seq 1 300); do
        HASH="$(cli_b files-of "$CONV" | awk 'NF{print $1}' | head -1)"
        [ -n "$HASH" ] && break
        sleep 0.2
    done
    [ -n "$HASH" ] || die "B 未等到文件消息"

    # B 下载并等待完成；download --wait 打印「耗时 X.XXs 平均 X.X MB/s」。
    SPEED="$(cli_b download "$HASH" --wait | grep '平均' | sed 's/.*平均 \([0-9.]*\) MB\/s.*/\1/')"
    [ -n "$SPEED" ] || die "未取得平均速度（下载失败或无速度行）"

    echo "BENCH 文件 $SIZE_MB MB 平均 $SPEED MB/s"
    print_result "$SPEED"
    ;;
remote)
    PEER_IP="${2:-}"
    [ -n "$PEER_IP" ] || die "用法：bench-throughput.sh remote <对端IP>"
    ROOT_DIR="$(mktemp -d /tmp/lanmsg-bench.XXXXXX)"
    DIR_A="$ROOT_DIR/b"
    PORT_A=53341
    trap 'cli_a shutdown >/dev/null 2>&1 || true; rm -rf "$ROOT_DIR"' EXIT

    XDG_RUNTIME_DIR="$DIR_A/run" "$BIN_D" --no-multicast --port "$PORT_A" --data-dir "$DIR_A" >/dev/null 2>&1 &

    for _ in $(seq 1 50); do cli_a self >/dev/null 2>&1 && break; sleep 0.2; done
    USER_A="$(cli_a self | grep '^user_id:' | awk '{print $2}')"
    echo "本机 user_id: $USER_A（对端 request-contact 需要它）"
    echo "等待对端联系人请求与文件…"

    # 等联系人请求 → add。
    REQ=""
    for _ in $(seq 1 150); do
        REQ="$(cli_a requests | awk 'NR==1{print $1}')"
        [ -n "$REQ" ] && break
        sleep 0.2
    done
    [ -n "$REQ" ] || die "未等到联系人请求"
    cli_a respond "$REQ" add

    # 关自动下载。
    cli_a settings set auto_file_max 0
    cli_a settings set auto_image_max 0

    # 等任一会话出现文件项，取首个含文件的会话与 hash。
    CONV=""
    HASH=""
    for _ in $(seq 1 600); do
        CONV="$(cli_a convs | awk 'NR>1 && NF {print $1}' | head -1)"
        if [ -n "$CONV" ]; then
            HASH="$(cli_a files-of "$CONV" | awk 'NF{print $1}' | head -1)"
            [ -n "$HASH" ] && break
        fi
        sleep 0.2
    done
    [ -n "$HASH" ] || die "未等到对端文件"

    SPEED="$(cli_a download "$HASH" --wait | grep '平均' | sed 's/.*平均 \([0-9.]*\) MB\/s.*/\1/')"
    [ -n "$SPEED" ] || die "未取得平均速度（下载失败或无速度行）"

    echo "BENCH 平均 $SPEED MB/s"
    print_result "$SPEED"
    ;;
*)
    die "用法：bench-throughput.sh local [SIZE_MB] | remote <对端IP>"
    ;;
esac

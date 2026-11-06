#!/usr/bin/env bash
# P3 内存与空闲 CPU 测量（Linux）。
#
# 用法：
#   bash scripts/measure-memory.sh <data_dir> [seconds=60]
# 环境变量：
#   LANMSG_BIN_DIR   二进制目录（默认 <仓库>/target/release；建议 release 构建）
#   CPU_SECONDS      lanmsgd 空闲 CPU 观察窗口秒数（默认 30，0 跳过）
#   THRESHOLD_KB     lanmsgd private 内存阈值（默认 15360）
#   MSG_COUNT        第二次测量前写入的消息条数（默认 1000）
#   RESULT_JSON      若设置，机器可读结果（JSON）另存到该文件
#
# 流程：启动 lanmsgd（有 DISPLAY 为常驻托盘模式，否则 --headless）→ 空闲 <seconds> 秒测一次 →
# 空闲 CPU 窗口 → 向“我的设备”会话写 MSG_COUNT 条 → 再等 <seconds> 秒测一次 →
# 界面进程：有 DISPLAY 时真窗口常驻 10 秒后测量并 SIGTERM 验证 5 秒内退出；
# 无 DISPLAY 时退化为离屏截图模式（--demo chats --screenshot）的峰值，仅作参考，不等于真窗口常驻内存。
# 输出：首行显示缩放；每次测量一行 `lanmsgd private_kb=<n> pss_kb=<n> rss_kb=<n>`；
# 末尾 `RESULT {json}`。lanmsgd private 超阈值或界面进程 5 秒内未退出则非零退出。
set -euo pipefail

DATA_DIR="${1:?用法: measure-memory.sh <data_dir> [seconds=60]}"
SECONDS_IDLE="${2:-60}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="${LANMSG_BIN_DIR:-$ROOT/target/release}"
BIN_D="$BIN_DIR/lanmsgd"
BIN_CLI="$BIN_DIR/lanmsg-cli"
BIN_UI="$BIN_DIR/lanmsg"
CPU_SECONDS="${CPU_SECONDS:-30}"
THRESHOLD_KB="${THRESHOLD_KB:-15360}"
MSG_COUNT="${MSG_COUNT:-1000}"

for b in "$BIN_D" "$BIN_CLI" "$BIN_UI"; do
    [ -x "$b" ] || { echo "缺少二进制 $b（请先 cargo build --release -p lanmsgd -p lanmsg-cli -p lanmsg-desktop）" >&2; exit 1; }
done

mkdir -p "$DATA_DIR/run"
# Linux 端点在 XDG_RUNTIME_DIR 下，socket 路径有长度上限（108），请使用较短的 data_dir
export XDG_RUNTIME_DIR="$DATA_DIR/run"
cli() { "$BIN_CLI" --data-dir "$DATA_DIR" "$@"; }

echo "display_scale: GDK_SCALE=${GDK_SCALE:-unset} QT_SCALE_FACTOR=${QT_SCALE_FACTOR:-unset} DISPLAY=${DISPLAY:-unset}"

DPID=""
UPID=""
cleanup() {
    [ -n "$UPID" ] && kill "$UPID" 2>/dev/null || true
    [ -n "$DPID" ] && kill "$DPID" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT

# smaps_rollup 字段（KB）
field() { awk -v k="$2:" '$1==k {print $2}' "/proc/$1/smaps_rollup"; }
# private = Private_Clean + Private_Dirty
sample() {
    local pid="$1" pc pd
    pc=$(field "$pid" Private_Clean)
    pd=$(field "$pid" Private_Dirty)
    echo "$((pc + pd)) $(field "$pid" Pss) $(field "$pid" Rss)"
}
# CPU 时间（clock tick）：utime+stime。comm 可含空格，先去掉到最后一个 ')'。
cpu_ticks() { sed 's/^.*) //' "/proc/$1/stat" | awk '{print $12 + $13}'; }

RESULTS=()
DPRIV_MAX=0
record() { # name private pss rss
    echo "$1 private_kb=$2 pss_kb=$3 rss_kb=$4"
    RESULTS+=("{\"name\":\"$1\",\"private_kb\":$2,\"pss_kb\":$3,\"rss_kb\":$4}")
}
measure_daemon() { # label
    local p s r
    read -r p s r < <(sample "$DPID")
    record "lanmsgd[$1]" "$p" "$s" "$r"
    [ "$p" -gt "$DPRIV_MAX" ] && DPRIV_MAX=$p
    return 0
}

# ---- lanmsgd ----
DARGS=(--no-multicast --port 0 --data-dir "$DATA_DIR")
MODE=tray
if [ -z "${DISPLAY:-}" ]; then MODE=headless; DARGS+=(--headless); fi
"$BIN_D" "${DARGS[@]}" >/dev/null 2>&1 &
DPID=$!
for _ in $(seq 1 50); do cli self >/dev/null 2>&1 && break; sleep 0.2; done
cli self >/dev/null 2>&1 || { echo "lanmsgd 未就绪" >&2; exit 1; }
if ! cli self | grep -q 'initialized: true'; then cli init --name 内存测试 --device-name 测试机 >/dev/null; fi
echo "lanmsgd 模式: $MODE（pid $DPID）"

sleep "$SECONDS_IDLE"
measure_daemon "empty,${SECONDS_IDLE}s"

IDLE_CPU_JSON="null"
if [ "$CPU_SECONDS" -gt 0 ]; then
    t0=$(cpu_ticks "$DPID")
    sleep "$CPU_SECONDS"
    t1=$(cpu_ticks "$DPID")
    hz=$(getconf CLK_TCK)
    # 毫秒 = ticks * 1000 / hz
    ms=$(((t1 - t0) * 1000 / hz))
    echo "lanmsgd idle_cpu window_s=$CPU_SECONDS cpu_ticks=$((t1 - t0)) cpu_ms=$ms"
    IDLE_CPU_JSON="{\"window_s\":$CPU_SECONDS,\"cpu_ticks\":$((t1 - t0)),\"clk_tck\":$hz,\"cpu_ms\":$ms}"
fi

CONV=$(cli convs | awk 'NR==2 {print $1}')
[ -n "$CONV" ] || { echo "取不到会话 ID" >&2; exit 1; }
for i in $(seq 1 "$MSG_COUNT"); do cli send "$CONV" "内存测试消息 $i" >/dev/null; done
echo "已写入 $MSG_COUNT 条消息"
sleep "$SECONDS_IDLE"
measure_daemon "msgs${MSG_COUNT},${SECONDS_IDLE}s"

# ---- 界面进程 ----
UI_JSON="null"
UI_FAIL=0
if [ -n "${DISPLAY:-}" ]; then
    "$BIN_UI" --data-dir "$DATA_DIR" >/dev/null 2>&1 &
    UPID=$!
    sleep 10
    if kill -0 "$UPID" 2>/dev/null; then
        read -r p s r < <(sample "$UPID")
        record "lanmsg[window,10s]" "$p" "$s" "$r"
        t0=$(cpu_ticks "$UPID")
        sleep 10
        t1=$(cpu_ticks "$UPID")
        hz=$(getconf CLK_TCK)
        echo "lanmsg idle_cpu window_s=10 cpu_ticks=$((t1 - t0)) cpu_ms=$(((t1 - t0) * 1000 / hz))"
        kill -TERM "$UPID"
        exited=0
        for _ in $(seq 1 50); do
            kill -0 "$UPID" 2>/dev/null || { exited=1; break; }
            sleep 0.1
        done
        if [ "$exited" = 1 ]; then echo "lanmsg SIGTERM 后 5 秒内退出: 是"; UI_JSON="{\"mode\":\"window\",\"exit_within_5s\":true}"
        else echo "lanmsg SIGTERM 后 5 秒内退出: 否" >&2; UI_FAIL=1; UI_JSON="{\"mode\":\"window\",\"exit_within_5s\":false}"; fi
        UPID=""
    else
        echo "lanmsg 窗口进程提前退出" >&2; UI_FAIL=1
    fi
else
    # 离屏截图模式：进程短命，轮询取峰值（private/rss 取各自最大值）
    PNG="$DATA_DIR/mem-shot.png"
    "$BIN_UI" --demo chats --screenshot "$PNG" >/dev/null 2>&1 &
    UPID=$!
    mp=0; ms=0; mr=0; hwm=0
    while kill -0 "$UPID" 2>/dev/null; do
        if out=$(sample "$UPID" 2>/dev/null) && [ -n "$out" ]; then
            read -r p s r <<<"$out"
            [ "$p" -gt "$mp" ] && mp=$p
            [ "$s" -gt "$ms" ] && ms=$s
            [ "$r" -gt "$mr" ] && mr=$r
            h=$(awk '$1=="VmHWM:" {print $2}' "/proc/$UPID/status" 2>/dev/null || true)
            [ -n "$h" ] && [ "$h" -gt "$hwm" ] && hwm=$h
        fi
        sleep 0.02
    done
    wait "$UPID" 2>/dev/null || true
    UPID=""
    record "lanmsg[offscreen-screenshot-peak]" "$mp" "$ms" "$mr"
    echo "lanmsg offscreen vmhwm_kb=$hwm（离屏截图峰值，仅供参考，不等于真窗口常驻内存；真窗口与 10Hz 定时器空闲 CPU 需有 DISPLAY 的真机测）"
    UI_JSON="{\"mode\":\"offscreen\",\"vmhwm_kb\":$hwm,\"note\":\"offscreen screenshot peak, not resident window\"}"
fi

# ---- 汇总 ----
JOINED=$(IFS=,; echo "${RESULTS[*]}")
JSON="{\"lanmsgd_mode\":\"$MODE\",\"display\":\"${DISPLAY:-}\",\"gdk_scale\":\"${GDK_SCALE:-}\",\"threshold_kb\":$THRESHOLD_KB,\"measurements\":[$JOINED],\"lanmsgd_idle_cpu\":$IDLE_CPU_JSON,\"ui\":$UI_JSON}"
echo "RESULT $JSON"
[ -n "${RESULT_JSON:-}" ] && echo "$JSON" >"$RESULT_JSON"

if [ "$DPRIV_MAX" -gt "$THRESHOLD_KB" ]; then
    echo "FAIL: lanmsgd private ${DPRIV_MAX} KB 超过阈值 ${THRESHOLD_KB} KB" >&2
    exit 1
fi
[ "$UI_FAIL" = 0 ] || exit 1
echo "MEASURE OK"

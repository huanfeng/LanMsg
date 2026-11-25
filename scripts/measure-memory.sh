#!/usr/bin/env bash
# P3 内存与空闲 CPU 测量（Linux）。
#
# 用法：
#   bash scripts/measure-memory.sh <data_dir> [seconds=60]
# 环境变量：
#   LANMSG_BIN_DIR   二进制目录（默认 <仓库>/target/release；建议 release 构建）
#   CPU_SECONDS      lanmsgd 空闲 CPU 观察窗口秒数（默认 30，0 跳过）
#   HEAP_THRESHOLD_KB  lanmsgd 匿名堆内存（Private_Dirty）阈值，超过即失败（默认 8192）。
#                    这是真正由 LanMsg 代码持有的内存，增长就是回归；2026-11-25 同机实测
#                    P3 版 ≈3.0 MB、P5 后 ≈3.7 MB（写入 1000 条消息后）。
#   THRESHOLD_KB     lanmsgd private 总量（Private_Clean+Private_Dirty）参考线（默认 15360），
#                    超过只告警不失败：其中 Private_Clean 含只被本进程映射的二进制代码页，
#                    读数取决于“当时是否有别的进程同时映射同一个二进制”（同一状态实测在
#                    2.2~15.8 MB 间摆动），不是可靠的回归信号；二进制变大（P5 后 +5 MB）也会抬高它。
#   MSG_COUNT        第二次测量前写入的消息条数（默认 1000）
#   RESULT_JSON      若设置，机器可读结果（JSON）另存到该文件
#
# 流程：启动 lanmsgd（有 DISPLAY 为常驻托盘模式，否则 --headless）→ 空闲 <seconds> 秒测一次 →
# 空闲 CPU 窗口 → 向“我的设备”会话写 MSG_COUNT 条 → 再等 <seconds> 秒测一次 →
# 界面进程：有 DISPLAY 时真窗口常驻 10 秒后测量并 SIGTERM 验证 5 秒内退出；
# 无 DISPLAY 时退化为离屏截图模式（--demo chats --screenshot）的峰值，仅作参考，不等于真窗口常驻内存。
# 输出：首行显示缩放；每次测量一行 `lanmsgd private_kb=<n> pss_kb=<n> rss_kb=<n> dirty_kb=<n>`；
# 末尾 `RESULT {json}`。lanmsgd 匿名堆（dirty_kb）超 HEAP_THRESHOLD_KB 或界面进程 5 秒内未退出则非零退出；
# private 总量超 THRESHOLD_KB 只打印 WARN。
set -euo pipefail

DATA_DIR="${1:?用法: measure-memory.sh <data_dir> [seconds=60]}"
SECONDS_IDLE="${2:-60}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$ROOT"
# shellcheck source=lib/e2e-bins.sh
source "$ROOT/scripts/lib/e2e-bins.sh"
e2e_resolve_bins release   # 目录：LANMSG_BIN_DIR > $CARGO_TARGET_DIR/release > $REPO/target/release
BIN_UI="$BIN_DIR/lanmsg"
CPU_SECONDS="${CPU_SECONDS:-30}"
THRESHOLD_KB="${THRESHOLD_KB:-15360}"
HEAP_THRESHOLD_KB="${HEAP_THRESHOLD_KB:-8192}"
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
    echo "$((pc + pd)) $(field "$pid" Pss) $(field "$pid" Rss) $pd"
}
# CPU 时间（clock tick）：utime+stime。comm 可含空格，先去掉到最后一个 ')'。
cpu_ticks() { sed 's/^.*) //' "/proc/$1/stat" | awk '{print $12 + $13}'; }

RESULTS=()
DPRIV_MAX=0
DDIRTY_MAX=0
record() { # name private pss rss [dirty]
    echo "$1 private_kb=$2 pss_kb=$3 rss_kb=$4 dirty_kb=${5:-0}"
    RESULTS+=("{\"name\":\"$1\",\"private_kb\":$2,\"pss_kb\":$3,\"rss_kb\":$4,\"dirty_kb\":${5:-0}}")
}
measure_daemon() { # label
    local p s r d
    read -r p s r d < <(sample "$DPID")
    record "lanmsgd[$1]" "$p" "$s" "$r" "$d"
    [ "$p" -gt "$DPRIV_MAX" ] && DPRIV_MAX=$p
    [ "$d" -gt "$DDIRTY_MAX" ] && DDIRTY_MAX=$d
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
        read -r p s r _ < <(sample "$UPID")
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
            read -r p s r _ <<<"$out"
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

if [ "$DDIRTY_MAX" -gt "$HEAP_THRESHOLD_KB" ]; then
    echo "FAIL: lanmsgd 匿名堆 ${DDIRTY_MAX} KB 超过阈值 ${HEAP_THRESHOLD_KB} KB" >&2
    exit 1
fi
if [ "$DPRIV_MAX" -gt "$THRESHOLD_KB" ]; then
    echo "WARN: lanmsgd private 总量 ${DPRIV_MAX} KB 超过参考线 ${THRESHOLD_KB} KB（含二进制代码页，读数随并发进程数摆动；以匿名堆 ${DDIRTY_MAX} KB 为准）" >&2
fi
[ "$UI_FAIL" = 0 ] || exit 1
echo "MEASURE OK"

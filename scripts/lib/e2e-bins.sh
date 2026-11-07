#!/usr/bin/env bash
# e2e 脚本共用：定位 lanmsgd / lanmsg-cli 二进制，并拒绝使用过期的二进制。
# 用法（先设好 REPO）：source scripts/lib/e2e-bins.sh; e2e_resolve_bins
# 目录优先级：LANMSG_BIN_DIR > $CARGO_TARGET_DIR/debug > $REPO/target/debug。
# 设置 BIN_DIR、BIN_D（守护进程）、BIN_CLI。
# 可选参数：构建档位，默认 debug（性能/内存脚本传 release）。

e2e_resolve_bins() {
    BIN_DIR="${LANMSG_BIN_DIR:-${CARGO_TARGET_DIR:-$REPO/target}/${1:-debug}}"
    BIN_D="$BIN_DIR/lanmsgd"
    BIN_CLI="$BIN_DIR/lanmsg-cli"
}

# 检查二进制存在，打印路径与修改时间；二进制依赖的任一本仓库源码（取自 cargo 生成的
# <二进制>.d 依赖清单）比它新则报错退出，避免拿旧二进制跑出假的 ACCEPTANCE OK。
# 没有 .d 清单时退化为与 crates/ 下全部 .rs 比较。
e2e_check_bins() {
    local b newer dep
    for b in "$BIN_D" "$BIN_CLI"; do
        if [ ! -x "$b" ]; then
            echo "缺少二进制：$b，请先 cargo build -p lanmsgd -p lanmsg-cli" >&2
            exit 2
        fi
        echo "二进制：$b（修改时间 $(date -r "$b" '+%F %T')）"
        newer=""
        if [ -f "$b.d" ]; then
            for dep in $(grep -o "$REPO/[^ :]*\.rs" "$b.d" | sort -u); do
                if [ "$dep" -nt "$b" ]; then newer="$dep"; break; fi
            done
        else
            newer="$(find "$REPO/crates" -name '*.rs' -newer "$b" -print -quit)"
        fi
        if [ -n "$newer" ]; then
            echo "二进制已过期：$b 比源码 $newer 旧，请重新 cargo build -p lanmsgd -p lanmsg-cli（产物在 CARGO_TARGET_DIR 下时，设置 CARGO_TARGET_DIR 或 LANMSG_BIN_DIR）" >&2
            exit 2
        fi
    done
}

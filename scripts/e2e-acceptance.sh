#!/usr/bin/env bash
# P1/P2 单机端到端功能验收：真实 lanmsgd 进程 + lanmsg-cli，把手工验收项尽量自动化。
#
# 用法：
#   bash scripts/e2e-acceptance.sh                 # 跑全部场景
#   bash scripts/e2e-acceptance.sh --only <场景名>   # 只跑一个场景（可重复）
#   KEEP=1 bash scripts/e2e-acceptance.sh          # 结束后保留临时目录（调试用）
#
# 场景：text_chat pair_qr pair_profile pair_sas offline_catchup sibling_dm_sync
#       stranger_request file_transfer folder_transfer file_resume revoke recall_read bench_local
# 末行 `ACCEPTANCE OK` 表示全部通过；任一场景失败则打印场景名与日志并非零退出
# （末行为 `ACCEPTANCE FAILED`）。
#
# 隔离方式同 e2e-two-daemons.sh：每实例独立 data-dir 与 XDG_RUNTIME_DIR，
# 默认关组播，靠 --peer 静态直连（pair_sas 例外，见该场景注释）。
# 每个场景在子进程中运行并带超时。
set -euo pipefail

SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
REPO="${REPO:-$(cd "$(dirname "$SELF")/.." && pwd)}"
# shellcheck source=lib/e2e-bins.sh
source "$REPO/scripts/lib/e2e-bins.sh"
e2e_resolve_bins  # 目录：LANMSG_BIN_DIR > $CARGO_TARGET_DIR/debug > $REPO/target/debug

ALL_SCENARIOS=(text_chat pair_qr pair_profile pair_sas offline_catchup sibling_dm_sync stranger_request file_transfer folder_transfer file_resume revoke recall_read bench_local)

# 各场景超时（秒）。
scenario_timeout() {
    case "$1" in
    file_resume) echo 300 ;;
    bench_local) echo 300 ;;
    offline_catchup) echo 180 ;;
    *) echo 120 ;;
    esac
}

# ====================================================================
# 子进程侧：单个场景运行时（--child 模式）
# ====================================================================

declare -A NPORT=() NPID=()
BGPIDS=()
SC=""
PORT_BASE=0

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

# 声明节点（分配端口），不启动。
new_node() { NPORT[$1]=$((PORT_BASE + ${#NPORT[@]})); }

# CLI 包装：c <节点> <子命令…>
c() {
    local n="$1"
    shift
    XDG_RUNTIME_DIR="$SC/$n/run" "$BIN_CLI" --data-dir "$SC/$n" "$@"
}

# 启动节点：start_node <节点> [静态对端节点…]；日志追加到 $SC/<节点>.log。
start_node() {
    local n="$1"
    shift
    mkdir -p "$SC/$n"
    local args=(--port "${NPORT[$n]}" --data-dir "$SC/$n") p
    if [ "${MCAST:-0}" != 1 ]; then args+=(--no-multicast); fi
    for p in "$@"; do args+=(--peer "127.0.0.1:${NPORT[$p]}"); done
    XDG_RUNTIME_DIR="$SC/$n/run" "$BIN_D" "${args[@]}" >>"$SC/$n.log" 2>&1 &
    NPID[$n]=$!
    local i
    for i in $(seq 1 100); do
        if c "$n" self >/dev/null 2>&1; then return 0; fi
        sleep 0.1
    done
    fail "节点 $n 的 IPC 未就绪"
}

# 优雅关闭 / 强杀节点。
stop_node() {
    c "$1" shutdown >/dev/null 2>&1 || true
    wait "${NPID[$1]}" 2>/dev/null || true
}
kill_node() {
    kill -9 "${NPID[$1]}" 2>/dev/null || true
    wait "${NPID[$1]}" 2>/dev/null || true
}

# 后台跑 CLI，输出写入文件：bg <输出文件> <节点> <子命令…>
bg() {
    local out="$1"
    shift
    c "$@" >"$out" 2>&1 &
    BGPIDS+=($!)
}

child_cleanup() {
    local p
    for p in "${BGPIDS[@]:-}"; do if [ -n "$p" ]; then kill "$p" 2>/dev/null || true; fi; done
    for p in "${NPID[@]:-}"; do if [ -n "$p" ]; then kill -9 "$p" 2>/dev/null || true; fi; done
}

# 轮询直至命令成功：wait_for <秒> <描述> <命令…>
wait_for() {
    local secs="$1" desc="$2"
    shift 2
    local end=$((SECONDS + secs))
    while ((SECONDS < end)); do
        if "$@" >/dev/null 2>&1; then return 0; fi
        sleep 0.25
    done
    fail "等待超时（${secs}s）：$desc"
}

uid() { c "$1" self | awk '/^user_id:/{print $2}'; }
did() { c "$1" self | awk '/^device_id:/{print $2}'; }

# 会话中的文本（每行一条，已去掉 [时间] 作者: 前缀），按 log 顺序；
# 系统消息（CLI 加 [system] 前缀，如 "contact added: X"）是各设备本地生成的，不属于用户文字，滤掉。
texts() { c "$1" log "$2" --limit 500 | grep -v '<unsupported message type>' | sed -E 's/^\[[0-9:]+\] [^:]*: //' | awk '!/^\[system\] /'; }
has_text() { texts "$1" "$2" | grep -qxF -- "$3"; }

# 对端在线：peer_seen <节点> <显示名>
peer_seen() { c "$1" peers | grep -qF -- "$2"; }

# 节点 watch 事件流文件里是否有某消息的某状态（MessageUpdated）。
msg_state_seen() { grep '"MessageUpdated"' "$1" | grep -F "\"id\":\"$2\"" | grep -q "\"state\":\"$3\""; }

# 初始化一个独立用户：mk_user <节点> <显示名> [静态对端节点…]
mk_user() {
    local n="$1" name="$2"
    shift 2
    new_node "$n"
    start_node "$n" "$@"
    c "$n" init --name "$name" --device-name "${n}机" >/dev/null
}

# 互加联系人：link <请求方> <接受方> <接受方显示名>，返回时双方可 DM。
link() {
    local a="$1" b="$2" bname="$3"
    wait_for 30 "$a 发现 $b" peer_seen "$a" "$bname"
    c "$a" request-contact "$(uid "$b")" >/dev/null
    local req="" i
    for i in $(seq 1 100); do
        req="$(c "$b" requests | awk 'NR==1 && $1 ~ /^[0-9a-f]+$/ {print $1}')"
        if [ -n "$req" ]; then break; fi
        sleep 0.2
    done
    [ -n "$req" ] || fail "$b 未收到 $a 的联系人请求"
    c "$b" respond "$req" add >/dev/null
}

# 同一用户加设备（二维码方式）：pair_qr_join <已有设备> <新节点（已启动且 --peer 已有设备）>
pair_qr_join() {
    local a="$1" n="$2"
    c "$n" join --device-name "${n}机" >/dev/null
    local ps="$SC/ps_$n.out" pj="$SC/pj_$n.out"
    : >"$ps"
    bg "$ps" "$a" pair-start
    wait_for 20 "$a 打印 qr_uri" grep -q '^qr_uri:' "$ps"
    local uri
    uri="$(awk '/^qr_uri:/{print $2}' "$ps")"
    bg "$pj" "$n" pair-join "$uri"
    wait_for 30 "$a 配对完成" grep -q 'pairing complete' "$ps"
    wait_for 30 "$n 配对完成" grep -q 'pairing complete' "$pj"
}

# 取「我的设备」会话 ID。
selfconv() { c "$1" convs | awk '/My devices/{print $1; exit}'; }

# 设备数（devices 输出去掉表头后的行数）。
ndevices() { c "$1" devices | awk 'NR>1 && NF' | wc -l | tr -d ' '; }
# wait_for 的命令只求值一次，需要重复求值的条件必须封装成函数。
ndev_is() { [ "$(ndevices "$1")" = "$2" ]; }
self_name_is() { [ "$(c "$1" self | awk '/^display_name:/{print $2}')" = "$2" ]; }
files_listed() { [ "$(c "$1" files-of "$2" | grep -c -e d1.bin -e s1.bin -e root.bin -e note.txt)" = 4 ]; }
hash_ready() { [ -n "$(hash_of "$1" "$2" "$3")" ]; }

# ---------------------------------------------------------------- 场景 1
sc_text_chat() {
    mk_user a 甲
    mk_user b 乙 a
    link b a 甲
    local conv
    conv="$(c b dm "$(uid a)")"
    [ -n "$conv" ] || fail "未取得会话 ID"
    local expect="" i
    for i in 1 2 3 4 5 6; do
        local from=b to=a
        if ((i % 2 == 0)); then
            from=a
            to=b
        fi
        c "$from" send "$conv" "消息$i" >/dev/null
        # 逐条等对端可见，保证因果顺序确定。
        wait_for 20 "$to 收到 消息$i" has_text "$to" "$conv" "消息$i"
        expect+="消息$i"$'\n'
    done
    local who got
    for who in a b; do
        got="$(texts "$who" "$conv")"$'\n'
        [ "$got" = "$expect" ] || fail "$who 的消息顺序/内容不符。期望：$(echo "$expect" | tr '\n' ',') 实际：$(echo "$got" | tr '\n' ',')"
    done
    # CLI 的输出被管道对端提前关闭（`| head`、`| grep -q`）时不得 panic：读端立即退出，
    # CLI 写 stdout 必遇 EPIPE，stderr 里不应出现 panic 信息。
    c a log "$conv" --limit 500 2>"$SC/sigpipe.err" | true || true
    if grep -qE 'panicked|failed printing' "$SC/sigpipe.err"; then
        fail "CLI 在管道提前关闭时 panic：$(cat "$SC/sigpipe.err")"
    fi
    echo "text_chat：双方各 6 条且顺序一致"
}

# ---------------------------------------------------------------- 场景 2
sc_pair_qr() {
    mk_user a 甲
    new_node b
    start_node b a
    pair_qr_join a b
    [ "$(uid b)" = "$(uid a)" ] || fail "B 的 user_id 与 A 不同"
    wait_for 20 "A 看到 2 台设备" ndev_is a 2
    wait_for 20 "B 看到 2 台设备" ndev_is b 2
    c a devices | grep -q 'a机' || fail "A 的 devices 缺少 a机"
    c a devices | grep -q 'b机' || fail "A 的 devices 缺少 b机"
    c b devices | grep -q 'a机' || fail "B 的 devices 缺少 a机"
    c b devices | grep -q 'b机' || fail "B 的 devices 缺少 b机"
    echo "pair_qr：两端同一 user，devices 均列出两台"
}

# 场景 2b：新设备配对后继承用户资料（self 的 display_name 与 A 一致）。
sc_pair_profile() {
    mk_user a 甲
    new_node b
    start_node b a
    pair_qr_join a b
    wait_for 15 "B 的 display_name 与 A 一致" self_name_is b 甲
    echo "pair_profile：B 继承显示名 甲"
}

# ---------------------------------------------------------------- 场景 3
# sas_pair <已有设备> <新节点> <A 端答案> <B 端答案>
sas_pair() {
    local a="$1" n="$2" ans_a="$3" ans_n="$4"
    c "$n" join --device-name "${n}机" >/dev/null
    local ps="$SC/ps_$n.out" pj="$SC/pj_$n.out" wn="$SC/watch_$n.out"
    : >"$ps"
    : >"$pj"
    : >"$wn"
    bg "$wn" "$n" watch
    bg "$SC/watch_$a.out" "$a" watch
    bg "$ps" "$a" pair-start --sas
    wait_for 20 "$a 打印 session" grep -q '^session:' "$ps"
    local sess_start
    sess_start="$(awk '/^session:/{print $2}' "$ps")"
    # SAS 方式按设备 ID 加入，要求 A 在「组播发现表」里（pairing.rs 的 discovered.get），
    # 静态 --peer 直连不会填充该表，所以本场景的节点需开启组播；
    # 发现表填充有延迟，NotFound/PeerOffline 时重试。
    local tries=0
    while :; do
        bg "$pj" "$n" pair-join "$(did "$a")"
        sleep 1
        if ! grep -qE 'NotFound|PeerOffline' "$pj"; then break; fi
        tries=$((tries + 1))
        [ "$tries" -lt 25 ] || fail "$n 始终发现不了 $a（组播发现表为空？）：$(cat "$pj")"
        sleep 1
    done
    # 数字与会话 ID 直接取自 pair-start / pair-join 的输出（CLI 先订阅事件再发请求，
    # 响应方事件会话与 pair-start 返回值一致）。
    wait_for 30 "$a 打印比对数字" grep -q 'compare digits: ' "$ps"
    wait_for 30 "$n 打印比对数字" grep -q 'compare digits: ' "$pj"
    local code_a code_n sess_n
    code_a="$(grep -o 'compare digits: [0-9]*' "$ps" | head -1 | sed -E 's/.*: //')"
    code_n="$(grep -o 'compare digits: [0-9]*' "$pj" | head -1 | sed -E 's/.*: //')"
    [ "${#code_a}" = 6 ] || fail "A 端数字不是 6 位：'$code_a'"
    [ "$code_a" = "$code_n" ] || fail "两端数字不同：A=$code_a N=$code_n"
    sess_n="$(awk '/^session:/{print $2}' "$pj" | head -1)"
    [ -n "$sess_n" ] || fail "pair-join 未打印 session：$(cat "$pj")"
    c "$a" pair-confirm "$sess_start" "$ans_a" --code "$code_a" >/dev/null
    c "$n" pair-confirm "$sess_n" "$ans_n" --code "$code_n" >/dev/null
    SAS_CODE="$code_a"
}

sc_pair_sas() {
    MCAST=1
    mk_user a 甲
    new_node b
    start_node b a
    sas_pair a b yes yes
    wait_for 30 "A 端 Done" grep -q '"PairStatus".*"Done"' "$SC/watch_a.out"
    wait_for 30 "B 端 Done" grep -q '"PairStatus".*"Done"' "$SC/watch_b.out"
    [ "$(uid b)" = "$(uid a)" ] || fail "SAS 配对后 user_id 不同"
    echo "pair_sas：两端数字 $SAS_CODE 相同，yes/yes 后 Done"

    # 否定分支：一端回答 no，应 Failed（用新的独立用户，避免受已配对状态影响）。
    mk_user x 丙
    new_node y
    start_node y x
    sas_pair x y yes no
    wait_for 30 "x 端 Failed" grep -q '"PairStatus".*"Failed"' "$SC/watch_x.out"
    wait_for 30 "y 端 Failed" grep -q '"PairStatus".*"Failed"' "$SC/watch_y.out"
    if grep -q '"PairStatus".*"Done"' "$SC/watch_x.out" "$SC/watch_y.out"; then fail "一端 no 却出现 Done"; fi
    [ "$(ndevices x)" = 1 ] || fail "一端 no 后 x 的设备数应仍为 1，实际 $(ndevices x)"
    echo "pair_sas：一端 no 后两端 Failed，设备数未增加"
}

# ---------------------------------------------------------------- 场景 4
sc_offline_catchup() {
    # A 为中心；B、C 同用户（经配对）；D 为另一用户。B/C/D 都直连 A，C 同时直连 B。
    mk_user a 甲
    new_node b
    new_node c
    new_node d
    start_node b a
    pair_qr_join a b
    start_node c a b
    pair_qr_join a c
    start_node d a
    c d init --name 丁 --device-name d机 >/dev/null
    wait_for 20 "三台设备互认" ndev_is c 3
    link d a 甲
    local dm self
    dm="$(c a dm "$(uid d)")"
    self="$(selfconv a)"
    [ -n "$dm" ] && [ -n "$self" ] || fail "未取得会话 ID（dm=$dm self=$self）"
    # 先在线时发一条，确认 C 的通路正常。
    c a send "$self" "在线消息" >/dev/null
    wait_for 30 "C 收到在线消息" has_text c "$self" "在线消息"

    stop_node c
    local i
    for i in 1 2 3; do
        c a send "$self" "A自$i" >/dev/null
        c b send "$self" "B自$i" >/dev/null
    done
    # 与丁的私聊：B 尚无该会话（见 sibling_dm_sync 暴露的缺陷），故只由 A 与丁互发。
    c a send "$dm" "A给丁1" >/dev/null
    wait_for 30 "D 收到 A给丁1" has_text d "$dm" "A给丁1"
    c d send "$dm" "丁给甲1" >/dev/null
    wait_for 30 "A 收到丁给甲1" has_text a "$dm" "丁给甲1"

    start_node c a b
    local m
    for m in "A自1" "A自2" "A自3" "B自1" "B自2" "B自3"; do
        wait_for 60 "C 补齐我的设备会话：$m" has_text c "$self" "$m"
    done
    for m in "A给丁1" "丁给甲1"; do
        wait_for 60 "C 补齐与丁的私聊：$m" has_text c "$dm" "$m"
    done
    # C 与 A 在两个会话里的消息集合相同。
    [ "$(texts c "$self" | sort)" = "$(texts a "$self" | sort)" ] || fail "C 与 A 的我的设备会话内容不一致"
    [ "$(texts c "$dm" | sort)" = "$(texts a "$dm" | sort)" ] || fail "C 与 A 的私聊内容不一致"
    echo "offline_catchup：C 重启后补齐 我的设备 $(texts c "$self" | wc -l | tr -d ' ') 条、私聊 $(texts c "$dm" | wc -l | tr -d ' ') 条"
}

# 场景 4b：同用户另一台在线设备应同步到「与第三方的私聊」（会话与消息）。
sc_sibling_dm_sync() {
    mk_user a 甲
    new_node b
    start_node b a
    pair_qr_join a b
    mk_user d 丁 a
    link d a 甲
    local dm
    dm="$(c a dm "$(uid d)")"
    c a send "$dm" "A发给丁" >/dev/null
    wait_for 20 "D 收到 A发给丁" has_text d "$dm" "A发给丁"
    c d send "$dm" "丁发给甲" >/dev/null
    wait_for 20 "A 收到 丁发给甲" has_text a "$dm" "丁发给甲"
    wait_for 30 "在线的同用户设备 B 出现与丁的私聊（含 A 的外发消息）" has_text b "$dm" "A发给丁"
    wait_for 30 "B 出现 丁发给甲" has_text b "$dm" "丁发给甲"
    echo "sibling_dm_sync：B 同步到与丁的私聊"
}

# ---------------------------------------------------------------- 场景 5
sc_stranger_request() {
    mk_user a 甲
    mk_user e 戊 a
    mk_user f 己 a
    wait_for 30 "E 发现 A" peer_seen e 甲
    wait_for 30 "F 发现 A" peer_seen f 甲
    local conv
    conv="$(c e dm "$(uid a)")"
    c e send "$conv" "陌生人你好" >/dev/null
    local req="" i
    for i in $(seq 1 100); do
        req="$(c a requests | awk '$1 ~ /^[0-9a-f]+$/ {print $1; exit}')"
        if [ -n "$req" ]; then break; fi
        sleep 0.2
    done
    [ -n "$req" ] || fail "A 未出现陌生人请求"
    c a requests | grep -q '戊' || fail "请求里没有来自 戊 的条目：$(c a requests)"
    if has_text a "$conv" "陌生人你好"; then fail "respond 之前陌生人消息不应可见"; fi
    c a respond "$req" once >/dev/null
    wait_for 20 "respond once 后消息可见" has_text a "$conv" "陌生人你好"
    echo "stranger_request：请求出现，respond once 后消息可见"

    # block：F 第一次发消息 → 请求 → block；F 再发不应产生新请求/可见消息。
    local conv2 req2=""
    conv2="$(c f dm "$(uid a)")"
    c f send "$conv2" "己的第一条" >/dev/null
    for i in $(seq 1 100); do
        req2="$(c a requests | awk '/己/ && $1 ~ /^[0-9a-f]+$/ {print $1; exit}')"
        if [ -n "$req2" ]; then break; fi
        sleep 0.2
    done
    [ -n "$req2" ] || fail "A 未出现 己 的请求"
    c a respond "$req2" block >/dev/null
    c f send "$conv2" "己的第二条" >/dev/null || true
    sleep 4
    if c a requests | grep -q '己'; then fail "block 之后仍出现 己 的新请求：$(c a requests)"; fi
    if has_text a "$conv2" "己的第二条"; then fail "block 之后 己 的消息仍可见"; fi
    echo "stranger_request：block 后再发无新请求、消息不可见"
}

# ---------------------------------------------------------------- 场景 6
# 取节点下载目录。
dl_dir() { c "$1" settings | awk -F': ' '/^download_dir:/{print $2}'; }
# files-of 中按文件名取 hash。
hash_of() { c "$1" files-of "$2" | awk -v n="$3" '$2==n{print $1; exit}'; }
# 在下载目录下找文件（按名字）。
find_dl() { find "$(dl_dir "$1")" -type f -name "$2" 2>/dev/null | head -1; }
sha() { sha256sum <"$1" | cut -d' ' -f1; }

# 文件传输场景公共准备：a、b 互为联系人，关自动下载，下载目录放到场景目录下；
# 设置 CONV 与 SRC（源文件目录）。
ft_setup() {
    mk_user a 甲
    mk_user b 乙 a
    link b a 甲
    local n
    for n in a b; do
        c "$n" settings set auto_file_max 0 >/dev/null
        c "$n" settings set auto_image_max 0 >/dev/null
        c "$n" settings set download_dir "$SC/$n/downloads" >/dev/null
    done
    CONV="$(c a dm "$(uid b)")"
    SRC="$SC/src"
    mkdir -p "$SRC"
}

sc_file_transfer() {
    ft_setup
    head -c $((64 * 1024 * 1024)) /dev/urandom >"$SRC/big64.bin"
    head -c 30000 /dev/urandom >"$SRC/pic.png"
    c a send-files "$CONV" "$SRC/big64.bin" "$SRC/pic.png" >/dev/null
    wait_for 60 "B 列出 big64.bin" hash_ready b "$CONV" big64.bin
    wait_for 60 "B 列出 pic.png" hash_ready b "$CONV" pic.png
    local h1 h2 out f
    h1="$(hash_of b "$CONV" big64.bin)"
    h2="$(hash_of b "$CONV" pic.png)"
    out="$(c b download "$h1" --wait)" || fail "下载 big64.bin 失败：$out"
    echo "file_transfer：64MB $out"
    out="$(c b download "$h2" --wait)" || fail "下载 pic.png 失败：$out"
    f="$(find_dl b big64.bin)"
    [ -n "$f" ] || fail "下载目录里找不到 big64.bin（$(dl_dir b)）"
    [ "$(sha "$f")" = "$(sha "$SRC/big64.bin")" ] || fail "big64.bin sha256 不一致"
    f="$(find_dl b pic.png)"
    [ -n "$f" ] || fail "下载目录里找不到 pic.png"
    cmp -s "$f" "$SRC/pic.png" || fail "pic.png 内容不一致"
    echo "file_transfer：大文件与小图片 sha256 一致"
}

# 文件夹（含子目录）：下载后目录结构与内容应与源一致。
sc_folder_transfer() {
    ft_setup
    local fol="$SRC/项目目录"
    mkdir -p "$fol/sub/deep"
    head -c 1000000 /dev/urandom >"$fol/root.bin"
    head -c 2000000 /dev/urandom >"$fol/sub/s1.bin"
    head -c 3000 /dev/urandom >"$fol/sub/deep/d1.bin"
    printf 'hello\n' >"$fol/sub/deep/note.txt"
    c a send-folder "$CONV" "$fol" >/dev/null
    wait_for 60 "B 列出文件夹内 4 个文件" files_listed b "$CONV"
    local hash fname
    while read -r hash fname _; do
        [ -n "$hash" ] || continue
        c b download "$hash" --wait >/dev/null || fail "下载 $fname 失败"
    done < <(c b files-of "$CONV")
    # 先确认内容完整（每个源文件的 sha256 都能在下载目录里找到同名同内容文件），
    # 再严格比较目录结构。
    local rel f
    while read -r rel; do
        f="$(find "$(dl_dir b)" -type f -name "$(basename "$rel")" | head -1)"
        [ -n "$f" ] && [ "$(sha "$f")" = "$(sha "$fol/$rel")" ] || fail "文件内容缺失或不一致：$rel"
    done < <(cd "$fol" && find . -type f | sed 's#^\./##')
    echo "folder_transfer：4 个文件内容均完整一致"
    local got
    got="$(find "$(dl_dir b)" -type d -name '项目目录' | head -1)"
    [ -n "$got" ] || fail "目录结构不符：下载目录里没有 项目目录 文件夹；实际布局：$(cd "$(dl_dir b)" && find . -printf '%y:%p ' | head -c 600)"
    diff -r "$fol" "$got" >"$SC/folder.diff" 2>&1 || fail "文件夹目录结构/内容不一致：$(head -n 5 "$SC/folder.diff")"
    echo "folder_transfer：目录结构一致"
}

# 断点续传：下载 768MB 中途 kill -9 B 的守护进程，重启后续传，hash 正确。
sc_file_resume() {
    ft_setup
    head -c $((768 * 1024 * 1024)) /dev/urandom >"$SRC/huge.bin"
    c a send-files "$CONV" "$SRC/huge.bin" >/dev/null
    wait_for 120 "B 列出 huge.bin" hash_ready b "$CONV" huge.bin
    local hh out f
    hh="$(hash_of b "$CONV" huge.bin)"
    local wf="$SC/resume_watch.out"
    : >"$wf"
    bg "$wf" b watch
    sleep 0.3
    c b download "$hh" >/dev/null
    wait_for 60 "B 出现下载进度" grep -q TransferProgress "$wf"
    kill_node b
    if grep -q '"TransferFinished"' "$wf"; then
        fail "续传场景未能在传输中途 kill（传输在 kill 前已完成，文件过小/过快）"
    fi
    local done_before
    done_before="$(grep -o '"done":[0-9]*' "$wf" | tail -n 1 | cut -d: -f2)"
    echo "file_resume：已在传输中途 kill -9 B（$(grep -c TransferProgress "$wf") 条进度事件后，已完成 ${done_before:-?} 字节）"
    start_node b a
    wait_for 30 "B 重启后重连 A" peer_seen b 甲
    # 重启后的进度事件：首个 done 应 >0 才说明从断点续传（而非从头开始）。
    local wf2="$SC/resume_watch2.out"
    : >"$wf2"
    bg "$wf2" b watch
    sleep 0.3
    # 重启后再请求同一 hash，应续传并完成。
    [ "$(hash_of b "$CONV" huge.bin)" = "$hh" ] || fail "重启后 huge.bin 的 hash 不可见"
    out="$(c b download "$hh" --wait)" || fail "续传下载失败：$out"
    local done_after
    done_after="$(grep -o '"done":[0-9]*' "$wf2" | head -n 1 | cut -d: -f2)"
    echo "file_resume：重启后首个进度事件 done=${done_after:-无}（kill 前已完成 ${done_before:-?}）"
    if [ -n "${done_after:-}" ] && [ -n "${done_before:-}" ] && [ "$done_after" -lt "$done_before" ]; then
        fail "重启后进度回退（$done_after < $done_before），疑似未续传而是从头下载"
    fi
    f="$(find_dl b huge.bin)"
    [ -n "$f" ] || fail "续传后找不到 huge.bin"
    [ "$(sha "$f")" = "$(sha "$SRC/huge.bin")" ] || fail "续传后 huge.bin sha256 不一致"
    echo "file_resume：续传完成且 sha256 正确（$out）"
}

# ---------------------------------------------------------------- 场景 7
sc_revoke() {
    mk_user a 甲
    new_node b
    start_node b a
    pair_qr_join a b
    wait_for 20 "A 看到 2 台设备" ndev_is a 2
    local self
    self="$(selfconv a)"
    c a send "$self" "撤销前" >/dev/null
    wait_for 30 "B 收到撤销前消息" has_text b "$self" "撤销前"
    local bid
    bid="$(did b)"
    c a remove-device "$bid" >/dev/null || fail "remove-device 失败"
    # A 的 devices 不再有 b机，或显示为已移除。
    wait_for 20 "A 的 devices 不再列出 B" bash -c "! XDG_RUNTIME_DIR='$SC/a/run' '$BIN_CLI' --data-dir '$SC/a' devices | grep -q 'b机'"
    sleep 2
    c a send "$self" "撤销后" >/dev/null
    # 与另一用户的私聊也不应再投递给 B：建联系人 D。
    mk_user d 丁 a
    link d a 甲
    local dmconv
    dmconv="$(c a dm "$(uid d)")"
    c a send "$dmconv" "撤销后的私聊" >/dev/null
    wait_for 30 "D 收到私聊" has_text d "$dmconv" "撤销后的私聊"
    sleep 4
    if has_text b "$self" "撤销后"; then fail "B 被移除后仍收到 A 的新消息（我的设备会话）"; fi
    if c b convs | grep -qF "撤销后的私聊"; then fail "B 被移除后仍收到 A 的私聊消息"; fi
    echo "revoke：移除后 B 未收到新消息，A 的 devices 现有 $(ndevices a) 台"
}

# ---------------------------------------------------------------- 场景 8
sc_recall_read() {
    mk_user a 甲
    mk_user b 乙 a
    link b a 甲
    local conv
    conv="$(c a dm "$(uid b)")"
    local wa="$SC/watch_a.out" wb="$SC/watch_b.out"
    : >"$wa"
    : >"$wb"
    bg "$wa" a watch
    bg "$wb" b watch
    sleep 0.5
    local m1 m2
    m1="$(c a send "$conv" "要被读的")"
    m2="$(c a send "$conv" "要被撤回的")"
    wait_for 30 "B 收到两条" has_text b "$conv" "要被撤回的"
    # B 侧未读数 > 0，标记已读后归零。
    local unread
    unread="$(c b convs | awk -v c="$conv" '$1==c{print $3}')"
    [ "${unread:-0}" -ge 1 ] || fail "B 的未读数应 >=1，实际 '${unread:-}'"
    c b read "$conv" "$m1" >/dev/null
    # A 侧能观察到 m1 状态为 Read（MessageUpdated 事件）。
    wait_for 30 "A 观察到 m1 已读" msg_state_seen "$wa" "$m1" Read
    c a recall "$m2" >/dev/null
    # B 侧观察到 m2 被撤回（事件 + log 中不再是原文本）。
    wait_for 30 "B 观察到 m2 撤回" msg_state_seen "$wb" "$m2" Recalled
    if has_text b "$conv" "要被撤回的"; then fail "撤回后 B 的 log 仍显示原文"; fi
    wait_for 15 "A 侧 m2 为 Recalled" msg_state_seen "$wa" "$m2" Recalled
    unread="$(c b convs | awk -v c="$conv" '$1==c{print $3}')"
    echo "recall_read：Read 与 Recalled 状态均在对端可见（B 已读 m1 后未读数=$unread）"
}

# ---------------------------------------------------------------- 场景 9
sc_bench_local() {
    local out
    out="$(LANMSG_BIN_DIR="$BIN_DIR" bash "$REPO/scripts/bench-throughput.sh" local 256 2>&1)" || fail "bench 失败：$out"
    echo "$out" | tail -n 3
    echo "$out" | grep -q '^BENCH ' || fail "bench 输出里没有 BENCH 行"
}

# ====================================================================
# 子进程入口
# ====================================================================
if [ "${1:-}" = "--child" ]; then
    name="$2"
    SC="$3"
    PORT_BASE="$4"
    mkdir -p "$SC"
    trap child_cleanup EXIT
    trap 'exit 143' TERM INT
    "sc_$name"
    exit 0
fi

# ====================================================================
# 父进程：调度、超时、汇总
# ====================================================================
ONLY=()
while [ $# -gt 0 ]; do
    case "$1" in
    --only)
        ONLY+=("${2:?--only 需要场景名}")
        shift 2
        ;;
    *)
        echo "未知参数：$1" >&2
        exit 2
        ;;
    esac
done
SELECTED=("${ALL_SCENARIOS[@]}")
if [ "${#ONLY[@]}" -gt 0 ]; then SELECTED=("${ONLY[@]}"); fi
for s in "${SELECTED[@]}"; do
    declare -F "sc_$s" >/dev/null || {
        echo "未知场景：$s（可选：${ALL_SCENARIOS[*]}）" >&2
        exit 2
    }
done

e2e_check_bins

TMP="$(mktemp -d /tmp/lanmsg-acc.XXXXXX)"
cleanup() {
    # 兜底：杀掉任何仍引用本临时目录的守护进程。
    pkill -9 -f -- "--data-dir $TMP/" 2>/dev/null || true
    if [ "${KEEP:-0}" = 1 ]; then echo "保留临时目录：$TMP"; else rm -rf "$TMP"; fi
}
trap cleanup EXIT

# 端口基数随机化：固定端口曾与同机并发运行的其他实例冲突（守护进程会静默回退到
# 随机端口，导致 --peer 指向落空）。
PORT_RUN_BASE=$((15000 + (RANDOM % 100) * 100)) # 避开内核临时端口段（32768+），否则会与出站连接撞端口

RESULTS=()
FAILED=0
for s in "${SELECTED[@]}"; do
    # 端口基数按场景序号错开，避免相互干扰（每场景最多 10 个节点）。
    pos=0
    for k in "${!ALL_SCENARIOS[@]}"; do if [ "${ALL_SCENARIOS[$k]}" = "$s" ]; then pos=$k; fi; done
    base=$((PORT_RUN_BASE + pos * 10))
    out="$TMP/$s.out"
    t0=$SECONDS
    if timeout --signal=TERM --kill-after=10 "$(scenario_timeout "$s")" bash "$SELF" --child "$s" "$TMP/$s" "$base" >"$out" 2>&1; then
        RESULTS+=("PASS $s ($((SECONDS - t0))s)")
        sed 's/^/    /' "$out"
    else
        rc=$?
        FAILED=1
        RESULTS+=("FAIL $s ($((SECONDS - t0))s, rc=$rc)")
        echo "===== 场景失败：$s（退出码 $rc；124/137 表示超时）====="
        sed 's/^/    /' "$out"
        for lg in "$TMP/$s"/*.log; do
            [ -f "$lg" ] || continue
            echo "----- 日志尾部：$lg"
            tail -n 25 "$lg" | sed 's/^/    /'
        done
        # 清理该场景残留守护进程。
        pkill -9 -f -- "--data-dir $TMP/$s/" 2>/dev/null || true
    fi
done

echo
echo "================ 汇总 ================"
printf '%s\n' "${RESULTS[@]}"
if [ "$FAILED" = 1 ]; then
    echo "ACCEPTANCE FAILED"
    exit 1
fi
echo "ACCEPTANCE OK"

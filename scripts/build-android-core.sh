#!/usr/bin/env bash
# 编译 lanmsg-ffi 的 Android 三个 ABI 动态库，并生成 Kotlin 绑定。
# 用法：scripts/build-android-core.sh [debug|release]（默认 release）
set -euo pipefail

PROFILE="${1:-release}"
case "$PROFILE" in
  debug) CARGO_PROFILE_FLAG="" ;;
  release) CARGO_PROFILE_FLAG="--release" ;;
  *) echo "错误：参数必须是 debug 或 release" >&2; exit 1 ;;
esac

if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "错误：未找到 cargo-ndk，请先执行 cargo install cargo-ndk" >&2
  exit 1
fi
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  echo "错误：未设置 ANDROID_NDK_HOME，请指向 Android NDK（r26 及以上）" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# 1. 三个 ABI 的 .so
# shellcheck disable=SC2086
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
  -o android/app/src/main/jniLibs build -p lanmsg-ffi $CARGO_PROFILE_FLAG

# 2. 宿主版本，供 bindgen 读取元数据
cargo build -p lanmsg-ffi
case "$(uname -s)" in
  Darwin) EXT=dylib ;;
  *) EXT=so ;;
esac
HOST_LIB="${CARGO_TARGET_DIR:-target}/debug/liblanmsg_ffi.$EXT"

# 3. 生成 Kotlin 绑定
cargo run -p lanmsg-ffi --features bindgen --bin uniffi-bindgen -- generate \
  --library "$HOST_LIB" --language kotlin \
  --out-dir android/app/src/main/java --no-format

# 4. 修正名称遮蔽：生成代码里 sealed class Response 的嵌套类 SelfInfo 与顶层
#    DTO SelfInfo 同名，其字段类型 `SelfInfo` 会被 Kotlin 解析成嵌套类自身而编译失败，
#    因此把这一处替换为全限定名。
KT="android/app/src/main/java/uniffi/lanmsg_api/lanmsg_api.kt"
#    生成格式为两行：`data class SelfInfo(` 换行 `val \`value\`: SelfInfo) : Response() {`，
#    只有 Response 内这一处以 `: Response() {` 结尾，据此精确定位。
sed -i 's/^\( *val `value`: \)SelfInfo\() : Response() {\)$/\1uniffi.lanmsg_api.SelfInfo\2/' "$KT"
HITS="$(grep -c '^ *val `value`: uniffi\.lanmsg_api\.SelfInfo) : Response() {$' "$KT" || true)"
if [ "$HITS" != "1" ]; then
  echo "错误：$KT 中 Response.SelfInfo 的遮蔽修正未恰好命中 1 处（实际 $HITS 处），UniFFI 生成格式可能已变化，请更新本脚本" >&2
  exit 1
fi

# 5. 修正 FfiException.Api 的 message 冲突：Rust 侧字段名 `message` 与 kotlin.Exception 的
#    message 成员冲突（生成代码里既声明 val message 又 override message），Kotlin 编译失败。
#    改为让构造参数直接 override Throwable.message，并删除多余的 getter。
KT2="android/app/src/main/java/uniffi/lanmsg_ffi/lanmsg_ffi.kt"
python3 - "$KT2" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p, encoding="utf-8").read()
pat = re.compile(
    r"(\n\s*)val `message`: kotlin\.String\n(\s*\) : FfiException\(\) \{)\n\s*override val message\n\s*get\(\) = \"code=\$\{ `code` \}, message=\$\{ `message` \}\"\n"
)
s2, n = pat.subn(lambda m: m.group(1) + "override val `message`: kotlin.String\n" + m.group(2) + "\n", s)
if n != 1:
    sys.stderr.write("错误：%s 中 FfiException.Api 的 message 冲突修正未恰好命中 1 处（实际 %d 处），UniFFI 生成格式可能已变化，请更新本脚本\n" % (p, n))
    sys.exit(1)
open(p, "w", encoding="utf-8").write(s2)
PY

echo "完成：jniLibs 与 Kotlin 绑定已生成"

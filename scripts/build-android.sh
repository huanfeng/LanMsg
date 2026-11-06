#!/usr/bin/env bash
# 构建 Android APK。用法：scripts/build-android.sh [debug|release]（默认 release）
set -euo pipefail

VARIANT="${1:-release}"
case "$VARIANT" in
  debug) TASK="assembleDebug" ;;
  release) TASK="assembleRelease" ;;
  *) echo "错误：参数必须是 debug 或 release" >&2; exit 1 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
bash "$ROOT/scripts/build-android-core.sh" "$VARIANT"
(cd "$ROOT/android" && ./gradlew ":app:$TASK")
echo "APK：$ROOT/android/app/build/outputs/apk/$VARIANT/app-$VARIANT.apk"

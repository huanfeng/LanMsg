#!/bin/bash
# 输出 CHANGELOG.md 中指定版本的小节正文（不含标题行）；找不到时输出占位说明。
# 用法：scripts/changelog-section.sh <x.y.z>
set -euo pipefail
version=${1#v}
root=$(cd "$(dirname "$0")/.." && pwd)
body=$(awk -v h="## [$version]" '
    index($0, h) == 1 { on = 1; next }
    on && /^## \[/ { exit }
    on { print }
' "$root/CHANGELOG.md" | sed -e :a -e '/^[[:space:]]*$/{$d;N;ba' -e '}' | sed '/./,$!d')
if [ -n "$body" ]; then
    printf '%s\n' "$body"
else
    echo "LanMsg $version"
fi

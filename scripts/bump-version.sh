#!/bin/bash
# 升级版本号：修改根 Cargo.toml 的 [workspace.package].version、刷新 Cargo.lock，
# 并把 CHANGELOG.md 的 [Unreleased] 内容归入新版本小节。
# Android 的 versionName/versionCode 在 gradle 里从根 Cargo.toml 推导，无需另改。
# 用法：scripts/bump-version.sh <x.y.z>
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "usage: bump-version.sh <x.y.z>" >&2
    exit 2
fi
version=${1#v}
if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: version must be x.y.z: $1" >&2
    exit 2
fi

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if grep -q "^## \[$version\]" CHANGELOG.md; then
    echo "error: CHANGELOG.md already has a section for $version" >&2
    exit 1
fi
if ! grep -q '^## \[Unreleased\]' CHANGELOG.md; then
    echo "error: CHANGELOG.md has no [Unreleased] section" >&2
    exit 1
fi

# 只改 [workspace.package] 段内的第一个 version
awk -v v="$version" '
    /^\[/ { inpkg = ($0 == "[workspace.package]") }
    inpkg && !done && /^version[ \t]*=/ { print "version = \"" v "\""; done = 1; next }
    { print }
    END { if (!done) exit 3 }
' Cargo.toml > Cargo.toml.new || { rm -f Cargo.toml.new; echo "error: no version in [workspace.package]" >&2; exit 1; }
mv Cargo.toml.new Cargo.toml

today=$(date +%Y-%m-%d)
awk -v v="$version" -v d="$today" '
    /^## \[Unreleased\]/ { print; print ""; print "## [" v "] - " d; next }
    { print }
' CHANGELOG.md > CHANGELOG.md.new
mv CHANGELOG.md.new CHANGELOG.md

# 刷新 Cargo.lock 中工作区成员的版本（不升级任何依赖）
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null

echo "bumped to $version; review CHANGELOG.md, then commit"

#!/usr/bin/env python3
"""检查 Rust 非测试代码的字符串字面量不含中日韩文字。

背景：日志与错误消息一律英文，界面文案只放本地化资源（桌面 strings.rs、
lanmsgd strings.rs）。本脚本把这条规则固定下来。

规则：
- 扫描 crates/**/*.rs，跳过 tests/ 目录；
- 跳过注释（行注释、块注释、文档注释），注释允许中文；
- 跳过 `#[cfg(test)]` 后紧跟 `mod` 的整个文件尾部（测试模块）；
- 白名单文件（界面文案与演示数据）整体放行。
有命中时逐行打印 file:line 并以退出码 1 结束；无命中退出码 0。

用法：python3 scripts/check-no-cjk.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WHITELIST = {
    "crates/lanmsg-desktop/src/strings.rs",
    "crates/lanmsgd/src/strings.rs",
    "crates/lanmsg-desktop/src/fixtures.rs",
}
CJK = re.compile(r"[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uac00-\ud7af]")  # 假名、汉字、谚文；全角标点属排版符号，放行
CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]\s*$")
MOD = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod\s")


def strip_test_tail(lines):
    """`#[cfg(test)]` 后紧跟 `mod` 的位置之后全部丢弃，返回保留的行。"""
    for i, line in enumerate(lines):
        if CFG_TEST.match(line):
            j = i + 1
            while j < len(lines) and lines[j].lstrip().startswith("#["):
                j += 1
            if j < len(lines) and MOD.match(lines[j]):
                return lines[:i]
    return lines


def string_literals(text):
    """逐字符扫描，产出 (起始行号, 字面量内容)；跳过注释与字符字面量。"""
    i, n, line = 0, len(text), 1
    out = []
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            while i < n and text[i] != "\n":
                i += 1
        elif text.startswith("/*", i):
            depth = 1
            i += 2
            while i < n and depth:
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    if text[i] == "\n":
                        line += 1
                    i += 1
        elif c == "r" and re.match(r'r#*"', text[i:i + 8]) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            m = re.match(r'r(#*)"', text[i:])
            hashes = m.group(1)
            start = i + m.end()
            end = text.find('"' + hashes, start)
            end = n if end < 0 else end
            out.append((line, text[start:end]))
            line += text.count("\n", i, end)
            i = end + 1 + len(hashes)
        elif c == '"':
            start_line = line
            i += 1
            buf = []
            while i < n and text[i] != '"':
                if text[i] == "\\" and i + 1 < n:
                    buf.append(text[i:i + 2])
                    if text[i + 1] == "\n":
                        line += 1
                    i += 2
                    continue
                if text[i] == "\n":
                    line += 1
                buf.append(text[i])
                i += 1
            out.append((start_line, "".join(buf)))
            i += 1
        elif c == "'":
            # 字符字面量（'x'、'\n'、'中'）整体跳过；生命周期（'a）只吃一个引号
            m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:])
            i += m.end() if m else 1
        else:
            i += 1
    return out


def main():
    hits = []
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        rel = path.relative_to(ROOT).as_posix()
        if rel in WHITELIST or "/tests/" in rel:
            continue
        lines = path.read_text(encoding="utf-8").split("\n")
        text = "\n".join(strip_test_tail(lines))
        for lineno, content in string_literals(text):
            if CJK.search(content):
                hits.append(f"{rel}:{lineno}: {content.strip()[:60]}")
    for h in hits:
        print(h)
    if hits:
        print(f"\n{len(hits)} string literal(s) contain CJK text (see scripts/check-no-cjk.py)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

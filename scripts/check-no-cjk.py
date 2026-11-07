#!/usr/bin/env python3
"""检查 Rust 非测试代码的字符串字面量不含中日韩文字（含全角标点）。

背景：日志与错误消息一律英文，界面文案只放本地化资源（桌面 strings.rs、
lanmsgd strings.rs）。本脚本把这条规则固定下来。

规则：
- 扫描 crates/**/*.rs，跳过 tests/ 目录；
- 跳过注释（行注释、块注释、文档注释），注释允许中文；
- 跳过测试模块：`#[cfg(test)]` / `#[cfg(all(test, ...))]`（属性之间可夹其他属性）
  修饰的 `mod xxx { ... }`，按花括号配对只剔除这个模块本身，模块之后的生产代码照常检查；
- 白名单文件（界面文案与演示数据）整体放行；
- 放行的非 ASCII 只有省略号 `…`，其余全角标点同样视为界面文案，必须放进 strings.rs。
有命中时逐行打印 file:line 并以退出码 1 结束；无命中退出码 0。

用法：
    python3 scripts/check-no-cjk.py             # 检查仓库
    python3 scripts/check-no-cjk.py --self-test  # 运行内置反例
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
# CJK 标点与符号、假名、扩展 A、汉字、谚文、全角形式
CJK = re.compile(r"[　-〿぀-ヿ㐀-䶿一-鿿가-힯＀-￯]")


def scan(text):
    """逐字符扫描，返回 (masked, strings)。

    masked：注释与字面量内容换成空格（保留换行与引号），只剩代码结构，供花括号配对；
    strings：[(起始行号, 起始偏移, 结束偏移, 内容)]，偏移指向 text。
    """
    n = len(text)
    masked = list(text)
    strings = []
    i, line = 0, 1

    def blank(a, b):
        for k in range(a, b):
            if masked[k] != "\n":
                masked[k] = " "

    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            j = i
            while j < n and text[j] != "\n":
                j += 1
            blank(i, j)
            i = j
        elif text.startswith("/*", i):
            j, depth = i + 2, 1
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(i, j)
            line += text.count("\n", i, j)
            i = j
        elif c == "r" and re.match(r'r#*"', text[i:i + 12]) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            m = re.match(r'r(#*)"', text[i:])
            hashes = m.group(1)
            start = i + m.end()
            end = text.find('"' + hashes, start)
            end = n if end < 0 else end
            strings.append((line, start, end, text[start:end]))
            blank(start, end)
            line += text.count("\n", i, end)
            i = end + 1 + len(hashes)
        elif c == '"':
            start_line, start = line, i + 1
            j = i + 1
            while j < n and text[j] != '"':
                if text[j] == "\\" and j + 1 < n:
                    if text[j + 1] == "\n":
                        line += 1
                    j += 2
                    continue
                if text[j] == "\n":
                    line += 1
                j += 1
            strings.append((start_line, start, j, text[start:j]))
            blank(start, j)
            i = j + 1
        elif c == "'":
            # 字符字面量（'x'、'\n'、'中'）整体跳过；生命周期（'a）只吃一个引号
            m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:])
            if m:
                blank(i + 1, i + m.end() - 1)
                i += m.end()
            else:
                i += 1
        else:
            i += 1
    return "".join(masked), strings


ATTR = re.compile(r"#\s*\[")
CFG_TEST = re.compile(r"#\s*\[\s*cfg\s*\(\s*(.*?)\s*\)\s*\]", re.S)


def is_cfg_test(inner):
    """cfg 条件是 test，或 all(...) 中含顶层 test（不含 not）。"""
    inner = inner.strip()
    if inner == "test":
        return True
    if inner.startswith("all(") and "not(" not in inner:
        return re.search(r"(?<![\w:])test(?![\w:])", inner) is not None
    return False


def skip_balanced(s, i, open_c, close_c):
    """s[i] 是 open_c，返回配对的 close_c 之后的位置；失败返回 -1。"""
    depth = 0
    for j in range(i, len(s)):
        if s[j] == open_c:
            depth += 1
        elif s[j] == close_c:
            depth -= 1
            if depth == 0:
                return j + 1
    return -1


def test_module_ranges(masked):
    """返回测试模块的 [起, 止) 偏移列表（含其属性）。"""
    ranges = []
    pos = 0
    while True:
        m = CFG_TEST.search(masked, pos)
        if not m:
            break
        pos = m.end()
        if not is_cfg_test(m.group(1)):
            continue
        j = m.end()
        # 属性与 mod 之间可夹其他属性与空白
        while True:
            while j < len(masked) and masked[j].isspace():
                j += 1
            am = ATTR.match(masked, j)
            if not am:
                break
            end = skip_balanced(masked, am.end() - 1, "[", "]")
            if end < 0:
                break
            j = end
        mm = re.compile(r"(pub(\([^)]*\))?\s+)?mod\s+\w+\s*([{;])").match(masked, j)
        if not mm or mm.group(3) != "{":
            continue
        brace = mm.end() - 1
        end = skip_balanced(masked, brace, "{", "}")
        if end < 0:
            continue
        ranges.append((m.start(), end))
        pos = end
    return ranges


def cjk_hits(text):
    """返回 [(行号, 内容)]：测试模块之外含 CJK 的字符串字面量。"""
    masked, strings = scan(text)
    ranges = test_module_ranges(masked)
    hits = []
    for line, a, b, content in strings:
        if any(r0 <= a < r1 for r0, r1 in ranges):
            continue
        if CJK.search(content):
            hits.append((line, content))
    return hits


def self_test():
    cases = [
        # (名称, 源码, 期望命中行号列表)
        ("普通字面量命中", 'fn f() { log("中文"); }\n', [1]),
        ("注释中的中文放行", '// 中文\n/* 中文 */\n/// 中文\nfn f() {}\n', []),
        ("测试模块内放行", '#[cfg(test)]\nmod tests {\n    fn t() { let _ = "中文"; }\n}\n', []),
        ("测试模块之后的生产代码必须检查", '#[cfg(test)]\nmod tests {\n    fn t() { let _ = "中文"; }\n}\nfn prod() { log("中文"); }\n', [5]),
        ("同一行的测试模块", '#[cfg(test)] mod tests { fn t() { let _ = "中文"; } }\nfn prod() { log("中文"); }\n', [2]),
        ("cfg(all(test, ...))", '#[cfg(all(test, unix))]\nmod tests {\n    fn t() { let _ = "中文"; }\n}\nfn prod() { log("中文"); }\n', [5]),
        ("属性之间夹其他属性", '#[cfg(test)]\n#[allow(dead_code)]\nmod tests {\n    fn t() { let _ = "中文"; }\n}\nfn prod() { log("中文"); }\n', [6]),
        ("测试模块内字符串里的花括号不影响配对", '#[cfg(test)]\nmod tests {\n    fn t() { let _ = "}}}"; let _ = "中文"; }\n}\nfn prod() { log("中文"); }\n', [5]),
        ("cfg(not(test)) 不是测试模块", '#[cfg(not(test))]\nmod real {\n    pub fn f() { log("中文"); }\n}\n', [3]),
        ("原始字符串命中", 'fn f() { let _ = r#"中文"#; }\n', [1]),
        ("跨行字符串按起始行报", 'fn f() {\n    let _ = "第一行\n第二行";\n}\n', [2]),
        ("全角标点命中", 'fn f() { let _ = "（x）"; }\n', [1]),
        ("省略号放行", 'fn f() { let _ = "wait…"; }\n', []),
        ("字符字面量与生命周期", "fn f<'a>(x: &'a str) { let _ = '中'; let _ = \"ok\"; }\n", []),
        ("外部测试模块声明不吞后文", '#[cfg(test)]\nmod tests;\nfn prod() { log("中文"); }\n', [3]),
    ]
    failed = 0
    for name, src, expect in cases:
        got = [ln for ln, _ in cjk_hits(src)]
        if got != expect:
            failed += 1
            print(f"FAIL {name}: expect {expect}, got {got}")
    if failed:
        print(f"{failed} self-test case(s) failed", file=sys.stderr)
        return 1
    print(f"self-test ok ({len(cases)} cases)")
    return 0


def main():
    if "--self-test" in sys.argv[1:]:
        return self_test()
    hits = []
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        rel = path.relative_to(ROOT).as_posix()
        if rel in WHITELIST or "/tests/" in rel:
            continue
        for lineno, content in cjk_hits(path.read_text(encoding="utf-8")):
            hits.append(f"{rel}:{lineno}: {content.strip()[:60]}")
    for h in hits:
        print(h)
    if hits:
        print(f"\n{len(hits)} string literal(s) contain CJK text (see scripts/check-no-cjk.py)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

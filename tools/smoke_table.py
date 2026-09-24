#!/usr/bin/env python3
"""Run the smoke suite and render the execution results as a table.

Usage:  python tools/smoke_table.py   (from the repo root)

Table columns: 测试名 | 测试点 | 结果 | 收益/成本要点
Test names + descriptions are parsed statically from cli/tests/smoke.rs (reliable);
pass/fail comes from the actual cargo test run.
"""
import re
import subprocess
import sys
from pathlib import Path

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

ROOT = Path(__file__).resolve().parent.parent
SMOKE = ROOT / "cli" / "tests" / "smoke.rs"

# ---- static: test name + description from source ---------------------------
tests = []  # (name, desc)
src = SMOKE.read_text(encoding="utf-8")
for m in re.finditer(r"#\[test\]\nfn (\w+)\(\) \{\n(.*?)\n\}", src, re.S):
    name = m.group(1)
    body = m.group(2)
    dm = re.search(r'eprintln!\("== ([^"]+)', body)
    desc = dm.group(1) if dm else ""
    tests.append((name, desc))

# ---- dynamic: actual results (serialized so each test's output stays adjacent) ---
print(f"running: cargo test --test smoke --test-threads=1 -- --nocapture")
res = subprocess.run(
    ["cargo", "test", "--test", "smoke", "--", "--test-threads=1", "--nocapture"],
    cwd=ROOT, capture_output=True, text=True,
    encoding="utf-8", errors="replace",
)
combined = res.stdout + res.stderr
status = dict(re.findall(r"^test (\w+) \.\.\. (ok|FAILED)", combined, re.M))

# Serial output: each `== <desc>` starts a block; every `[量化]` line after it
# belongs to that test until the next `==` or `running N tests`.
blocks = {}  # name -> [quant lines]
order = []
cur = None
for line in combined.splitlines():
    m = re.match(r"== (.*)", line)
    if m:
        # match desc back to a test name
        cur = next((n for n, d in tests if d and d in m.group(1)), None)
        if cur and cur not in blocks:
            blocks[cur] = []
            order.append(cur)
        elif cur:
            pass
        continue
    q = re.match(r"\s*\[量化\] (.*)", line)
    if q and cur:
        blocks[cur].append(q.group(1))

def _width(s):
    """Display width: CJK chars count as 2, ASCII as 1."""
    return sum(2 if ord(c) > 127 else 1 for c in s)

def _pad(s, w):
    return s + " " * max(w - _width(s), 0)

def _trunc(s, w):
    if _width(s) <= w:
        return s
    out, cur = "", 0
    for c in s:
        cw = 2 if ord(c) > 127 else 1
        if cur + cw > w - 1:
            break
        out += c
        cur += cw
    return out + "…"

def _table(headers, rows, caps):
    ncol = len(headers)
    widths = []
    for i in range(ncol):
        w = _width(headers[i])
        for r in rows:
            w = max(w, _width(r[i]))
        widths.append(min(w, caps[i]))
    line = "+" + "+".join("-" * (x + 2) for x in widths) + "+"

    def fmt(r):
        return "| " + " | ".join(_pad(_trunc(r[i], widths[i]), widths[i]) for i in range(ncol)) + " |"

    return "\n".join([line, fmt(headers), line] + [fmt(r) for r in rows] + [line])

total = passed = 0
rows = []
for name, desc in tests:
    r = status.get(name)
    total += 1
    mark = "通过" if r == "ok" else ("失败" if r == "FAILED" else "未运行")
    q = "；".join(blocks.get(name, [])[:3]) or "—"
    rows.append([name, desc or "—", mark, q])
    if r == "ok":
        passed += 1

for name, r in status.items():
    if r and name not in [t[0] for t in tests]:
        total += 1
        rows.append([name, "(源码未匹配)", "失败" if r == "FAILED" else "通过", "—"])

print()
print(_table(["测试名", "测试点", "结果", "收益 / 成本（实测）"], rows, caps=[26, 46, 8, 70]))
print()
print(f"汇总: {passed}/{total} 通过" + ("" if passed == total else f"  ❌ {total-passed} 失败"))
sys.exit(0 if passed == total else 1)

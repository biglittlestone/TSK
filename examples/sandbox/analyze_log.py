#!/usr/bin/env python3
"""TSK sandbox example: summarize a web access log to 6 lines of conclusions.

Usage:  tsk exec examples/sandbox/analyze_log.py <access.log>

Only stdout (the conclusions) enters the agent's context; the raw lines never do.
Run the sandbox yourself — TSK does not auto-generate analysis scripts.
"""
import collections
import re
import sys


def main(path: str) -> None:
    total = 0
    codes = collections.Counter()
    sent = 0
    times = []
    paths = collections.Counter()
    slow = 0
    pat = re.compile(r'"(\S+ \S+ [^"]*)" (\d{3}) (\d+) "[^"]*" "[^"]*" ([\d.]+)')
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            m = pat.search(line)
            total += 1
            if not m:
                continue
            codes[m.group(2)] += 1
            sent += int(m.group(3))
            times.append(float(m.group(4)))
            paths[m.group(1).split()[1]] += 1
            if float(m.group(4)) > 1.0:
                slow += 1
    print(f"Total requests: {total}")
    print(f"Status codes: {dict(sorted(codes.items()))}")
    print(f"Top paths: {paths.most_common(3)}")
    print(f"Total bytes: {sent}")
    print(f"Avg response: {sum(times) / len(times) if times else 0:.3f}s")
    print(f"Slow requests (>1s): {slow}")


if __name__ == "__main__":
    main(sys.argv[1])

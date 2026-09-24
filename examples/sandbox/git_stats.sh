#!/bin/sh
# TSK sandbox example: git log summary. Only these 4 lines of conclusions enter
# the agent's context — the full history never does.
# Usage: tsk exec examples/sandbox/git_stats.sh <git-repo-dir>
echo "commits: $(git -C "$1" rev-list --count HEAD 2>/dev/null || echo 0)"
echo "authors: $(git -C "$1" shortlog -sn 2>/dev/null | wc -l)"
echo "churn: $(git -C "$1" log --oneline 2>/dev/null | wc -l) commits in history"
echo "merges: $(git -C "$1" log --merges --oneline 2>/dev/null | wc -l)"

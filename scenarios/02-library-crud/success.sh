#!/usr/bin/env bash
cd "$(dirname "$0")"
[ -f solution/index.html ] || [ -f solution/app.js ] || { echo "FAIL"; exit 1; }
echo "OK: solution 有页面"

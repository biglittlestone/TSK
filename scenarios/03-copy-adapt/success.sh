#!/usr/bin/env bash
cd "$(dirname "$0")"
[ -f solution/index.html ] && [ -f solution/app.js ] && echo "OK" || { echo "FAIL"; exit 1; }

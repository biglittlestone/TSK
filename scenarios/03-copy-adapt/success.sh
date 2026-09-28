#!/usr/bin/env bash
cd "$(dirname "$0")"
[ -f solution/index.html ] && [ -f solution/app.js ] && [ -f solution/style.css ] && echo "OK" || { echo "FAIL"; exit 1; }
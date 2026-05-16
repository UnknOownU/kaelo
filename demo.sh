#!/bin/bash
# Kaelo Demo Script for asciinema
set -e

KAELO="${KAELO:-${HOME}/.local/bin/kaelo}"

echo "🚀 Kaelo - Intelligent Web Fetching for AI Agents"
echo ""
echo "$ kaelo prove-it"
sleep 0.5
"$KAELO" prove-it
echo ""
echo "$ kaelo fetch https://httpbin.org/get"
sleep 0.5
"$KAELO" fetch https://httpbin.org/get
echo ""
echo "$ kaelo cache status"
sleep 0.5
"$KAELO" cache status
echo ""
echo "✅ Demo complete!"

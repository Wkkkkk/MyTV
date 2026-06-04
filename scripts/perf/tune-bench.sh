#!/usr/bin/env bash
# Measure tune-endpoint latency distribution with hyperfine.
# Usage: ./tune-bench.sh [channel_id] [base_url]
set -euo pipefail
command -v hyperfine >/dev/null || { echo "hyperfine not found: brew install hyperfine"; exit 1; }
CHANNEL_ID="${1:-1}"
BASE_URL="${2:-http://localhost:3000}"

hyperfine --warmup 3 --runs 20 "curl -sf '${BASE_URL}/channel/${CHANNEL_ID}/tune'"

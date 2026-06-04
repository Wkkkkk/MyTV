#!/usr/bin/env bash
# Load-test the guide routes against a running server.
# Usage: ./load-guide.sh [base_url]   (default http://localhost:3000)
set -euo pipefail
command -v oha >/dev/null || { echo "oha not found: brew install oha"; exit 1; }
BASE_URL="${1:-http://localhost:3000}"

echo "== GET /guide (30s, 5 conns) =="
oha -z 30s -c 5 --no-tui "$BASE_URL/guide"

echo "== GET /guide/partial (30s, 5 conns) =="
oha -z 30s -c 5 --no-tui "$BASE_URL/guide/partial"

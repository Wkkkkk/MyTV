#!/bin/sh
set -e

echo "pre-push: cargo fmt --check"
cargo fmt --check

echo "pre-push: cargo clippy"
cargo clippy -- -D warnings

echo "pre-push: cargo test"
cargo test

echo "pre-push: all checks passed"

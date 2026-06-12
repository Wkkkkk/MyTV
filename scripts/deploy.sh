#!/usr/bin/env sh
# Deploy to Fly.io, then run the e2e smoke suite against the live instance.
# MYTV_ADMIN_PASSWORD must be set in the caller's environment.
set -e

fly deploy --app kunstv

: "${MYTV_ADMIN_PASSWORD:?set MYTV_ADMIN_PASSWORD to run the post-deploy e2e suite}"
MYTV_BASE_URL=https://kunstv.fly.dev \
  cargo test --test e2e -- --ignored --nocapture

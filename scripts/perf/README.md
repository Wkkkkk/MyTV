# Performance harness

Companion tooling for `docs/performance/FRAMEWORK.md`. All load tests run against a **release** build with seeded data.

## Setup

```bash
brew install oha hyperfine        # load generator + CLI benchmarker

# Seeded local server on :3000:
DATABASE_URL=sqlite:perf.db cargo run --release   # creates db + runs migrations, then Ctrl-C
sqlite3 perf.db < tests/fixtures/seed.sql
DATABASE_URL=sqlite:perf.db cargo run --release
```

Note: seed channel 1 = live OK, 3 = has fallback, 4 = VOD with items (see CLAUDE.md). Seed source URLs point at unreachable test hosts, so tune latency against seed data measures the app + timeout path, not a real upstream. For realistic numbers, add a channel with a real stream via /admin.

## Recipes

| What | How |
|---|---|
| Guide latency under load | `./load-guide.sh` |
| Tune latency distribution | `./tune-bench.sh 1` (live), `./tune-bench.sh 4` (VOD) |
| Micro-benches | `cargo bench` (criterion reports in `target/criterion/`) |
| Live RSS + route histograms | `curl -u user:$ADMIN_PASSWORD https://kunstv.fly.dev/admin/metrics` |
| Local route histograms | `curl -u user:admin http://localhost:3000/admin/metrics` |
| Cold start | `fly machine stop <id> --app kunstv`, then `curl -w '@-' -o /dev/null -s https://kunstv.fly.dev/health <<< 'total=%{time_total}\n'` |
| Proxy manifest overhead | time the same manifest URL direct vs through `/stream-proxy?url=<pct-encoded>` — run each a few times, compare medians. Keep rates low; these are third-party upstreams. |
| CPU flamegraph | `cargo install flamegraph`, then `cargo flamegraph --root --release` while driving load with oha (`--root` runs dtrace via sudo on macOS) |
| Heap deep-dive (macOS) | Instruments → Allocations against `target/release/mytv` |
| yt-dlp cost in isolation | `time yt-dlp -g '<youtube-url>'`; in a separate terminal, watch `ps -o rss= -p <pid>` |
| Fly-side view | `fly machine status --app kunstv`, Fly dashboard → Metrics (RSS, CPU steal) |

## Why no offline proxy load test?

The SSRF guard blocks loopback/private upstreams in `stream_proxy` by design, so a local mock origin can't be proxied without weakening production security. Proxy compute (manifest rewrite) is covered by `cargo bench`; end-to-end proxy overhead is measured against real upstreams at low rate.

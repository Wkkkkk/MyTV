# E2E Suite — Design (Spec 4 of 4)

**Date:** 2026-06-12
**Status:** Approved design, pending implementation plan
**Part of:** "Agent + E2E testing capability" effort

## Context

Fourth and final spec of the effort that made MyTV scriptable and end-to-end testable:

1. **Spec 1 — Player observability** ✅ (merged): `source_id`/`source_url`/`playlist_item_id` on `TuneResponse`; public `GET /watch/:id`.
2. **Spec 2 — Admin automation** ✅ (merged): JSON `/api/admin` CRUD + `mytvctl` CLI.
3. **Spec 3 — Discovery API + CLI** ✅ (merged): `/api/admin/discover/**` + `mytvctl discover`.
4. **Spec 4 — E2E suite (this doc):** an end-to-end test suite that drives the **live prod instance** (`https://kunstv.fly.dev`), doing real, self-cleaning CRUD through both the JSON API and the `mytvctl` binary.

### Problem

Specs 1–3 built the JSON API + CLI surface but verified it only with in-process `oneshot` integration tests against an in-memory SQLite DB. Nothing exercises the **deployed** app over the network, and the `mytvctl` binary's real HTTP path (send + auth + exit codes) is entirely unexercised — Spec 2/3 unit-tested only the pure `request_for` arg→request mapping. There is no smoke test to confirm a fresh deploy actually works.

### Goals

- A network-driven E2E suite (`tests/e2e.rs`) that smoke-tests the deployed prod app via real CRUD.
- Self-cleaning, uniquely-tagged test data so a crashed run never orphans real channels.
- Exercise the `mytvctl` binary's real HTTP path (exit codes, auth, stdout JSON).
- Use Spec 1's `source_id` to assert *which* source actually tuned.
- Run manually on demand, and automatically after a deploy via a wrapper script.
- Stay invisible to CI and a plain `cargo test` (contributors without prod creds keep a green build).

### Non-goals

- **No browser / UI E2E** (Playwright et al.). The risky logic lives in the backend/resolver; the UI is thin HTMX and live external-stream playback can't be reliably asserted. Explicitly out of scope.
- No new CD pipeline. Deploys stay a manual `fly deploy --app kunstv`, wrapped by a script.
- No changes to app behavior — this spec only adds tests + a deploy script + docs.

## Single source of truth: live prod

The user has explicitly chosen the deployed instance (`kunstv.fly.dev`) as the single source of truth. Real, destructive CRUD against prod is **in scope and approved**, made safe by unique tagging + a prefix sweep (below).

## Architecture

### Gating & invocation

Each test is annotated `#[ignore = "e2e against prod — run manually"]` **and** early-returns (with a printed skip message) if `MYTV_BASE_URL` / `MYTV_ADMIN_PASSWORD` are unset. Double safety:

- CI runs `cargo test` without `--ignored` → e2e never executes.
- CI has no prod creds → even if `--ignored` were passed, the suite would skip cleanly.

Config model (from Spec 2): `MYTV_BASE_URL` + `MYTV_ADMIN_PASSWORD` (password env-only; HTTP Basic; the username half is ignored by the server). Mirrors the repo's existing `#[ignore = "requires network access — run manually"]` convention (8 such tests today).

Manual run:

```bash
MYTV_BASE_URL=https://kunstv.fly.dev MYTV_ADMIN_PASSWORD=… \
  cargo test --test e2e -- --ignored --nocapture
```

### Structure — single serial orchestrator

A single `#[test] #[ignore] fn e2e_smoke()`:

1. **Start sweep** — list channels via the API; delete every channel whose name begins with the test prefix (cleans up any prior crashed run).
2. Run each scenario in sequence, each creating entities tagged `__e2e__<token>__<scenario>` and cleaning up its own via an RAII guard (`Drop` → `DELETE`).
3. **End sweep** — repeat the prefix sweep inside a guard so it runs even if a scenario panics, ensuring nothing tagged survives.

Rationale: cargo runs `#[test]` functions in parallel by default. A *global* prefix sweep would race with another test's in-flight tagged channel. One serial orchestrator sidesteps the hazard entirely and yields a single, clear pass/fail for the post-deploy check. The per-run `token` is derived from the process id (a real test binary, so this is fine — unlike workflow scripts there is no determinism constraint).

### Two-tier failure policy

| Tier | Scenarios | On failure |
|------|-----------|-----------|
| **Authoritative** | 1 (CRUD arc), 2 (Tune `source_id`) | **Hard-fail** the test |
| **Advisory** | 3 (mytvctl path), 4 (Discovery) | **Warn, don't fail** — print `⚠ WARN …`, continue |

Scenarios 1 & 2 use fake URLs and no external dependencies — fully deterministic; they *are* the deploy smoke signal. Scenarios 3 & 4 cross process/network boundaries and touch external resources (iptv-org, YouTube, yt-dlp, live/VOD streams that may legitimately be down or removed), so a failure there is advisory, not a deploy regression.

**Implementation of warn-not-fail:** scenario helpers return `Result<(), String>` (non-panicking). The orchestrator `?`-propagates scenarios 1 & 2 (an `Err`/panic fails the test) and, for 3 & 4, logs a `⚠ WARN` line on `Err` and increments a warning counter printed in the end-of-run summary. No `catch_unwind` needed — reqwest calls and subprocess exit-status checks return `Result` naturally.

### Scenarios

**1. Channel CRUD arc** (authoritative)
`POST` create → `GET` → `PATCH` → `DELETE` via `reqwest`, deserializing responses into the lib's own DTO/model types for a compile-checked contract. Asserts status codes and that the round-tripped fields match.

**2. Tune asserts `source_id`** (authoritative)
Create channel → attach a source with the harmless fake URL `https://example.invalid/__e2e__.m3u8` → `GET /channel/:id/tune` → assert the returned `source_id` and `source_url` match the source just created → delete. Relies on the fact that `tune` does **not** probe the stream (it returns the active source's URL regardless of reachability — confirmed by seed channel 1's unreachable URL still tuning), so no live stream is required and there is no flakiness.

**3. mytvctl CLI real path** (advisory)
Drive the compiled binary via `CARGO_BIN_EXE_mytvctl` (cargo provides this env var to integration tests) against prod:

- **exit 0** — `channel create` → `channel get` → `channel delete` round-trip; parse stdout JSON on the success path.
- **exit 1** — `channel get 999999999` → server 404 → exit 1 (clean, no side effects).
- **exit 2** — invoke with `MYTV_ADMIN_PASSWORD` unset → the binary exits 2 before any request.

Verified exit-code mapping (`src/bin/mytvctl.rs`): `0` = HTTP 2xx; `1` = any non-2xx (incl. **401 wrong-password**, 404, 422, 500) or a network/send error; `2` = missing `MYTV_ADMIN_PASSWORD` or a clap usage error. (Note: a *wrong-password* run reaches the server and exits **1**, not 2 — exit 2 is reserved for the env var being absent.) The response body always prints to stdout when non-empty, even on error statuses.

**4. Discovery** (advisory)
Best-effort, warn-not-fail throughout — gone/changed streams must never fail the run:

| Check | Call | Expectation |
|-------|------|-------------|
| YouTube search w/o key | `discover youtube --keyword test` | HTTP 503 (`YOUTUBE_API_KEY` unset in prod) — deterministic |
| Channel URL parse | `discover channel --url https://www.youtube.com/@NASA` | 200 + candidate JSON (pure URL parse, no network/yt-dlp) |
| m3u search | `discover m3u --country se --group News` | 200 + JSON array (possibly empty); shape only |
| Resolve (representative URLs) | `discover resolve --url <each below>` | best-effort: 200 + candidate JSON, else warn |

Representative resolve URLs (provided by the user — real streams across protocols/types):

| Kind | URL |
|------|-----|
| DASH live | `https://demo.unified-streaming.com/k8s/live/scte35.isml/.mpd` |
| DASH VOD | `https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd` |
| HLS live | `https://stream.mux.com/v69RSHhFelSm4701snP22dYz2jICy4E4FUyk02rW4gxRM.m3u8` |
| HLS VOD | `https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8` |
| YouTube video | `https://www.youtube.com/watch?v=dQw4w9WgXcQ` |

### Harness module

A small `mod harness` (inside `tests/e2e.rs`, or a `tests/e2e/` dir if it grows):

- `Config { base_url, password }` + `env_or_skip() -> Option<Config>` (returns `None` + prints skip when env absent).
- `ApiClient` — wraps `reqwest::Client` with the base URL + HTTP Basic auth; typed helpers (`get`/`post`/`patch`/`delete` returning status + deserialized body).
- `TestChannel` — RAII guard holding a channel id; `Drop` issues a `DELETE` (best-effort).
- `sweep(prefix)` — list channels, delete any whose name starts with `prefix`.
- A warning accumulator (count + messages) for the advisory tier, printed in the summary.

The test prefix is a single constant (`__e2e__`); the per-run token disambiguates concurrent humans/agents but the sweep matches the prefix alone.

### Post-deploy wiring

`scripts/deploy.sh`:

```sh
#!/usr/bin/env sh
set -e
fly deploy --app kunstv
MYTV_BASE_URL=https://kunstv.fly.dev cargo test --test e2e -- --ignored --nocapture
```

`MYTV_ADMIN_PASSWORD` is supplied by the caller's environment (not hard-coded). Deploying via the script gives the "automatically after a major deploy" trigger; running the `cargo test` line by itself is the manual trigger. Documented in `CLAUDE.md` (key commands + a short E2E note).

## Testing

This spec *is* tests, so "testing the tests" is bounded:

- Any **pure helper** added (e.g. prefix-matching for the sweep) gets a normal unit test that runs in CI.
- The orchestrator itself is validated by a real manual run against prod (`--ignored`), confirming: it skips cleanly with no env, the CRUD arc + tune assertions pass, the advisory tier warns rather than fails on an intentionally-bad URL, and start/end sweeps leave zero `__e2e__` channels behind.
- CI behavior is verified by confirming a plain `cargo test` neither runs nor compiles-away the e2e tests (they compile but are ignored).

## Conventions / gotchas carried in

- `cargo fmt` + `cargo clippy --all-targets -- -D warnings` must pass before every commit (CI fails on any diff/warning). Rust pinned 1.96.
- `mytvctl` is a standalone binary (doesn't link the `mytv` lib); clap pinned 4.5.x.
- Editor/rust-analyzer diagnostics are routinely stale — ground-truth with a real `cargo test` / `cargo clippy` run.

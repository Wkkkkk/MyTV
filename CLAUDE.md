# MyTV — Claude Context

Personal IPTV web app. Rust/Axum backend, SQLite database, Askama templates, HTMX frontend. Single-user, no transcoding, no HTTPS (put a reverse proxy in front).

Live instance: https://kunstv.fly.dev/

---

## Tech stack

- **Rust** (toolchain pinned to `1.96` via `rust-toolchain.toml`)
- **Axum 0.7** — HTTP framework
- **SQLx 0.7** — async SQLite, compile-time checked queries
- **Askama 0.12** — type-safe HTML templates in `templates/`
- **HTMX** — partial page updates (EPG guide, admin forms)
- **reqwest** — HTTP client for health checks and stream proxy

---

## Key commands

```bash
cargo build            # compile
cargo test             # 399 tests: 294 unit + 105 integration (9 ignored — need yt-dlp/network)
cargo fmt              # format (ALWAYS run before committing)
cargo clippy           # lint (CI runs with -D warnings)
cargo run              # start server on :3000
fly deploy --app kunstv  # deploy to Fly.io
./scripts/deploy.sh      # deploy to Fly.io, then run e2e smoke suite against prod
```

---

## Project structure

```
src/
  lib.rs          # library root: exports AppState, build_router, public modules
  main.rs         # thin startup wrapper — only tokio::main, env setup, serve
  bin/
    mytvctl.rs    # CLI client for the JSON API (clap; MYTV_BASE_URL + MYTV_ADMIN_PASSWORD)
  config.rs       # Config struct, reads env vars
  db.rs           # db::connect(), runs migrations
  health.rs       # background health checker (spawned in main)
  epg.rs          # EPG time-window calculations
  budget.rs       # CORS budget badge computation (⚡/☁) for guide display
  ssrf.rs         # SSRF URL validation and 60 s hostname cache
  metrics.rs      # latency histograms, proxy counters, track_metrics middleware
  proxy.rs        # stream-proxy deep module: fetch_rewritten (redirect-follow, SSRF, detect, HLS/DASH rewrite)
  model/          # Channel, Source, PlaylistItem structs + DB queries
  routes/
    player.rs     # /channel/:id/tune, /channel/:id/next, /stream-proxy
    guide/        # /guide, /guide/partial — layout, badges, data aggregation
    health.rs     # /health
    admin/        # /admin/** — channel/source/playlist CRUD, discovery, live-status badge, /admin/metrics
    api/          # /api/admin/** — JSON CRUD (channels/sources/playlist incl. edit + test) + discover (m3u/youtube/resolve/channel/add), ApiError, mytvctl backend
  media/          # yt-dlp resolution + live-status probe (capped concurrency), HLS helpers, M3U parsing
benches/
  hot_paths.rs    # criterion benches (epg, hls rewrite, m3u parse, budget)
scripts/perf/     # load-test scripts + profiling recipes
migrations/       # 001_initial.sql, 002_source_health.sql, 003_indexes.sql
templates/        # Askama .html files mirroring routes structure
docs/performance/ # FRAMEWORK.md — perf mind map + baseline table
tests/
  http.rs         # integration tests (tower::ServiceExt::oneshot)
  api.rs          # /api/admin JSON API integration tests (oneshot)
  fixtures/
    seed.sql      # 5 test channels, 4 sources, 2 playlist items
```

---

## Architecture notes

**Library crate**: `src/lib.rs` is the library target; `src/main.rs` is the binary. Integration tests import `mytv::{build_router, AppState, ...}` from the library. Adding new public items requires making them `pub` in `lib.rs`.

**Router layers**: `redirect_trailing_slash` is registered with `.layer()` (outermost) so it fires before route matching and before auth middleware. Auth is registered with `.route_layer()` on the admin sub-router. Consequence: `GET /admin/` returns 308 (redirect) before auth ever runs — use `/admin` (no trailing slash) to test auth.

**Player routes return JSON**: `/channel/:id/tune` and `/channel/:id/next` return `Json<TuneResponse>` with HTTP 200 on success or HTTP 503 on failure. They do not redirect. `TuneResponse` carries two booleans beyond the metadata: `skip_proxy` (player uses the unproxied resolved URL for `<video src>` — set for resolved YouTube VOD direct MP4s) and `ended` (the live broadcast finished; client auto-advances).

**Ended live → VOD**: `next_live` calls `resolver::resolve_url_with_status` (one yt-dlp subprocess that prints `live_status` + URL). When the returned status is `WasLive` or `PostLive`, or the resolved URL contains `force_finished/1` (fallback for extractors that don't set `live_status`), the handler returns `{ ended: true, url: "" }` and fires a detached `tokio::spawn` (`broadcast::spawn_conversion`) that runs `broadcast::convert_if_ended`: the awaitable conversion core in `src/broadcast.rs` takes the watch-url/duration resolution as an injected closure (`resolve_recording` in production), then flips the channel to `vod_loop`, appends the recording as a `playlist_item`, and deactivates the sources. Resolve runs before the atomic flip, which is the idempotency gate. The injected-closure seam makes the whole conversion testable without yt-dlp. Idempotent; no migration (recording URL lives on the playlist_item). See `docs/architecture/tune-flow.md`.

**VOD position**: `tune_vod_at` computes current playlist position using `Utc::now()` and the channel's `loop_anchor`. The returned URL depends on current time — tests assert `url.contains(...)` rather than exact equality.

**Live-status badges**: `GET /admin/live-status?url=...` returns a small badge partial; source rows and discovery results lazy-load it via HTMX (`hx-trigger="load"`), so admin pages never block on yt-dlp. Results are cached in `AppState.live_cache` (60s TTL; 10s for Unknown). All yt-dlp subprocesses — probes and resolvers alike — go through `resolver::run_under_cap`, a global 2-permit semaphore with a bounded wait that load-sheds instead of queueing indefinitely (each yt-dlp process holds ~73 MB; the production VM has 256 MB — see `docs/bug-logs/2026-06-10-live-status-badge-ytdlp-oom.md`).

**JSON admin API + CLI**: `src/routes/api/` serves `/api/admin/**` — JSON CRUD for channels, sources, and playlist items (list/get/create/`PATCH`/delete, plus `toggle` and `test` for sources & items). It is nested behind the *same* `basic_auth` route-layer as the form admin, reuses the `model::*` layer, and serializes the model structs directly (requests use string-friendly DTOs in each submodule; `ChannelRequest`/source/playlist `PATCH` is full-replace). Errors funnel through one `ApiError` enum → `{"error": "..."}` at 404/422/500 (`internal()` logs the real error, returns a generic 500 — no detail leaks). The `mytvctl` binary (`src/bin/mytvctl.rs`) is a standalone clap client: `mytvctl <channel|source|playlist> <verb> ...`, configured by `MYTV_BASE_URL` + `MYTV_ADMIN_PASSWORD` (password env-only), always prints the raw JSON response, exit codes 0/1/2. Its `request_for` (args→method/path/body) is a pure, unit-tested fn. The `/api/admin/discover/**` endpoints (m3u/youtube search, resolve, channel, add — the last wrapping `do_discover_add`) expose the discovery subsystem as JSON, and `mytvctl discover <m3u|youtube|resolve|channel|add>` drives them; YouTube search returns 503 when `YOUTUBE_API_KEY` is unset. See `docs/superpowers/specs/2026-06-12-admin-automation-design.md`.

**Health checker**: `health::start(pool, client)` spawns a detached Tokio task. It uses `MissedTickBehavior::Skip` on a 15-minute interval. Sources are auto-disabled after consecutive failures and re-enabled after a cooldown.

---

## Testing

Integration tests use `tower::ServiceExt::oneshot` — no TCP socket, no port binding. Each test calls `app()` which builds a fresh router with an in-memory SQLite DB seeded from `tests/fixtures/seed.sql`.

Seed channels:
| ID | Name | Type | Scenario |
|----|------|------|----------|
| 1 | Live OK | live | active source → tune returns live.m3u8 |
| 2 | All Down | live | inactive source → tune returns 503 |
| 3 | Has Fallback | live | primary inactive, backup active → next returns backup.m3u8; next with backup as failed_url → 503 |
| 4 | VOD Has Items | vod_loop | two episodes → tune returns 200 |
| 5 | VOD Empty | vod_loop | no items → tune returns 503 |

---

## CI (GitHub Actions)

Runs on push/PR to `main`: `cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo test` → `docker build`.

**Critical**: `cargo fmt` must be run before every commit. CI will fail on any formatting diff. The toolchain is pinned in `rust-toolchain.toml` to keep local and CI rustfmt versions in sync.

---

## Deployment

Hosted on Fly.io. Config in `fly.toml`. Deploy with `fly deploy --app kunstv`. Health check at `/health`.

**E2E smoke suite**: `tests/e2e.rs` drives the live prod instance with self-cleaning, `__e2e__`-prefixed CRUD. It is `#[ignore]`d and skips unless `MYTV_BASE_URL` + `MYTV_ADMIN_PASSWORD` are set (loaded from `.env` if present — real env vars take precedence), so CI and a plain `cargo test` never touch prod. Run manually with `cargo test --test e2e -- --ignored`, or automatically via `./scripts/deploy.sh`. Scenarios 1–2 (channel CRUD, tune `source_id`) hard-fail; scenarios 3–4 (`mytvctl` exit codes, discovery) warn-not-fail.

---

## Conventions

- No comments unless the WHY is non-obvious
- Askama templates use `{% match %}` / `{% when %}` for `Option` types (not `{% if let %}`)
- Admin password is set via `ADMIN_PASSWORD` env var; default in dev is `admin`
- YouTube API key is optional (`YOUTUBE_API_KEY`) — discovery YouTube search is disabled if unset

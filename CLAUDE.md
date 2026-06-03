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
cargo test             # 117 tests: 102 unit + 15 integration
cargo fmt              # format (ALWAYS run before committing)
cargo clippy           # lint (CI runs with -D warnings)
cargo run              # start server on :3000
fly deploy --app kunstv  # deploy to Fly.io
```

---

## Project structure

```
src/
  lib.rs          # library root: exports AppState, build_router, public modules
  main.rs         # thin startup wrapper — only tokio::main, env setup, serve
  config.rs       # Config struct, reads env vars
  db.rs           # db::connect(), runs migrations
  health.rs       # background health checker (spawned in main)
  epg.rs          # EPG time-window calculations
  budget.rs       # CORS budget badge computation (⚡/☁) for guide display
  ssrf.rs         # SSRF URL validation and 60 s hostname cache
  model/          # Channel, Source, PlaylistItem structs + DB queries
  routes/
    player.rs     # /channel/:id/tune and /channel/:id/next
    guide.rs      # /guide, /guide/partial
    health.rs     # /health
    admin/        # /admin/** — channel/source/playlist CRUD, discovery
  media/          # yt-dlp resolution, HLS stream proxy
migrations/       # 001_initial.sql, 002_source_health.sql, 003_indexes.sql
templates/        # Askama .html files mirroring routes structure
tests/
  http.rs         # integration tests (tower::ServiceExt::oneshot)
  fixtures/
    seed.sql      # 5 test channels, 4 sources, 2 playlist items
```

---

## Architecture notes

**Library crate**: `src/lib.rs` is the library target; `src/main.rs` is the binary. Integration tests import `mytv::{build_router, AppState, ...}` from the library. Adding new public items requires making them `pub` in `lib.rs`.

**Router layers**: `redirect_trailing_slash` is registered with `.layer()` (outermost) so it fires before route matching and before auth middleware. Auth is registered with `.route_layer()` on the admin sub-router. Consequence: `GET /admin/` returns 308 (redirect) before auth ever runs — use `/admin` (no trailing slash) to test auth.

**Player routes return JSON**: `/channel/:id/tune` and `/channel/:id/next` return `Json<TuneResponse>` with HTTP 200 on success or HTTP 503 on failure. They do not redirect.

**VOD position**: `tune_vod_at` computes current playlist position using `Utc::now()` and the channel's `loop_anchor`. The returned URL depends on current time — tests assert `url.contains(...)` rather than exact equality.

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

---

## Conventions

- No comments unless the WHY is non-obvious
- Askama templates use `{% match %}` / `{% when %}` for `Option` types (not `{% if let %}`)
- Admin password is set via `ADMIN_PASSWORD` env var; default in dev is `admin`
- YouTube API key is optional (`YOUTUBE_API_KEY`) — discovery YouTube search is disabled if unset

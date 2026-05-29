# Behavior Tests — Design Spec

**Date:** 2026-05-29
**Scope:** HTTP integration tests using `tower::ServiceExt::oneshot` that verify route wiring, auth middleware, redirect middleware, and player route HTTP contracts.

---

## Problem

The existing 102 tests cover pure functions and handler logic well, but nothing tests the assembled HTTP layer. Route wiring bugs, accidentally unauthenticated admin routes, wrong status codes, and broken redirect middleware are all invisible to the current suite.

---

## Architecture

A single `tests/http.rs` integration test file calls the full Axum router as a `tower::Service` via `oneshot()`. No TCP socket is needed — requests are `http::Request` values, responses are `http::Response` values. Each test gets its own in-memory SQLite DB (migrations + seed data loaded fresh), so tests are fully isolated and stateless.

No new production dependencies. Dev dependencies needed: `tower` (feature `util`) and `http-body-util` — both already in the transitive dependency tree.

---

## Infrastructure Changes

### Extract router into `build_router`

`src/main.rs` currently assembles the Axum router inside `main()`. Extract it into:

```rust
pub fn build_router(state: AppState) -> Router
```

`main()` becomes: build state → call `build_router` → serve. No behavior change.

### Move `test_pool` to `src/db.rs`

The `test_pool()` helper is currently private inside each `model::*` test module. Move it to `src/db.rs` as:

```rust
#[cfg(test)]
pub async fn test_pool() -> SqlitePool
```

This makes it importable from `tests/http.rs`.

---

## Test Data

### `tests/fixtures/seed.sql`

Loaded after migrations in each test. Provides fixed IDs so tests reference scenarios by constant rather than by dynamic lookup.

```sql
INSERT INTO channels (id, name, category, type, sort_order) VALUES
  (1, 'Live OK',       'test', 'live',     1),
  (2, 'All Down',      'test', 'live',     2),
  (3, 'Has Fallback',  'test', 'live',     3),
  (4, 'VOD Has Items', 'test', 'vod_loop', 4),
  (5, 'VOD Empty',     'test', 'vod_loop', 5);

INSERT INTO sources (id, channel_id, kind, url, priority, is_active, consecutive_failures) VALUES
  (1, 1, 'hls', 'https://stream.example.com/live.m3u8',    1, 1, 0),
  (2, 2, 'hls', 'https://stream.example.com/down.m3u8',    1, 0, 3),
  (3, 3, 'hls', 'https://stream.example.com/primary.m3u8', 1, 0, 0),
  (4, 3, 'hls', 'https://stream.example.com/backup.m3u8',  2, 1, 0);

INSERT INTO playlist_items (channel_id, title, url, duration_secs, sort_order) VALUES
  (4, 'Episode 1', 'https://vod.example.com/ep1.mp4', 3600, 1),
  (4, 'Episode 2', 'https://vod.example.com/ep2.mp4', 3600, 2);
```

**Scenarios:**
- Channel 1 (`Live OK`): one active HLS source — happy path tune
- Channel 2 (`All Down`): one inactive source (3 failures) — tune returns 503
- Channel 3 (`Has Fallback`): primary inactive, backup active — fallback via `next`
- Channel 4 (`VOD Has Items`): two playlist items — tune returns 302 to stream URL
- Channel 5 (`VOD Empty`): no playlist items — tune returns 503

### Test helper setup

```rust
async fn app() -> Router {
    let pool = db::test_pool().await;
    sqlx::query(include_str!("fixtures/seed.sql"))
        .execute(&pool).await.unwrap();
    let state = AppState {
        pool,
        config: Arc::new(test_config()),
        http_client: reqwest::Client::new(),
    };
    build_router(state)
}

fn test_config() -> Config {
    Config { admin_user: "admin".into(), admin_pass: "test".into(), port: 0, .. }
}
```

---

## Test Coverage

### Layer 1: Auth middleware (3 tests)

Verifies the Basic Auth middleware is applied to the entire `/admin` prefix. One route is sufficient — the middleware is registered at the router layer, not per-route.

| Request | Expected |
|---------|----------|
| `GET /admin/` — no credentials | 401 |
| `GET /admin/` — wrong password | 401 |
| `GET /admin/` — correct credentials | 200 |

### Layer 2: Smoke tests (5 tests)

Each route family gets one test asserting it exists and returns the right status. No assertion on body content.

| Request | Expected |
|---------|----------|
| `GET /health` | 200 |
| `GET /` | 308 redirect to `/guide` |
| `GET /guide` | 200 |
| `GET /guide/partial` | 200 |
| `GET /admin/channels` (authed) | 200 |

### Layer 3: Redirect middleware (1 test)

| Request | Expected |
|---------|----------|
| `GET /admin/channels/` (trailing slash, authed) | 308 redirect to `/admin/channels` |

### Layer 4: Player contract tests (6 tests)

Assert on status code and `Location` header. Do not assert on body content or exact business logic outcomes (those are covered in unit tests).

| Request | Expected |
|---------|----------|
| `GET /channel/1/tune` (live, active source) | 302, `Location` = `https://stream.example.com/live.m3u8` |
| `GET /channel/2/tune` (live, all sources inactive) | 503 |
| `GET /channel/3/next?failed=https://stream.example.com/primary.m3u8` (live, fallback) | 302, `Location` = `https://stream.example.com/backup.m3u8` |
| `GET /channel/3/next?failed=https://stream.example.com/backup.m3u8` (live, all failed) | 503 |
| `GET /channel/4/tune` (VOD, has playlist) | 302, `Location` contains `vod.example.com/ep` |
| `GET /channel/5/tune` (VOD, empty playlist) | 503 |

**Note on VOD tune:** The exact `Location` URL includes a `?start=` offset calculated from `Utc::now()`. Tests assert the URL *contains* the stream host/path, not the exact value.

---

## What Is Not Covered

| Area | Reason |
|------|--------|
| Response body HTML content | Too brittle — every UI change breaks it; not an HTTP contract |
| Admin CRUD mutations (`POST /admin/channels`) | Business logic already tested in `discover` unit tests |
| Stream proxy | Requires real TCP streaming; separate problem |
| VOD `next` HTTP contract | Depends on time-varying state; already covered in unit tests |
| Each admin route individually for auth | Auth is middleware — one test covers all |
| EPG schedule math, M3U parsing | Already unit-tested; HTTP layer adds nothing |

---

## File Layout

```
src/
  main.rs           — extract build_router (pub fn)
  db.rs             — add test_pool (pub, cfg(test))
tests/
  http.rs           — all 15 behavior tests
  fixtures/
    seed.sql        — test channel/source/playlist data
```

**Total new tests: 15** (3 auth + 5 smoke + 1 redirect + 6 player contract)

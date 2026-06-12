# Narrow the Health Probe Interface — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the five-function health-probe surface with a single `probe(target)` that classifies and dispatches internally, so callers stop branching on "is this a live source?".

**Architecture:** Introduce `ProbeTarget::{Source, PlaylistItem}` and one public `probe()` in `src/health.rs` that folds in the resolved-CORS step (live sources) and the VOD-host-mark step (needs-resolution playlist items) that callers currently hand-roll. Migrate the four admin/api test handlers, then delete the old `probe_source`/`probe_playlist_item` and privatize `probe_and_cache_resolved_cors`. `record_source_liveness`, `probe_and_cache_cors`, and the background-sweep helpers are untouched.

**Tech Stack:** Rust 1.96, Axum 0.7, SQLx 0.7 (SQLite), tokio. `cargo test` / `cargo fmt` / `cargo clippy --all-targets -- -D warnings`.

**Spec:** `docs/superpowers/specs/2026-06-12-health-probe-interface-design.md`

---

## File Structure

- `src/health.rs` — add `ProbeTarget` enum + `probe()`; later remove `probe_source`/`probe_playlist_item`, privatize `probe_and_cache_resolved_cors`. New + rewritten unit tests live in its `#[cfg(test)] mod tests`.
- `src/routes/admin/sources.rs` — `source_test`: call `probe(Source)`, delete the `needs_resolution → probe_and_cache_resolved_cors` block.
- `src/routes/api/sources.rs` — `test`: call `probe(Source)`.
- `src/routes/admin/playlist.rs` — `playlist_item_test`: call `probe(PlaylistItem)`, delete the inline `needs_resolution → cors_cache.insert` block.
- `src/routes/api/playlist.rs` — `test`: call `probe(PlaylistItem)`.
- `src/lib.rs` — no change expected (`health` module already public; `probe`/`ProbeTarget` become reachable via `crate::health::`).

---

### Task 1: Introduce `ProbeTarget` + `probe()`

`probe()` is added with the full pipeline; the old `probe_source`/`probe_playlist_item`/`probe_and_cache_resolved_cors` are left untouched this task (callers still use them). `probe` is `pub`, so the temporarily-unused fn raises no dead-code/clippy warning.

**Files:**
- Modify: `src/health.rs` (add enum + fn after `probe_playlist_item`, ~line 225)
- Test: `src/health.rs` `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/health.rs`. This proves `probe()` routes a `PlaylistItem` whose URL needs resolution (a YouTube URL) to the VOD-host-mark branch — deterministic, no network (a `youtube_live`-detected item with no live_cache short-circuits the HTTP check to healthy, and `probe_and_cache_cors` no-ops on a resolution-needed URL).

```rust
    #[tokio::test]
    async fn probe_marks_needs_resolution_vod_item_host_direct() {
        let pool = crate::db::connect("sqlite::memory:").await.unwrap();
        let ch = crate::model::channel::create(
            &pool,
            crate::model::channel::NewChannel {
                name: "vod".to_string(),
                category: "test".to_string(),
                logo_url: None,
                channel_type: crate::model::channel::ChannelType::VodLoop,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap();
        let item = crate::model::playlist_item::create(
            &pool,
            crate::model::playlist_item::NewPlaylistItem {
                channel_id: ch.id,
                title: "rec".to_string(),
                url: "https://www.youtube.com/watch?v=abc123".to_string(),
                duration_secs: 3600,
                sort_order: 0,
            },
        )
        .await
        .unwrap();

        let client = reqwest::Client::new();
        let cors_cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let live_cache: LiveStatusCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

        probe(
            &pool,
            &client,
            &cors_cache,
            &live_cache,
            ProbeTarget::PlaylistItem(&item),
        )
        .await;

        let host = crate::media::hls::extract_manifest_host(&item.url);
        assert_eq!(
            cors_cache.read().await.get(&host).copied(),
            Some(true),
            "needs-resolution VOD item host must be marked Direct"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib health::tests::probe_marks_needs_resolution_vod_item_host_direct`
Expected: FAIL to compile — `cannot find function 'probe'` / `cannot find type 'ProbeTarget'`.

- [ ] **Step 3: Add the enum + `probe()`**

Insert into `src/health.rs` immediately after the `probe_playlist_item` function (before `probe_and_cache_cors`):

```rust
/// A health-probe target: a live/VOD source row, or a playlist item. `probe`
/// classifies the variant and runs the full pipeline (health update, CORS cache,
/// and — for resolution-needed URLs — the resolved-CDN CORS probe / VOD host mark)
/// internally, so callers never branch on the URL kind.
pub enum ProbeTarget<'a> {
    Source(&'a Source),
    PlaylistItem(&'a crate::model::playlist_item::PlaylistItem),
}

/// Probes a target's health and warms the CORS cache, without touching `is_active`.
/// Used by the admin + JSON-API Test buttons. Dispatches on the target kind:
/// a `Source` consults the live-status cache and, when the URL needs resolution,
/// resolves it to probe the real CDN's CORS; a `PlaylistItem` skips the live cache
/// and, when its URL needs resolution, marks the host Direct (VOD items bypass the
/// proxy entirely).
pub async fn probe(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    live_cache: &LiveStatusCache,
    target: ProbeTarget<'_>,
) {
    match target {
        ProbeTarget::Source(src) => {
            let ok = run_check(
                client,
                &src.url,
                &src.kind,
                src.consecutive_failures,
                Some(live_cache),
                |status, reason, failures| async move {
                    source::update_health(pool, src.id, status, reason.as_deref(), failures, None)
                        .await
                },
            )
            .await;
            if ok {
                probe_and_cache_cors(client, cors_cache, &src.url).await;
            }
            if crate::media::resolver::needs_resolution(&src.url) {
                probe_and_cache_resolved_cors(client, cors_cache, &src.url).await;
            }
        }
        ProbeTarget::PlaylistItem(item) => {
            let kind = crate::model::source::SourceKind::detect(&item.url);
            let ok = run_check(
                client,
                &item.url,
                kind.as_str(),
                item.consecutive_failures,
                None,
                |status, reason, failures| async move {
                    crate::model::playlist_item::update_health(
                        pool,
                        item.id,
                        status,
                        reason.as_deref(),
                        failures,
                        None,
                    )
                    .await
                },
            )
            .await;
            if ok {
                probe_and_cache_cors(client, cors_cache, &item.url).await;
            }
            if crate::media::resolver::needs_resolution(&item.url) {
                let host = crate::media::hls::extract_manifest_host(&item.url);
                cors_cache.write().await.insert(host, true);
            }
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib health::tests::probe_marks_needs_resolution_vod_item_host_direct`
Expected: PASS.

- [ ] **Step 5: Format, lint, full compile of test targets, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --no-run
git add src/health.rs
git commit -m "feat(health): add ProbeTarget + unified probe() entry point

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```
Expected: clippy clean, `cargo test --no-run` compiles all targets.

---

### Task 2: Migrate the source test handlers to `probe()`

Both source test handlers switch to `probe(ProbeTarget::Source(&src))`. The admin handler's separate `needs_resolution → probe_and_cache_resolved_cors` block is deleted (now internal to `probe`). The api handler gains that behavior (approved reconciliation).

**Files:**
- Modify: `src/routes/admin/sources.rs:89-110` (`source_test`)
- Modify: `src/routes/api/sources.rs:116` (`test`)

- [ ] **Step 1: Update `admin/sources.rs::source_test`**

Replace the `probe_source(...)` call AND the following `if crate::media::resolver::needs_resolution(&src.url) { probe_and_cache_resolved_cors(...) }` block with a single call. The current code is:

```rust
    crate::health::probe_source(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        &src,
    )
    .await;

    // Unlike playlist_item_test (VOD items skip the proxy entirely, so their host is
    // marked Direct outright), a live source still proxies its manifest and only loads
    // segments direct when the resolved CDN sends CORS — so we must actually resolve
    // and probe to learn the real budget badge.
    if crate::media::resolver::needs_resolution(&src.url) {
        crate::health::probe_and_cache_resolved_cors(
            &state.http_client,
            &state.cors_cache,
            &src.url,
        )
        .await;
    }
```

Replace it with:

```rust
    crate::health::probe(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        crate::health::ProbeTarget::Source(&src),
    )
    .await;
```

- [ ] **Step 2: Update `api/sources.rs::test`**

Replace:

```rust
    crate::health::probe_source(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        &src,
    )
    .await;
```

with:

```rust
    crate::health::probe(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        crate::health::ProbeTarget::Source(&src),
    )
    .await;
```

- [ ] **Step 3: Format, lint, test, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --test http --test api
git add src/routes/admin/sources.rs src/routes/api/sources.rs
git commit -m "refactor(health): route source test handlers through probe()

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```
Expected: integration tests green.

---

### Task 3: Migrate the playlist test handlers to `probe()`

Both playlist test handlers switch to `probe(ProbeTarget::PlaylistItem(&item))`. The admin handler's inline `needs_resolution → cors_cache.insert` block is deleted (now internal to `probe`). The api handler gains the VOD-host-mark behavior (approved reconciliation).

**Files:**
- Modify: `src/routes/admin/playlist.rs:124-131` (`playlist_item_test`)
- Modify: `src/routes/api/playlist.rs:125` (`test`)

- [ ] **Step 1: Update `admin/playlist.rs::playlist_item_test`**

Replace the `probe_playlist_item(...)` call AND the following `if media::resolver::needs_resolution(&item.url) { ... cors_cache.insert ... }` block. The current code is:

```rust
    crate::health::probe_playlist_item(&state.pool, &state.http_client, &state.cors_cache, &item)
        .await;

    if media::resolver::needs_resolution(&item.url) {
        let host = media::hls::extract_manifest_host(&item.url);
        state.cors_cache.write().await.insert(host, true);
    }
```

Replace it with:

```rust
    crate::health::probe(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        crate::health::ProbeTarget::PlaylistItem(&item),
    )
    .await;
```

Note: this introduces a use of `state.live_cache` (ignored internally for the `PlaylistItem` variant). If the `media` import is now unused in this file after deleting the `media::resolver` / `media::hls` references, remove the now-dead `use` line — `cargo clippy -D warnings` will flag it. Verify with clippy before committing.

- [ ] **Step 2: Update `api/playlist.rs::test`**

Replace:

```rust
    crate::health::probe_playlist_item(&state.pool, &state.http_client, &state.cors_cache, &item)
        .await;
```

with:

```rust
    crate::health::probe(
        &state.pool,
        &state.http_client,
        &state.cors_cache,
        &state.live_cache,
        crate::health::ProbeTarget::PlaylistItem(&item),
    )
    .await;
```

- [ ] **Step 3: Format, lint, test, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --test http --test api
git add src/routes/admin/playlist.rs src/routes/api/playlist.rs
git commit -m "refactor(health): route playlist test handlers through probe()

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```
Expected: integration tests green.

---

### Task 4: Remove the old entry points; privatize resolved-CORS

With all callers on `probe()`, delete the now-unused public `probe_source` and `probe_playlist_item`, and change `probe_and_cache_resolved_cors` from `pub` to private (`async fn`). Rewrite the two reenable unit tests to call `probe()`.

**Files:**
- Modify: `src/health.rs` (delete `probe_source` ~123-145, delete `probe_playlist_item` ~195-225, drop `pub` on `probe_and_cache_resolved_cors` ~267; rewrite two tests)

- [ ] **Step 1: Rewrite the two reenable tests to call `probe()`**

In `src/health.rs` `mod tests`, in `probe_source_does_not_reenable_disabled_source`, replace the probe call:

```rust
        // probe_source is the manual Test-button path — must never change is_active.
        probe_source(&pool, &client, &cors_cache, &live_cache, &src).await;
```

with:

```rust
        // probe() is the manual Test-button path — must never change is_active.
        probe(
            &pool,
            &client,
            &cors_cache,
            &live_cache,
            ProbeTarget::Source(&src),
        )
        .await;
```

In `probe_playlist_item_does_not_reenable_disabled_item`, the test builds a `cors_cache` but no `live_cache`. Add a `live_cache` and replace the probe call. Replace:

```rust
        probe_playlist_item(&pool, &client, &cors_cache, &it).await;
```

with:

```rust
        let live_cache: LiveStatusCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        probe(
            &pool,
            &client,
            &cors_cache,
            &live_cache,
            ProbeTarget::PlaylistItem(&it),
        )
        .await;
```

- [ ] **Step 2: Run the rewritten tests — verify they still fail-safe / pass against the old fns**

Run: `cargo test --lib health::tests::probe_source_does_not_reenable_disabled_source health::tests::probe_playlist_item_does_not_reenable_disabled_item`
Expected: PASS (they now exercise `probe()`, which already exists from Task 1).

- [ ] **Step 3: Delete `probe_source` and `probe_playlist_item`; privatize `probe_and_cache_resolved_cors`**

Delete the entire `pub async fn probe_source(...) { ... }` block (with its doc comment) and the entire `pub async fn probe_playlist_item(...) { ... }` block (with its doc comment) from `src/health.rs`.

Change the signature line of `probe_and_cache_resolved_cors` from:

```rust
pub async fn probe_and_cache_resolved_cors(
```

to:

```rust
async fn probe_and_cache_resolved_cors(
```

(Its doc comment and body stay. It is still called by `probe()` and its tests live in the same module, so private visibility is sufficient.)

- [ ] **Step 4: Format, lint, full test compile + run**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --no-run
cargo test
```
Expected: no `unused`/`dead_code` warnings, all tests pass. If clippy reports `probe_and_cache_resolved_cors` is never used, that means a caller was missed — re-check Task 2.

- [ ] **Step 5: Commit**

```bash
git add src/health.rs
git commit -m "refactor(health): remove probe_source/probe_playlist_item, privatize resolved-CORS

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification (whole diff)

- [ ] `cargo fmt --check` — clean
- [ ] `cargo clippy --all-targets -- -D warnings` — clean
- [ ] `cargo test` — all pass (incl. lib `#[cfg(test)]` modules)
- [ ] `grep -rn "probe_source\|probe_playlist_item" src` returns nothing (the private sweep helpers are `check_source`/`check_playlist_item`, not these)
- [ ] `grep -rn "pub async fn probe_and_cache_resolved_cors" src` returns nothing
- [ ] Acceptance criteria in the spec all satisfied: public probe surface is `probe` + `probe_and_cache_cors` + `record_source_liveness`; no caller branches on `needs_resolution` around a probe call.

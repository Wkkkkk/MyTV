# Ended-Live → VOD Testable Seam Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the ended-live → VOD conversion into a top-level `src/broadcast.rs` deep module whose awaitable core (`convert_if_ended`) takes the yt-dlp resolution as an injected closure, making the entire conversion testable without a network.

**Architecture:** A new `pub mod broadcast` owns the conversion. `convert_if_ended` is the awaitable test surface: it calls an injected `resolve` closure for `(watch_url, duration)`, then performs the atomic flip → append → deactivate. `resolve_recording` is the production closure (yt-dlp), and `spawn_conversion` is a one-line `tokio::spawn` adapter. `player.rs`'s `next_live` calls `broadcast::spawn_conversion`; its three old conversion fns and their two unit tests are deleted (coverage moves to `broadcast.rs`).

**Tech Stack:** Rust 1.96, Axum 0.7, SQLx 0.7 (SQLite, in-memory for tests), tokio, anyhow, chrono.

**Reference:** `docs/superpowers/specs/2026-06-12-broadcast-seam-design.md`

**Conventions (every task):**
- Run `cargo fmt` before every commit — CI fails on any diff (toolchain pinned to Rust 1.96).
- `cargo build` and `cargo test --test <x>` do NOT compile the library's `#[cfg(test)]` modules. Always run `cargo test --no-run` to compile all test targets before trusting (or distrusting) compiler/LSP errors.
- No comments unless the WHY is non-obvious.
- End every commit message with: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

### Task 1: Create the `broadcast` module with the awaitable core + full test surface

**Files:**
- Create: `src/broadcast.rs`
- Modify: `src/lib.rs:1-11` (add `pub mod broadcast;` in alphabetical position — before `pub mod budget;`)

This task adds the new module alongside the existing (still-wired) `player.rs` conversion fns. Both coexist after this task; `convert_if_ended` is `pub` so it does not trigger a dead-code warning despite having no production caller yet (it is wired in Task 2).

- [ ] **Step 1: Register the module in `src/lib.rs`**

`src/lib.rs` currently begins:
```rust
pub mod budget;
pub mod config;
```
Add `broadcast` before `budget` (alphabetical):
```rust
pub mod broadcast;
pub mod budget;
pub mod config;
```

- [ ] **Step 2: Write `src/broadcast.rs` with the core, the production adapters' stubs deferred to Task 2, and the four failing tests**

Create `src/broadcast.rs` with exactly this content:

```rust
use std::future::Future;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::model::{channel, playlist_item, source};

/// Outcome of an ended-live → VOD conversion attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum ConversionOutcome {
    /// This call won the idempotency claim: flipped the channel to vod_loop,
    /// appended the recording, deactivated the live sources.
    Converted,
    /// The channel was already a VOD loop (a concurrent or repeat tune won the
    /// claim first) — nothing to do.
    AlreadyConverted,
}

/// Converts an ended live channel into a VOD loop. The recording's watch URL
/// and duration come from the injected `resolve` closure (yt-dlp in production,
/// a stub in tests), then the atomic flip → append → deactivate runs.
///
/// Resolve runs *before* the claim: claiming first then failing the resolve
/// would leave the channel flipped with no playlist item (an empty VOD → 503).
/// The flip (`set_type_and_anchor_if_live`) is the idempotency gate — an
/// already-converted channel yields `AlreadyConverted` without appending, and
/// two racing tunes append exactly one item.
pub async fn convert_if_ended<F, Fut>(
    pool: &SqlitePool,
    channel_id: i64,
    title: &str,
    source_url: &str,
    anchor: DateTime<Utc>,
    resolve: F,
) -> anyhow::Result<ConversionOutcome>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = anyhow::Result<(String, i64)>>,
{
    let (watch_url, duration) = resolve(source_url.to_string()).await?;
    if !channel::set_type_and_anchor_if_live(pool, channel_id, anchor).await? {
        return Ok(ConversionOutcome::AlreadyConverted);
    }
    playlist_item::create(
        pool,
        playlist_item::NewPlaylistItem {
            channel_id,
            title: title.to_string(),
            url: watch_url,
            duration_secs: duration,
            sort_order: 0,
        },
    )
    .await?;
    source::deactivate_all_for_channel(pool, channel_id).await?;
    Ok(ConversionOutcome::Converted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::DateTime;

    async fn test_pool() -> SqlitePool {
        db::connect("sqlite::memory:").await.unwrap()
    }

    async fn make_live_channel(pool: &SqlitePool) -> channel::Channel {
        channel::create(
            pool,
            channel::NewChannel {
                name: "Live Test".into(),
                category: "test".into(),
                logo_url: None,
                channel_type: channel::ChannelType::Live,
                sort_order: 0,
                loop_anchor: None,
            },
        )
        .await
        .unwrap()
    }

    async fn make_live_source(pool: &SqlitePool, channel_id: i64) {
        source::create(
            pool,
            source::NewSource {
                channel_id,
                kind: source::SourceKind::YoutubeLive,
                url: "https://www.youtube.com/live/abc123".into(),
                priority: 1,
            },
        )
        .await
        .unwrap();
    }

    async fn stub_ok(_url: String) -> anyhow::Result<(String, i64)> {
        Ok(("https://www.youtube.com/watch?v=abc123".to_string(), 212))
    }

    async fn stub_err(_url: String) -> anyhow::Result<(String, i64)> {
        Err(anyhow::anyhow!("resolve failed"))
    }

    #[tokio::test]
    async fn convert_if_ended_flips_and_appends() {
        let pool = test_pool().await;
        let ch = make_live_channel(&pool).await;
        make_live_source(&pool, ch.id).await;
        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        let outcome = convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok)
            .await
            .unwrap();
        assert_eq!(outcome, ConversionOutcome::Converted);

        let updated = channel::get(&pool, ch.id).await.unwrap().unwrap();
        assert_eq!(updated.channel_type(), channel::ChannelType::VodLoop);
        assert_eq!(updated.loop_anchor, Some(anchor));

        let items = playlist_item::list_active_for_channel(&pool, ch.id)
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://www.youtube.com/watch?v=abc123");
        assert_eq!(items[0].duration_secs, 212);
        assert_eq!(items[0].title, "Live Test");

        assert!(source::list_active_for_channel(&pool, ch.id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn convert_if_ended_is_idempotent() {
        let pool = test_pool().await;
        let ch = make_live_channel(&pool).await;
        make_live_source(&pool, ch.id).await;
        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok)
            .await
            .unwrap();
        let outcome = convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok)
            .await
            .unwrap();
        assert_eq!(outcome, ConversionOutcome::AlreadyConverted);

        assert_eq!(
            playlist_item::list_active_for_channel(&pool, ch.id)
                .await
                .unwrap()
                .len(),
            1,
            "second conversion must not append a duplicate item"
        );
    }

    #[tokio::test]
    async fn convert_if_ended_concurrent_appends_once() {
        let pool = test_pool().await;
        let ch = make_live_channel(&pool).await;
        make_live_source(&pool, ch.id).await;
        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        let (a, b) = tokio::join!(
            convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok),
            convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_ok),
        );
        a.unwrap();
        b.unwrap();

        assert_eq!(
            playlist_item::list_active_for_channel(&pool, ch.id)
                .await
                .unwrap()
                .len(),
            1,
            "two racing conversions must append exactly one item"
        );
    }

    #[tokio::test]
    async fn convert_if_ended_resolve_failure_leaves_channel_live() {
        let pool = test_pool().await;
        let ch = make_live_channel(&pool).await;
        make_live_source(&pool, ch.id).await;
        let anchor = DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        let result = convert_if_ended(&pool, ch.id, "Live Test", "src", anchor, stub_err).await;
        assert!(result.is_err());

        let updated = channel::get(&pool, ch.id).await.unwrap().unwrap();
        assert_eq!(
            updated.channel_type(),
            channel::ChannelType::Live,
            "a failed resolve must not flip the channel"
        );
        assert!(
            playlist_item::list_active_for_channel(&pool, ch.id)
                .await
                .unwrap()
                .is_empty(),
            "a failed resolve must not append an item"
        );
    }
}
```

> **Note on the stub resolvers:** `stub_ok` / `stub_err` are free `async fn`s. A function item is a zero-sized type implementing `FnOnce(String) -> impl Future` and is `Copy`, so the same name can be passed to multiple `convert_if_ended` calls (including both arms of `tokio::join!`) without a move conflict. Both satisfy the bound `F: FnOnce(String) -> Fut, Fut: Future<Output = anyhow::Result<(String, i64)>>`.

- [ ] **Step 3: Verify the tests compile and pass**

Run: `cargo test --no-run` first (compiles all test targets, incl. lib `#[cfg(test)]`).
Then: `cargo test --lib broadcast:: -- --nocapture`
Expected: 4 tests pass — `convert_if_ended_flips_and_appends`, `convert_if_ended_is_idempotent`, `convert_if_ended_concurrent_appends_once`, `convert_if_ended_resolve_failure_leaves_channel_live`.

- [ ] **Step 4: Format, lint, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/broadcast.rs src/lib.rs
git commit -m "feat(broadcast): add convert_if_ended core with injected resolver

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```
Expected: clippy clean, commit succeeds.

---

### Task 2: Add production adapters, wire `next_live`, delete the old `player.rs` conversion code

**Files:**
- Modify: `src/broadcast.rs` (append `resolve_recording` + `spawn_conversion` after `convert_if_ended`, before `#[cfg(test)]`)
- Modify: `src/routes/player.rs` (rewire `next_live`'s `Ended` arm; delete `convert_channel_to_vod_loop`, `live_to_vod_conversion`, `spawn_live_to_vod_conversion` and their two unit tests)

- [ ] **Step 1: Add the production resolver + spawn adapter to `src/broadcast.rs`**

Insert these two items immediately after the `convert_if_ended` function and before `#[cfg(test)]`. Add `use crate::media::resolver;` to the module's `use` block (it currently imports only `channel`, `playlist_item`, `source`).

The top-of-file `use` block becomes:
```rust
use std::future::Future;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::media::resolver;
use crate::model::{channel, playlist_item, source};
```

The two new items:
```rust
/// Production resolver: derives the recording's canonical watch URL (from the
/// embedded id, falling back to a yt-dlp id lookup) and its duration via yt-dlp.
pub async fn resolve_recording(source_url: String) -> anyhow::Result<(String, i64)> {
    let watch_url = match resolver::live_url_to_watch_url(&source_url) {
        Some(u) => u,
        None => {
            let id = resolver::fetch_video_id(&source_url).await?;
            format!("https://www.youtube.com/watch?v={id}")
        }
    };
    let duration = resolver::fetch_duration_secs(&watch_url).await?;
    Ok((watch_url, duration))
}

/// Thin adapter: fire the conversion as a detached task using the real yt-dlp
/// resolver. Failures are logged and dropped — the broadcast simply stays live
/// until the next tune retries.
pub fn spawn_conversion(pool: SqlitePool, channel_id: i64, title: String, source_url: String) {
    tokio::spawn(async move {
        if let Err(e) =
            convert_if_ended(&pool, channel_id, &title, &source_url, Utc::now(), resolve_recording)
                .await
        {
            tracing::warn!(channel_id, error = %e, "ended-live → VOD conversion failed");
        }
    });
}
```

- [ ] **Step 2: Rewire `next_live`'s `Ended` arm in `src/routes/player.rs`**

Find this arm (around `src/routes/player.rs:186-189`):
```rust
                Some(LiveOutcome::Ended) => {
                    spawn_live_to_vod_conversion(state, ch.id, ch.name.clone(), src.url.clone());
                    return Ok(tune_response_ended(ch));
                }
```
Replace the `spawn_live_to_vod_conversion(...)` line so the arm reads:
```rust
                Some(LiveOutcome::Ended) => {
                    crate::broadcast::spawn_conversion(
                        state.pool.clone(),
                        ch.id,
                        ch.name.clone(),
                        src.url.clone(),
                    );
                    return Ok(tune_response_ended(ch));
                }
```

- [ ] **Step 3: Delete the three old conversion fns from `src/routes/player.rs`**

Delete these three functions in full (around `src/routes/player.rs:229-302`): `convert_channel_to_vod_loop`, `spawn_live_to_vod_conversion`, and `live_to_vod_conversion` — including the doc comment above `convert_channel_to_vod_loop`. The block to remove runs from the `/// DB-only conversion of an ended live channel...` doc comment through the closing brace of `live_to_vod_conversion`.

- [ ] **Step 4: Delete the two now-orphaned tests from `src/routes/player.rs`**

In the `#[cfg(test)] mod tests` block, delete the `// ── convert_channel_to_vod_loop ──...` section header comment and both tests it introduces: `test_convert_channel_to_vod_loop` and `test_convert_channel_to_vod_loop_concurrent_appends_once` (around `src/routes/player.rs:1142-1256`). Their coverage now lives in `broadcast.rs`.

- [ ] **Step 5: Verify everything compiles and passes**

Run: `cargo test --no-run`
Expected: compiles with no errors and no `unused`/dead-code warnings (the three deleted fns are gone; `is_ended_live`, `classify_live_outcome`, `LiveOutcome` remain in use by `next_live`).

Then run the focused suites:
```bash
cargo test --lib broadcast::
cargo test --lib player
```
Expected: all pass; no reference to the deleted fns remains.

Sanity grep (expect no output):
```bash
grep -rn "convert_channel_to_vod_loop\|live_to_vod_conversion\|spawn_live_to_vod_conversion" src/
```
Expected: no matches.

- [ ] **Step 6: Full suite, format, lint, commit**

```bash
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
git add src/broadcast.rs src/routes/player.rs
git commit -m "refactor(broadcast): route ended-live conversion through broadcast::spawn_conversion

Delete player.rs's convert_channel_to_vod_loop / live_to_vod_conversion /
spawn_live_to_vod_conversion; next_live now calls broadcast::spawn_conversion.
Conversion coverage moves to broadcast.rs's convert_if_ended tests.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```
Expected: full test suite green, clippy clean, commit succeeds.

---

## Acceptance Criteria (verify after Task 2)

1. `src/broadcast.rs` exposes `ConversionOutcome`, `convert_if_ended`, `resolve_recording`, `spawn_conversion`; `src/lib.rs` declares `pub mod broadcast;`.
2. `convert_if_ended` is the awaitable core with yt-dlp resolution injected as a closure. `convert_channel_to_vod_loop`, `live_to_vod_conversion`, `spawn_live_to_vod_conversion` are removed from `player.rs`; `next_live` calls `broadcast::spawn_conversion`.
3. The four `broadcast.rs` tests pass with no network; the resolve-failure test asserts the channel is not flipped and no item is appended.
4. `cargo test` (all targets, incl. `--no-run`), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` all green.

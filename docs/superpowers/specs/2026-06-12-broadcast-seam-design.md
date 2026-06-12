# Spec — Give the ended-live → VOD flow a testable seam

_Candidate #3 of the architecture-deepening effort (`docs/architecture/changes-20260612.md` §3).
Created 2026-06-12._

## Problem

When a live broadcast ends, `next_live` (in `src/routes/player.rs`) converts the channel into a
VOD loop: it derives the recording's canonical `watch?v=` URL, fetches its duration, flips the
channel type, appends the recording as a playlist item, and deactivates the now-finished live
sources. Today this is spread across three functions, all private to `player.rs`:

- `spawn_live_to_vod_conversion` — fires the work as a detached `tokio::spawn`.
- `live_to_vod_conversion` — the **network half**: derives the watch URL (embedded id, or a
  yt-dlp `fetch_video_id` fallback) and fetches duration via yt-dlp.
- `convert_channel_to_vod_loop` — the **DB half**: atomic claim (`set_type_and_anchor_if_live`),
  append item, deactivate sources. Idempotent. Already unit-tested directly.

Two coupled problems:

1. **The conversion runs detached.** A test driving the request path cannot observe whether the
   VOD was appended — the `tokio::spawn` has no await point the test can reach.
2. **Tests reach _past_ the seam.** The only directly tested piece is the DB tail
   (`convert_channel_to_vod_loop`). The watch-URL derivation, the duration fetch, and—critically—
   the **failure handling** (a resolve error is swallowed by the spawn and merely logged) are
   invisible to tests. The interface is not the test surface.

## Solution

Extract a top-level `src/broadcast.rs` module that owns the entire conversion as one deep unit
with a single awaitable entry point. The yt-dlp resolution is **injected as a closure**, so the
full conversion—watch-URL derivation, duration, flip, append, idempotency, failure handling—is
testable with a deterministic stub and no network. The `tokio::spawn` becomes a one-line adapter.

### Module location

`src/broadcast.rs`, declared `pub mod broadcast;` in `src/lib.rs` (sibling of `health`, `epg`,
`budget`). It orchestrates `model::{channel, playlist_item, source}` plus `media::resolver` — a
lifecycle concern, not a media primitive — so a top-level home reads cleanly and matches the
review's `broadcast::convert_if_ended` naming.

### Public surface

**1. Outcome type** — replaces the silent `Result<()>` with an explicit, assertable outcome:

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum ConversionOutcome {
    /// This call won the idempotency claim: flipped the channel to vod_loop,
    /// appended the recording, deactivated the live sources.
    Converted,
    /// The channel was already a VOD loop (a concurrent or repeat tune won the
    /// claim first) — nothing to do.
    AlreadyConverted,
}
```

**2. The awaitable core** — the test surface; resolver injected:

```rust
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
```

This folds today's `live_to_vod_conversion` + `convert_channel_to_vod_loop` into one function.

- **`resolve` is `FnOnce(String) -> Fut`** returning `(watch_url, duration_secs)`. The production
  closure is `resolve_recording` (below); tests pass a deterministic stub.
- **`anchor` is a parameter** (not `Utc::now()` inside), so tests pass a fixed timestamp and
  assert exact equality — exactly as the existing DB tests already do.
- **Ordering preserved: resolve _then_ claim.** Claiming first then failing the resolve would
  leave the channel flipped to `vod_loop` with no playlist item (an empty VOD → 503). This matches
  today's `live_to_vod_conversion` order. Two racing tunes therefore both run the resolver, but
  only one wins `set_type_and_anchor_if_live` and appends — exactly one item.
- **Name.** `convert_if_ended` matches the review/tracking-memory naming. The "if" maps to the
  idempotency claim: the conversion proceeds only if the channel is _still live_; an
  already-converted channel yields `AlreadyConverted` without appending. The caller (`next_live`)
  has already decided the broadcast ended before calling.

**3. Production resolver + spawn adapter:**

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
/// resolver. Failures are logged and dropped (the broadcast simply stays live
/// until the next tune retries).
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

## Caller changes (`src/routes/player.rs`)

- `next_live`'s `Some(LiveOutcome::Ended)` arm replaces
  `spawn_live_to_vod_conversion(state, ch.id, ch.name.clone(), src.url.clone());` with
  `crate::broadcast::spawn_conversion(state.pool.clone(), ch.id, ch.name.clone(), src.url.clone());`.
- Delete `convert_channel_to_vod_loop`, `live_to_vod_conversion`, `spawn_live_to_vod_conversion`.
- Move their unit tests (`test_convert_channel_to_vod_loop`,
  `test_convert_channel_to_vod_loop_concurrent_appends_once`) into `broadcast.rs`, rewritten
  against `convert_if_ended` with a stub resolver.

## Stays in `player.rs` (out of scope)

- `is_ended_live`, `classify_live_outcome`, `LiveOutcome` and their tests — handler-side
  source-selection logic that feeds the Play and Waiting outcomes too, not just Ended.
- The `next_live` source-iteration loop itself.

## Testing — the win

The test surface becomes `convert_if_ended` itself. New `broadcast.rs` `#[cfg(test)]` module, each
test using an in-memory pool (mirroring player.rs's `test_state`) and a stub resolver — **no
network, no yt-dlp**:

1. **Converted** — stub returns `Ok(("https://www.youtube.com/watch?v=abc123".into(), 212))`:
   outcome is `Converted`; channel flipped to `vod_loop` with `loop_anchor == anchor`; exactly one
   playlist item with the stubbed url + duration + title; live sources deactivated.
2. **AlreadyConverted / idempotent** — a second call on the already-converted channel returns
   `AlreadyConverted` and appends no duplicate item.
3. **Concurrent** — two racing `convert_if_ended` calls (`tokio::join!`) append exactly one item.
4. **Resolve failure (new coverage)** — stub returns `Err(...)`: `convert_if_ended` returns `Err`;
   the channel is **still live** (not flipped); no playlist item is appended. This failure path was
   previously swallowed by the spawn and untestable.

`resolve_recording` and `spawn_conversion` are thin yt-dlp/spawn adapters and are not unit-tested
(the underlying `resolver::*` and `live_url_to_watch_url` are already covered in `resolver.rs`).

Existing `tests/http.rs` / `tests/api.rs` stay green; no integration test drove the detached
conversion before, so none changes.

## Acceptance criteria

1. New `src/broadcast.rs` exposes `ConversionOutcome`, `convert_if_ended`, `resolve_recording`,
   `spawn_conversion`; declared `pub mod broadcast;` in `src/lib.rs`.
2. `convert_if_ended` is the awaitable core with the yt-dlp resolution injected as a closure;
   `convert_channel_to_vod_loop`, `live_to_vod_conversion`, and `spawn_live_to_vod_conversion` are
   removed from `player.rs`; `next_live` calls `broadcast::spawn_conversion`.
3. The four tests above live in `broadcast.rs` and pass with no network. The resolve-failure test
   asserts the channel is not flipped.
4. `cargo test` (all targets, incl. `--no-run` to compile lib `#[cfg(test)]` modules),
   `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` all green.

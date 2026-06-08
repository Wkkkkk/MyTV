# Playlist Item Health Tracking — Design

**Date:** 2026-06-08
**Status:** Approved

## Goal

Add full health tracking and CORS/budget badges to VOD playlist items, matching the existing capability on live sources. The player skips disabled items so that broken VOD URLs don't interrupt playback.

## Approach

Approach A: add health columns to `playlist_items`. The `sources` table is unchanged. `playlist_items` gains the same five health fields already present on `sources`. The health checker, admin layer, and player are extended to use them.

## 1. Database

New migration `migrations/005_playlist_item_health.sql`:

```sql
ALTER TABLE playlist_items ADD COLUMN is_active           INTEGER NOT NULL DEFAULT 1;
ALTER TABLE playlist_items ADD COLUMN last_checked_at     INTEGER;
ALTER TABLE playlist_items ADD COLUMN last_status         TEXT CHECK(last_status IN ('ok', 'error'));
ALTER TABLE playlist_items ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playlist_items ADD COLUMN failure_reason      TEXT;

CREATE INDEX idx_playlist_items_is_active_channel_sort
    ON playlist_items(is_active, channel_id, sort_order);
```

All columns default to safe values — existing rows remain valid and all current tests pass without seed changes.

## 2. Model (`src/model/playlist_item.rs`)

`PlaylistItem` struct gains the five new fields:

```rust
pub is_active: bool,
pub last_checked_at: Option<i64>,
pub last_status: Option<String>,
pub consecutive_failures: i64,
pub failure_reason: Option<String>,
```

Three new DB functions (mirror of `source.rs`):

- `list_active_for_channel(pool, channel_id)` — `WHERE is_active = 1 ORDER BY sort_order ASC`
- `set_active(pool, id, active)` — manual admin toggle
- `update_health(pool, id, status, reason, consecutive_failures, is_active: Option<bool>)` — same signature as the source version; `None` leaves `is_active` unchanged (Test button path), `Some(bool)` changes it (background checker path)

`current_position` and `total_duration_secs` are pure slice functions — no changes needed. The caller passes active-only items for playback, all items for admin display.

## 3. Health checker (`src/health.rs`)

`do_http_check` is refactored from `(client, &Source)` to `(client, url: &str, kind: &str)` — allows reuse for playlist items. The `youtube_live` early-return stays, keyed on the kind string. Playlist item kind is inferred at check time via `SourceKind::detect(&item.url)` (no kind column needed on the table).

`probe_all_playlist_cors` is removed. Its work is absorbed by `check_all_playlist_items`, which runs a full health check (HTTP + CORS) per item — same auto-disable/re-enable logic as `check_source` (threshold: `FAILURE_THRESHOLD = 3`).

`check_all` becomes:

```rust
async fn check_all(pool, client, cors_cache) {
    for src  in source::list_all(pool)         { check_source(...)        .await; }
    for item in playlist_item::list_all(pool)  { check_playlist_item(...) .await; }
}
```

A new `probe_playlist_item` (mirror of `probe_source`) is the admin Test-button path — runs HTTP check + CORS probe, never touches `is_active`.

## 4. Admin layer

**`AdminPlaylistItemRow`** gains the same health fields as `AdminSourceRow`:

```rust
pub is_active: bool,
pub last_status: Option<String>,
pub consecutive_failures: i64,
pub failure_reason: Option<String>,
```

`From<PlaylistItem>` populates them directly.

**New route:** `POST /admin/playlist/:id/toggle` → `playlist_item_toggle` handler (mirrors `source_toggle`, calls `set_active`, redirects to channel detail).

**Template** `templates/admin/partials/playlist_item_row.html` gains the health badge (green/red/grey dot from `last_status`) and budget badge (⚡/☁ from CORS cache), matching the layout of `source_row.html`.

## 5. Player (`src/routes/player.rs`)

`vod_items_and_index` switches from `list_for_channel` to `list_active_for_channel`. If the active set is empty the existing empty-playlist 503 guard fires. Time-based position (`current_position`) runs over the active-only slice — disabled items don't contribute to total duration and are never landed on.

Loop anchor behaviour: `loop_anchor` is a fixed timestamp on the channel. `total_duration_secs` is computed from active items only, so disabling an item compresses the loop (the remaining active items cycle faster relative to wall-clock). This is the intended behaviour — the anchor stays unchanged and no migration is required when items are toggled.

## 6. Tests

**`src/model/playlist_item.rs`** — new unit tests mirroring source tests:
- `test_list_active_excludes_inactive_items`
- `test_set_active_toggles_item`
- `test_update_health_ok_resets_failures`
- `test_update_health_disables_after_threshold`
- `test_update_health_reenables_disabled_item`

**`src/health.rs`** — new unit tests:
- `process_result` variants reused for playlist items once `do_http_check` is refactored
- `probe_playlist_item_does_not_reenable_disabled_item`

**`src/routes/player.rs`** — new unit tests:
- `test_tune_vod_skips_disabled_item` — two items, first disabled, assert URL is second item
- `test_tune_vod_returns_503_when_all_items_disabled`

**`tests/http.rs`** — integration test variant against seed channel 4 with one item disabled.

## Files changed

| File | Change |
|------|--------|
| `migrations/005_playlist_item_health.sql` | new |
| `src/model/playlist_item.rs` | add fields, add 3 DB functions |
| `src/health.rs` | refactor `do_http_check`, remove `probe_all_playlist_cors`, add `check_playlist_item`, `probe_playlist_item` |
| `src/routes/admin/mod.rs` | add health fields to `AdminPlaylistItemRow` |
| `src/routes/admin/playlist.rs` | add `playlist_item_toggle`, update `playlist_item_test` |
| `src/lib.rs` | wire `playlist_item_toggle` route |
| `templates/admin/partials/playlist_item_row.html` | add health + budget badge columns |
| tests (unit + integration) | new test cases as listed above |

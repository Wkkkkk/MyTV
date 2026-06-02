# VOD CORS Budget Badge — Design

**Idea 12** in `docs/IDEAS.md`. Depends on idea 11's descend-into-master CORS probe (`media::hls::probe_source_cors`).

## Problem

The guide shows a per-channel network-budget badge (⚡ direct / ☁ proxied / blank unknown). `build_guide_data` derives budget only from the `sources` table: it builds `first_active_urls: HashMap<channel_id, url>` from active sources and calls `derive_budget_status`. VOD channels store their URLs in `playlist_items`, not `sources`, so they never appear in that map → always `Unknown` → no badge.

Two gaps must close:

1. **Derivation** — the guide budget must cover VOD playlist-item URLs.
2. **Cache population** — the CORS cache (keyed by `scheme://host`) is filled only by `health::check_source` (background checker + source Test button). VOD item hosts are never probed, so even after fixing derivation the badge would stay `Unknown` until those hosts get probed.

**YouTube/Twitch items**: `resolver::needs_resolution()` URLs (youtube.com, youtu.be, twitch.tv) resolve via yt-dlp at tune time and have no stable HLS manifest to probe — the same situation as `youtube_live` sources, which naturally yield `Unknown`. These are skipped from probing and render `·`.

## Decisions

- **Guide badge per VOD channel** = the **currently-playing** playlist item's URL (via `loop_anchor` + `playlist_item::current_position`), reflecting what a viewer gets if they tune now. Fallback: first item if no anchor; `None`/`Unknown` if the playlist is empty.
- **Probe trigger** = a per-item **Test** button (immediate) **and** the 15-min background health checker (durable across restarts, like live channels). The in-memory CORS cache is lost on restart; relying on the button alone would leave VOD badges `Unknown` until each item is manually re-tested.

## Components

### 1. Shared probe-and-cache helper (DRY)

The CORS-probe-then-cache block is currently inlined in `health::check_source`. Extract it into one reusable function used by `check_source`, the new playlist Test handler, and the background VOD sweep.

```rust
// health.rs — probes CORS for one URL and caches the result by host.
// Returns None (no-op) for non-HTTPS URLs or resolution-needed (youtube/twitch) URLs.
pub async fn probe_and_cache_cors(
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    url: &str,
) -> Option<bool> {
    if !url.starts_with("https://") || crate::media::resolver::needs_resolution(url) {
        return None;
    }
    let result = crate::media::hls::probe_source_cors(client, url).await?;
    let host = crate::media::hls::extract_manifest_host(url);
    cors_cache.write().await.insert(host, result);
    Some(result)
}
```

`check_source` keeps its existing `ok && ...` health guard and delegates the probe/cache to this helper:

```rust
if ok {
    probe_and_cache_cors(client, cors_cache, &src.url).await;
}
```

This is a behavior-preserving refactor with one beneficial addition: the `needs_resolution()` skip avoids a wasted HTML fetch for `youtube_live` sources (which today probe youtube.com, find no segment, and return `None` anyway).

### 2. Guide derivation (currently-playing item)

In `build_guide_data`, the per-channel loop already branches on `ch.channel_type()` and loads VOD `items` inside the `VodLoop` arm to build the schedule. Refactor so `items` are loaded once per VOD channel and reused for both the schedule **and** the budget URL.

Generalize the sources-only `derive_budget_status(channel_id, first_active_urls, cors_cache)` into a URL-based helper:

```rust
fn budget_for_url(url: Option<&str>, cors_cache: &HashMap<String, bool>) -> BudgetStatus {
    match url {
        Some(u) => status_for_url(u, cors_cache),
        None => BudgetStatus::Unknown,
    }
}
```

Budget URL selection per channel:

- **Live**: `first_active_urls.get(&ch.id)` (unchanged source of truth).
- **VOD**: with `items` and `ch.loop_anchor`, compute the playing item via `playlist_item::current_position(&items, now.timestamp(), anchor.timestamp())` → `items[idx].url`. Fallback to `items.first()` when `loop_anchor` is `None`; `None` when the playlist is empty.

`now` is the same `Utc::now()` already captured at the top of `build_guide_data`.

### 3. Per-item Test button + Budget column (admin)

- **Display type**: `AdminPlaylistItemRow` gains `budget_badge_class: &'static str` and `budget_badge_char: &'static str`, plus an `apply_budget(&mut self, cors_cache)` method mirroring `AdminSourceRow`. The `From<PlaylistItem>` impl defaults both to `Unknown`.
- **Detail page**: `channel_detail` maps playlist items with `apply_budget(&cors)` from the CORS snapshot, exactly as it already does for sources.
- **Template**: extract the inline playlist `<tr>` from `channel_detail.html` into `templates/admin/partials/playlist_item_row.html` with `id="pl-row-{{ item.id }}"`, mirroring `source_row.html`:
  - a **Budget** `<td>` rendering `·` (grey, "not yet probed") when `budget_badge_char.is_empty()`, else the `⚡`/`☁` glyph with its class;
  - a **Test** button: `hx-post="/admin/playlist/{{ item.id }}/test"`, `hx-target="#pl-row-{{ item.id }}"`, `hx-swap="outerHTML"`, `hx-disabled-elt="this"`.
  - Add a `Budget` column header to the playlist `<table>` in `channel_detail.html`.
- **Route + handler**: `POST /admin/playlist/:id/test` → `playlist_item_test` in `routes/admin/playlist.rs`:
  1. fetch item (404 if missing),
  2. `probe_and_cache_cors(&state.http_client, &state.cors_cache, &item.url)`,
  3. snapshot the cache, build an `AdminPlaylistItemRow`, `apply_budget`, render the row partial.
  Registered in `lib.rs` next to `/playlist/:id/delete`; exported via `routes/admin/mod.rs`.

### 4. Background checker sweeps VOD item hosts

- **Model**: add `playlist_item::list_all(pool) -> Result<Vec<PlaylistItem>>` (all items belong to vod_loop channels; live channels have none).
- **Health checker**: add a `probe_all_playlist_cors(pool, client, cors_cache)` step called from `check_all` after the sources loop. It iterates all items and calls `probe_and_cache_cors`, **deduping by host within the cycle** (a `HashSet<String>` of `extract_manifest_host` values) so each CDN is probed once. CORS-only — playlist items have no health/`is_active`/disable state.

## Data flow

```
Background (every 15 min):  check_all → sources loop (check_source → probe_and_cache_cors)
                                      → probe_all_playlist_cors (dedup by host → probe_and_cache_cors)
                                                              ↓ writes CorsCache (host → bool)
Admin Test button:          POST /admin/playlist/:id/test → probe_and_cache_cors → re-render row
Guide render:               build_guide_data → reads CorsCache snapshot
                                              → budget_for_url(current VOD item URL) → badge
```

## Error handling

- `probe_and_cache_cors` returns `None` on any network/timeout error or non-probeable URL; the cache is simply left unchanged (proxy-safe default; badge stays `·`/`Unknown`).
- `playlist_item_test`: 404 if the item id is unknown; the probe failing is not an error — the row still re-renders (badge `·`).
- Background sweep failures are logged and skipped per item, never aborting the cycle.

## Testing

- **Unit** (`budget.rs` or `guide.rs`): `budget_for_url` — `None → Unknown`, `Some(http) → Proxied`, `Some(https)` cache-hit `true → Direct` / `false → Proxied` / miss `→ Unknown`. Replaces `test_derive_budget_status_no_source_unknown`.
- **Unit**: `playlist_item::current_position` is already covered; no new selection tests needed.
- **Integration** (`tests/http.rs`): `POST /admin/playlist/:id/test` returns 200 and the swapped row HTML contains the Budget cell. The bounded test-client timeout means the probe to a seed URL fails cleanly, so the badge stays `·` — mirroring the existing source Test integration test. Use a VOD channel/item from `tests/fixtures/seed.sql` (channel 4 has two episodes).

## Out of scope

- Probing the *resolved* (yt-dlp) URL for YouTube/Twitch items — those stay `Unknown` by design.
- Persisting the CORS cache across restarts (still in-memory; the background sweep re-warms it).
- Aggregating budget across all items of a VOD channel (the guide badge uses the currently-playing item only).

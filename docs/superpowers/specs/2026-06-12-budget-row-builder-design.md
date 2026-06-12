# Spec — Fold the budget-badge dance behind a row builder

_Candidate #5 of the architecture-deepening effort (`docs/architecture/changes-20260612.html` §5).
Created 2026-06-12._

## Problem

Three admin handlers render budget-badged rows, and each repeats the same manual ritual: read
the CORS-cache snapshot, convert the model item into a display row, then call `apply_budget` to
fill the badge. The conversion itself (`From<Source>`/`From<PlaylistItem>`) deliberately produces
a **half-built** row — it sets the budget badge to a `BudgetStatus::Unknown` placeholder that the
caller is expected to overwrite afterward:

```rust
// src/routes/admin/mod.rs — inside From<Source>/From<PlaylistItem>
let (budget_badge_class, budget_badge_char) =
    crate::budget::budget_badge(crate::budget::BudgetStatus::Unknown); // placeholder
```

```rust
// repeated in every handler that renders a row
let cors = state.cors_cache.read().await.clone();
let mut row: AdminSourceRow = x.into();
row.apply_budget(&cors);
```

The `Unknown` placeholder is the residue of a forgettable step: call `.into()` and skip
`apply_budget`, and the badge silently renders blank. The badge-fill logic (`apply_budget`) is
itself **copy-pasted** verbatim across the two row types (`mod.rs:81` and `mod.rs:104`).

Verified facts:
- `From<Source> for AdminSourceRow` and `From<PlaylistItem> for AdminPlaylistItemRow` have **no
  callers outside the four build-sites** being refactored here (`channels.rs:246`, `channels.rs:255`,
  `sources.rs:104`, `playlist.rs:139`). They are entirely ours to reshape.
- `From<channel::Channel> for AdminChannelRow` carries no budget badge (channels have no URL) and
  is untouched.
- `cors_cache` is `CorsCache = Arc<RwLock<HashMap<String, bool>>>` (`src/lib.rs:28`).

## Solution

Make a budgeted row **un-constructable in a half-built state**: fold the cache read and badge
computation into the construction itself, behind one trait and two generic builders. The
`Unknown` placeholder disappears; the separate `apply_budget` step disappears; callers never read
the cache or fill a badge because the only construction path already does both.

### Module location

All of it lives in `src/routes/admin/mod.rs`, next to the row types it serves. No new file.

### The trait

```rust
pub(crate) trait BudgetRow<T>: Sized {
    /// Build a fully budget-badged display row from a raw model item and a CORS-cache snapshot.
    fn from_model(item: T, cors_cache: &HashMap<String, bool>) -> Self;
}
```

`pub(crate)` — these are crate-internal helpers. (A `pub` builder bounded on the trait would force
the trait public via the `private_bounds` lint under `-D warnings`; `pub(crate)` on both sides
avoids that and is the tightest correct visibility.)

### The two impls

Each impl is the **current `From` body moved verbatim**, with exactly one line changed: the
`Unknown` placeholder becomes the real cache lookup.

```rust
impl BudgetRow<source::Source> for AdminSourceRow {
    fn from_model(s: source::Source, cors_cache: &HashMap<String, bool>) -> Self {
        let (budget_badge_class, budget_badge_char) =
            crate::budget::badge_for_url(&s.url, cors_cache); // was budget_badge(Unknown)
        let status_lazy = s.is_active && s.kind == "youtube_live";
        let status = crate::status::compute(
            s.is_active,
            &s.kind,
            s.last_status.as_deref(),
            s.failure_reason.as_deref(),
            None,
        );
        let badge = crate::status::status_badge(&status);
        Self {
            id: s.id,
            kind: s.kind,
            url: s.url,
            priority: s.priority,
            is_active: s.is_active,
            failure_reason: s.failure_reason,
            budget_badge_class,
            budget_badge_char,
            status_color: badge.color,
            status_glyph: badge.glyph,
            status_title: badge.title,
            status_lazy,
        }
    }
}

impl BudgetRow<playlist_item::PlaylistItem> for AdminPlaylistItemRow {
    fn from_model(i: playlist_item::PlaylistItem, cors_cache: &HashMap<String, bool>) -> Self {
        let (budget_badge_class, budget_badge_char) =
            crate::budget::badge_for_url(&i.url, cors_cache); // was budget_badge(Unknown)
        let status = crate::status::compute(
            i.is_active,
            "hls", // playlist items use health only — never the youtube_live live branch
            i.last_status.as_deref(),
            i.failure_reason.as_deref(),
            None,
        );
        let badge = crate::status::status_badge(&status);
        Self {
            id: i.id,
            title: i.title,
            url: i.url,
            duration_secs: i.duration_secs,
            sort_order: i.sort_order,
            budget_badge_class,
            budget_badge_char,
            is_active: i.is_active,
            failure_reason: i.failure_reason,
            status_color: badge.color,
            status_glyph: badge.glyph,
            status_title: badge.title,
        }
    }
}
```

### The two builders

```rust
pub(crate) async fn build_rows<R, T, I>(items: I, cors_cache: &CorsCache) -> Vec<R>
where
    I: IntoIterator<Item = T>,
    R: BudgetRow<T>,
{
    let cors = cors_cache.read().await.clone();
    items.into_iter().map(|it| R::from_model(it, &cors)).collect()
}

pub(crate) async fn build_row<R, T>(item: T, cors_cache: &CorsCache) -> R
where
    R: BudgetRow<T>,
{
    let cors = cors_cache.read().await.clone();
    R::from_model(item, &cors)
}
```

### Deletions

- `From<source::Source> for AdminSourceRow` (body moves into `from_model`).
- `From<playlist_item::PlaylistItem> for AdminPlaylistItemRow` (body moves into `from_model`).
- The inherent `AdminSourceRow::apply_budget` (`mod.rs:102–109`) and
  `AdminPlaylistItemRow::apply_budget` (`mod.rs:79–86`) methods — both impl blocks contain only
  `apply_budget`, so they are removed entirely.
- `From<channel::Channel> for AdminChannelRow` **stays** (no budget badge).

### Imports

`mod.rs` gains `use std::collections::HashMap;` and `CorsCache` (extend the existing
`use crate::{...}` to include `CorsCache`).

### Caller changes

`src/routes/admin/channels.rs` (`channel_detail`, currently `:242–259`):

```rust
let sources: Vec<AdminSourceRow> = build_rows(srcs, &state.cors_cache).await;
let playlist_items: Vec<AdminPlaylistItemRow> = build_rows(items, &state.cors_cache).await;
```

The standalone `let cors = state.cors_cache.read().await.clone();` line and both `.map(|x| { ... apply_budget ... })`
closures are removed. Type ascription on the bindings drives inference — no turbofish.

`src/routes/admin/sources.rs` (`source_test`, currently `:103–105`):

```rust
let row: AdminSourceRow = build_row(updated, &state.cors_cache).await;
```

`src/routes/admin/playlist.rs` (`playlist_item_test`, currently `:138–140`):

```rust
let row: AdminPlaylistItemRow = build_row(updated, &state.cors_cache).await;
```

Each handler imports `build_rows`/`build_row` (and drops the now-unused `apply_budget` path — the
row-type imports stay). The `build_*` names resolve via `super::` / the existing
`crate::routes::admin` path already in use.

## Behavior preservation (byte-identical)

- The rendered badge for every row is **identical**. Before: `From` set `Unknown`, then
  `apply_budget` overwrote it with `badge_for_url(&url, cache)`. After: `from_model` computes
  `badge_for_url(&url, cache)` directly. Same final value, including the legitimate blank/`Unknown`
  outcome on an https cache-miss.
- `status_color` / `status_glyph` / `status_title` / `status_lazy` are computed by the same
  `status::compute` + `status_badge` calls, moved verbatim.
- **One change, unobservable:** `channel_detail` previously took a single cache snapshot for both
  row collections; it now takes two (one per `build_rows` call), microseconds apart on a host→bool
  map maintained by the background health checker. No rendered difference.

## Out of scope

- Guide-side budget badges (`src/routes/guide/badges.rs`, `guide/data.rs`) — a separate code path,
  not part of the admin row build.
- `From<channel::Channel> for AdminChannelRow`, status logic (`crate::status`), the `budget`
  module, and the `model::*` layer — all unchanged.
- No migration; no template changes (row field shapes are identical).

## Testing — the win

New unit tests in `mod.rs`'s `#[cfg(test)]` module (pure, no network, no DB):

1. **`from_model` source — cache hit true → ⚡:** construct a `source::Source` with an `https://`
   URL, a `CorsCache`-style `HashMap` mapping its manifest host to `true`; assert the resulting
   `AdminSourceRow.budget_badge_class == "budget-direct"` and `budget_badge_char == "⚡"`.
2. **`from_model` source — cache hit false → ☁:** host mapped to `false`; assert `"budget-proxied"`
   / `"☁"`.
3. **`from_model` source — cache miss → blank:** empty map; assert `"budget-unknown"` / `""`.
4. **`from_model` playlist item — cache hit true → ⚡:** parallel test for `AdminPlaylistItemRow`.
5. **`build_rows` fills every row:** build a `CorsCache` (`Arc::new(RwLock::new(map))`), pass a
   `Vec<source::Source>` of two items (one host known-true, one unknown); assert
   `build_rows::<AdminSourceRow, _, _>(...).await` returns two rows with the expected badges.
6. **`build_row` single:** assert `build_row::<AdminSourceRow, _>(src, &cache).await` fills the badge.

The `From`-based tests (none currently exist for these two impls) are not migrated because there
are none. Existing `mod.rs` auth tests are untouched.

Integration tests in `tests/http.rs` that render the channel-detail page and the source/playlist
row fragments stay green **without modification** — they are the behavior contract for the rendered
output.

## Acceptance criteria

1. `BudgetRow<T>` trait with `from_model` exists in `mod.rs`, implemented for `AdminSourceRow`
   (over `source::Source`) and `AdminPlaylistItemRow` (over `playlist_item::PlaylistItem`).
2. `build_rows` and `build_row` exist, fold the cache read inside, and are the only construction
   path used by the three handlers.
3. `From<source::Source>`, `From<playlist_item::PlaylistItem>`, and both inherent `apply_budget`
   methods are deleted; `From<channel::Channel>` remains.
4. The three handlers no longer read `cors_cache` directly or call `apply_budget`; rendered badges
   are byte-identical to today.
5. New `from_model`/`build_rows`/`build_row` unit tests pass; all `tests/http.rs` tests stay green
   unmodified.
6. `cargo test` (incl. `--no-run` for lib `#[cfg(test)]`), `cargo fmt --check`,
   `cargo clippy --all-targets -- -D warnings` all green.

# Budget-Row Builder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fold the CORS-cache read and budget-badge fill into row construction so admin handlers can no longer forget the step, and the row never exists half-built.

**Architecture:** A `pub(crate)` trait `BudgetRow<T>` with one method `from_model(item, cors_cache)` builds a fully budget-badged display row in a single pass (replacing the `From` impls that emitted an `Unknown` placeholder plus a separate `apply_budget` call). Two generic async builders — `build_rows` (batch) and `build_row` (single) — read the cache snapshot internally and map `from_model` over the items. The three admin handlers call the builders; the old `From` impls and `apply_budget` methods are deleted.

**Tech Stack:** Rust 1.96, Axum 0.7, tokio (`RwLock`), Askama. All changes in `src/routes/admin/`.

**Spec:** `docs/superpowers/specs/2026-06-12-budget-row-builder-design.md`

**Conventions (must honor):** run `cargo fmt` before every commit (CI fails on any diff); commit messages end with the `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer; no comments unless the WHY is non-obvious.

---

## Background for the implementer

`src/routes/admin/mod.rs` defines three display row structs and their conversions:

- `AdminChannelRow` — `From<channel::Channel>` (no budget badge — **leave untouched**).
- `AdminSourceRow` — `From<source::Source>` at `mod.rs:111–141`, plus an inherent `apply_budget` at `mod.rs:102–109`.
- `AdminPlaylistItemRow` — `From<playlist_item::PlaylistItem>` at `mod.rs:143–170`, plus an inherent `apply_budget` at `mod.rs:79–86`.

Today each `From` sets the budget badge to a placeholder (`crate::budget::budget_badge(crate::budget::BudgetStatus::Unknown)`), and callers must call `apply_budget(&cors)` afterward to overwrite it with the real value via `crate::budget::badge_for_url(&url, &cors)`. We are collapsing both steps into one.

Key types and helpers (already exist — do not change them):
- `crate::budget::badge_for_url(url: &str, cors_cache: &HashMap<String, bool>) -> (&'static str, &'static str)` — returns `(class, glyph)`: `("budget-direct","⚡")` / `("budget-proxied","☁")` / `("budget-unknown","")`.
- `crate::media::hls::extract_manifest_host(url: &str) -> String` — the key used to look up a URL in the CORS cache. **Use this in tests** to derive the cache key from a URL.
- `crate::CorsCache = Arc<RwLock<HashMap<String, bool>>>` (`src/lib.rs:28`; `RwLock` is `tokio::sync::RwLock`).
- `crate::status::compute(...)` + `crate::status::status_badge(&status)` — status fields (moved verbatim into `from_model`).

Model struct fields (for constructing test fixtures):
```rust
// crate::model::source::Source
{ id: i64, channel_id: i64, kind: String, url: String, priority: i64,
  is_active: bool, last_checked_at: Option<i64>, last_status: Option<String>,
  consecutive_failures: i64, failure_reason: Option<String> }

// crate::model::playlist_item::PlaylistItem
{ id: i64, channel_id: i64, title: String, url: String, duration_secs: i64,
  sort_order: i64, is_active: bool, last_checked_at: Option<i64>,
  last_status: Option<String>, consecutive_failures: i64, failure_reason: Option<String> }
```

Both model structs and all row-struct fields are `pub`, so tests can build them with literals.

---

## Task 1: Introduce `BudgetRow` + builders and rewire the three handlers

**Files:**
- Modify: `src/routes/admin/mod.rs` (add imports, trait, two `from_model` impls, two builders, unit tests — `From` impls + `apply_budget` stay for now, unused)
- Modify: `src/routes/admin/channels.rs:242-259`
- Modify: `src/routes/admin/sources.rs:103-105`
- Modify: `src/routes/admin/playlist.rs:138-140`

**Why this shape:** Adding the new path *and* rewiring callers in one commit keeps the tree green with no warnings. `build_rows`/`build_row` are `pub(crate)`, so if they were added but left uncalled, the `dead_code` lint would fail CI under `-D warnings`. Rewiring the callers in the same task uses them immediately. The now-unused `From` impls (trait impls — never `dead_code`-linted) and `apply_budget` methods (`pub` — never `dead_code`-linted) stay until Task 2, so this commit has zero warnings.

- [ ] **Step 1: Write the failing unit tests**

In `src/routes/admin/mod.rs`, add these tests inside the existing `#[cfg(test)] mod tests { ... }` block (after the auth tests, before the closing `}`):

```rust
    use crate::media::hls::extract_manifest_host;
    use crate::model::{playlist_item::PlaylistItem, source::Source};
    use std::collections::HashMap;

    fn sample_source(url: &str) -> Source {
        Source {
            id: 1,
            channel_id: 1,
            kind: "hls".to_string(),
            url: url.to_string(),
            priority: 0,
            is_active: true,
            last_checked_at: None,
            last_status: None,
            consecutive_failures: 0,
            failure_reason: None,
        }
    }

    fn sample_item(url: &str) -> PlaylistItem {
        PlaylistItem {
            id: 1,
            channel_id: 1,
            title: "ep".to_string(),
            url: url.to_string(),
            duration_secs: 60,
            sort_order: 0,
            is_active: true,
            last_checked_at: None,
            last_status: None,
            consecutive_failures: 0,
            failure_reason: None,
        }
    }

    #[test]
    fn from_model_source_cache_hit_true_is_direct() {
        let url = "https://cdn.example.com/live/stream.m3u8";
        let mut cache = HashMap::new();
        cache.insert(extract_manifest_host(url), true);
        let row = AdminSourceRow::from_model(sample_source(url), &cache);
        assert_eq!(row.budget_badge_class, "budget-direct");
        assert_eq!(row.budget_badge_char, "⚡");
    }

    #[test]
    fn from_model_source_cache_hit_false_is_proxied() {
        let url = "https://cdn.example.com/live/stream.m3u8";
        let mut cache = HashMap::new();
        cache.insert(extract_manifest_host(url), false);
        let row = AdminSourceRow::from_model(sample_source(url), &cache);
        assert_eq!(row.budget_badge_class, "budget-proxied");
        assert_eq!(row.budget_badge_char, "☁");
    }

    #[test]
    fn from_model_source_cache_miss_is_unknown() {
        let url = "https://cdn.example.com/live/stream.m3u8";
        let row = AdminSourceRow::from_model(sample_source(url), &HashMap::new());
        assert_eq!(row.budget_badge_class, "budget-unknown");
        assert_eq!(row.budget_badge_char, "");
    }

    #[test]
    fn from_model_playlist_item_cache_hit_true_is_direct() {
        let url = "https://cdn.example.com/vod/ep1.m3u8";
        let mut cache = HashMap::new();
        cache.insert(extract_manifest_host(url), true);
        let row = AdminPlaylistItemRow::from_model(sample_item(url), &cache);
        assert_eq!(row.budget_badge_class, "budget-direct");
        assert_eq!(row.budget_badge_char, "⚡");
    }

    #[tokio::test]
    async fn build_rows_fills_every_row() {
        let known = "https://known.example.com/a.m3u8";
        let unknown = "https://unknown.example.com/b.m3u8";
        let mut map = HashMap::new();
        map.insert(extract_manifest_host(known), true);
        let cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(map));

        let rows: Vec<AdminSourceRow> =
            build_rows(vec![sample_source(known), sample_source(unknown)], &cache).await;

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].budget_badge_class, "budget-direct");
        assert_eq!(rows[0].budget_badge_char, "⚡");
        assert_eq!(rows[1].budget_badge_class, "budget-unknown");
        assert_eq!(rows[1].budget_badge_char, "");
    }

    #[tokio::test]
    async fn build_row_fills_single_row() {
        let url = "https://cdn.example.com/live/stream.m3u8";
        let mut map = HashMap::new();
        map.insert(extract_manifest_host(url), false);
        let cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(map));

        let row: AdminSourceRow = build_row(sample_source(url), &cache).await;
        assert_eq!(row.budget_badge_class, "budget-proxied");
        assert_eq!(row.budget_badge_char, "☁");
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test --lib routes::admin::tests 2>&1 | head -30`
Expected: compile error — `no function or associated item named 'from_model'` / `cannot find function 'build_rows'`. (Compile failure counts as the failing-test state in Rust.)

- [ ] **Step 3: Add imports to `mod.rs`**

In `src/routes/admin/mod.rs`, extend the existing `use crate::{...}` block (currently `mod.rs:32-35`) to add `CorsCache`, and add a `HashMap` import. The block becomes:

```rust
use std::collections::HashMap;

use crate::{
    model::{channel, playlist_item, source},
    AppState, CorsCache,
};
```

(Place the `use std::collections::HashMap;` with the other top-of-file `use` statements; rustfmt will order it.)

- [ ] **Step 4: Add the `BudgetRow` trait**

In `src/routes/admin/mod.rs`, in the `// ── display types ──` / `// ── From impls ──` region (a natural spot is just above the `// ── From impls ──` comment), add:

```rust
/// Admin display rows that derive a network-budget badge from their URL.
/// `from_model` is the *only* construction path, so a row can never exist
/// without its badge filled — there is no half-built state to forget.
pub(crate) trait BudgetRow<T>: Sized {
    fn from_model(item: T, cors_cache: &HashMap<String, bool>) -> Self;
}
```

- [ ] **Step 5: Add the two `from_model` impls**

In `src/routes/admin/mod.rs`, add (next to the existing `From` impls). Each body is the current `From` body with the placeholder line replaced by the real lookup:

```rust
impl BudgetRow<source::Source> for AdminSourceRow {
    fn from_model(s: source::Source, cors_cache: &HashMap<String, bool>) -> Self {
        let (budget_badge_class, budget_badge_char) =
            crate::budget::badge_for_url(&s.url, cors_cache);
        let status_lazy = s.is_active && s.kind == "youtube_live";
        // Inline status for non-lazy rows (disabled, or non-youtube). Lazy rows
        // ignore these fields and fetch the badge via HTMX.
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
            crate::budget::badge_for_url(&i.url, cors_cache);
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

- [ ] **Step 6: Add the two builders**

In `src/routes/admin/mod.rs`, after the `from_model` impls, add:

```rust
/// Reads the CORS-cache snapshot once and builds a budget-badged row per item.
/// Callers never touch the cache or fill a badge — `from_model` does both.
pub(crate) async fn build_rows<R, T, I>(items: I, cors_cache: &CorsCache) -> Vec<R>
where
    I: IntoIterator<Item = T>,
    R: BudgetRow<T>,
{
    let cors = cors_cache.read().await.clone();
    items.into_iter().map(|it| R::from_model(it, &cors)).collect()
}

/// Single-row variant of [`build_rows`].
pub(crate) async fn build_row<R, T>(item: T, cors_cache: &CorsCache) -> R
where
    R: BudgetRow<T>,
{
    let cors = cors_cache.read().await.clone();
    R::from_model(item, &cors)
}
```

- [ ] **Step 7: Run the unit tests to verify they pass**

Run: `cargo test --lib routes::admin::tests 2>&1 | tail -20`
Expected: all `from_model_*`, `build_rows_fills_every_row`, `build_row_fills_single_row` tests PASS. (The old `apply_budget` methods and `From` impls still exist — that's fine, they're now unused.)

- [ ] **Step 8: Rewire `channels.rs`**

In `src/routes/admin/channels.rs`, replace the cache-read + two `.map(...)` blocks (currently `:242-259`) — which look like:

```rust
    let cors = state.cors_cache.read().await.clone();
    let sources: Vec<AdminSourceRow> = srcs
        .into_iter()
        .map(|s| {
            let mut row: AdminSourceRow = s.into();
            row.apply_budget(&cors);
            row
        })
        .collect();

    let playlist_items: Vec<AdminPlaylistItemRow> = items
        .into_iter()
        .map(|i| {
            let mut row: AdminPlaylistItemRow = i.into();
            row.apply_budget(&cors);
            row
        })
        .collect();
```

with:

```rust
    let sources: Vec<AdminSourceRow> =
        super::build_rows(srcs, &state.cors_cache).await;
    let playlist_items: Vec<AdminPlaylistItemRow> =
        super::build_rows(items, &state.cors_cache).await;
```

(`channels.rs` already imports `AdminSourceRow`/`AdminPlaylistItemRow` via `use super::{...}` at `:10`. The `build_rows` call is reached through `super::`.)

- [ ] **Step 9: Rewire `sources.rs`**

In `src/routes/admin/sources.rs`, replace (currently `:103-105`):

```rust
    let cors = state.cors_cache.read().await.clone();
    let mut row: AdminSourceRow = updated.into();
    row.apply_budget(&cors);
```

with:

```rust
    let row: AdminSourceRow = crate::routes::admin::build_row(updated, &state.cors_cache).await;
```

(`sources.rs` imports `AdminSourceRow` via `use crate::routes::admin::AdminSourceRow;` at `:9`; reach `build_row` through the same path.)

- [ ] **Step 10: Rewire `playlist.rs`**

In `src/routes/admin/playlist.rs`, replace (currently `:138-140`):

```rust
    let cors = state.cors_cache.read().await.clone();
    let mut row: AdminPlaylistItemRow = updated.into();
    row.apply_budget(&cors);
```

with:

```rust
    let row: AdminPlaylistItemRow = crate::routes::admin::build_row(updated, &state.cors_cache).await;
```

(`playlist.rs` imports `AdminPlaylistItemRow` via `use crate::routes::admin::AdminPlaylistItemRow;` at `:9`.)

- [ ] **Step 11: Format, lint, and run the full suite**

Run:
```bash
cargo fmt
cargo clippy --all-targets -- -D warnings 2>&1 | tail -20
cargo test 2>&1 | tail -30
```
Expected: `fmt` clean (no diff), clippy clean (no warnings — the unused `From` impls are trait impls and `apply_budget` is `pub`, so neither triggers `dead_code`), all tests pass (the new unit tests plus every existing `tests/http.rs` admin test, unmodified).

- [ ] **Step 12: Commit**

```bash
git add src/routes/admin/mod.rs src/routes/admin/channels.rs src/routes/admin/sources.rs src/routes/admin/playlist.rs
git commit -m "$(cat <<'EOF'
feat(admin): build budget-badged rows via BudgetRow::from_model

Add a pub(crate) BudgetRow<T>::from_model trait that builds a fully
budget-badged display row in one pass, plus build_rows/build_row helpers that
read the CORS-cache snapshot internally. Rewire channel-detail, source_test,
and playlist_item_test to the builders so no handler reads the cache or fills a
badge by hand. The old From impls + apply_budget remain (unused) and are
removed in the next commit.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Delete the dead `From` impls and `apply_budget` methods

**Files:**
- Modify: `src/routes/admin/mod.rs` (delete `From<source::Source>`, `From<playlist_item::PlaylistItem>`, and both inherent `apply_budget` impl blocks)

After Task 1, these four items have no callers (the handlers use `from_model`/`build_*`, and the `from_model` impls do not call them). Removing them is the deepening's payoff: the `Unknown` placeholder and the forgettable two-step are gone.

- [ ] **Step 1: Delete `AdminPlaylistItemRow::apply_budget`**

In `src/routes/admin/mod.rs`, remove this impl block (currently `:79-86`):

```rust
impl AdminPlaylistItemRow {
    /// Fills the budget badge fields from a CORS-cache snapshot, keyed by this item's URL host.
    pub fn apply_budget(&mut self, cors_cache: &std::collections::HashMap<String, bool>) {
        let (class, glyph) = crate::budget::badge_for_url(&self.url, cors_cache);
        self.budget_badge_class = class;
        self.budget_badge_char = glyph;
    }
}
```

- [ ] **Step 2: Delete `AdminSourceRow::apply_budget`**

In `src/routes/admin/mod.rs`, remove this impl block (currently `:102-109`):

```rust
impl AdminSourceRow {
    /// Fills the budget badge fields from a CORS-cache snapshot, keyed by this source's URL host.
    pub fn apply_budget(&mut self, cors_cache: &std::collections::HashMap<String, bool>) {
        let (class, glyph) = crate::budget::badge_for_url(&self.url, cors_cache);
        self.budget_badge_class = class;
        self.budget_badge_char = glyph;
    }
}
```

- [ ] **Step 3: Delete `From<source::Source> for AdminSourceRow`**

In `src/routes/admin/mod.rs`, remove the entire `impl From<source::Source> for AdminSourceRow { ... }` block (currently `:111-141`).

- [ ] **Step 4: Delete `From<playlist_item::PlaylistItem> for AdminPlaylistItemRow`**

In `src/routes/admin/mod.rs`, remove the entire `impl From<playlist_item::PlaylistItem> for AdminPlaylistItemRow { ... }` block (currently `:143-170`).

Leave `impl From<channel::Channel> for AdminChannelRow { ... }` in place — it has no budget badge and is still used by the channel list/detail handlers.

- [ ] **Step 5: Format, lint, and run the full suite**

Run:
```bash
cargo fmt
cargo clippy --all-targets -- -D warnings 2>&1 | tail -20
cargo test 2>&1 | tail -30
```
Expected: `fmt` clean, clippy clean, all tests pass. If clippy reports an unused import (e.g. if `playlist_item`/`source` are now only referenced by `from_model` — they still are, so no removal expected), remove only what it names. The `crate::budget::BudgetStatus`/`budget_badge` path is still used elsewhere (`From<Source>` used `budget_badge(Unknown)` — now gone, but `budget_badge`/`BudgetStatus` remain referenced via `badge_for_url` only indirectly; do **not** pre-emptively touch `src/budget.rs`).

- [ ] **Step 6: Commit**

```bash
git add src/routes/admin/mod.rs
git commit -m "$(cat <<'EOF'
refactor(admin): drop the half-built From impls + apply_budget

The From<Source>/From<PlaylistItem> impls (which emitted an Unknown budget-badge
placeholder) and the duplicated apply_budget methods now have no callers —
from_model is the sole construction path. Removing them eliminates the
forgettable two-step and the half-built-row state entirely.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final verification (after both tasks)

Run the full gate one more time on the finished branch:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all green; test count up by 6 (the new `from_model`/`build_rows`/`build_row` unit tests); no `tests/http.rs` file modified.

Confirm the acceptance criteria from the spec:
1. `BudgetRow<T>::from_model` implemented for both row types. ✓ (Task 1)
2. `build_rows`/`build_row` fold the cache read in and are the only construction path used by handlers. ✓ (Task 1)
3. `From<Source>`, `From<PlaylistItem>`, both `apply_budget` deleted; `From<Channel>` remains. ✓ (Task 2)
4. Handlers no longer read `cors_cache` or call `apply_budget`; badges byte-identical. ✓
5. New unit tests pass; `tests/http.rs` green unmodified. ✓
6. `cargo test` / `fmt --check` / `clippy -D warnings` green. ✓
```

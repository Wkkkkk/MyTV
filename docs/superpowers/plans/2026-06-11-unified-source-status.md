# Unified source Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the four overlapping live-source indicators (Active, Health, Budget, Live) with two — a single unified **Status** badge plus the existing **Budget** badge — and decouple `is_active` (manual intent) from the health checker.

**Architecture:** A new crate-level `status.rs` defines a `SourceStatus` enum, a pure `compute()` from a source/item's fields + cached live status, a `status_badge()` renderer, and a `most_optimistic()` aggregator. The health checker stops mutating `is_active`; the tune path skips observed-Down regular sources at read time via a new `list_tunable_for_channel` query while keeping `youtube_live` in rotation (for idea #38). Admin rows and the guide render the unified Status.

**Tech Stack:** Rust, Axum 0.7, SQLx 0.7 (SQLite), Askama 0.12, HTMX. Tests via `cargo test` (unit + `tests/http.rs` integration with `tower::ServiceExt::oneshot`).

**Spec:** `docs/superpowers/specs/2026-06-11-unified-source-status-design.md`

**Refinement vs spec (intentional):** The *display* **Down** state is `last_status == "error"` (any error), matching today's red-dot UI — so a transiently-errored source shows ✕ immediately (the user accepted "mid-recovery shows ✕ until the next successful probe"). The `consecutive_failures >= 3` threshold is used **only** by the tune-skip query (`list_tunable_for_channel`), never by the badge.

---

## File Structure

- **Create** `src/status.rs` — `SourceStatus` enum, `StatusBadge` struct, `compute()`, `status_badge()`, `rank()`, `most_optimistic()`. One responsibility: the unified status model + rendering.
- **Modify** `src/lib.rs` — register `pub mod status;`.
- **Modify** `src/health.rs` — remove `is_active` mutation (drop `HealthAction`, simplify `process_result` → `process_failures`, drop `manage_lifecycle`/`is_active` params from `run_check`, update callers + `record_source_liveness`); rewrite affected tests.
- **Modify** `src/model/source.rs` — add `list_tunable_for_channel` query + tests.
- **Modify** `src/routes/player.rs` — `next_live` uses `list_tunable_for_channel`.
- **Modify** `src/routes/admin/mod.rs` — add status fields to `AdminSourceRow` + `AdminPlaylistItemRow`, compute them in `From` impls.
- **Modify** `src/routes/admin/live_status.rs` — `/admin/live-status` returns the unified Status badge.
- **Modify** `templates/admin/partials/source_row.html`, `templates/admin/partials/playlist_item_row.html`, `templates/admin/channel_detail.html` — collapse Active/Health/Live columns to one Status column.
- **Modify** `src/routes/guide/data.rs`, `src/routes/guide/badges.rs`, `src/routes/guide/mod.rs`, `templates/partials/epg_content.html` — channel Status from most-optimistic per-source status; pass a live-status snapshot.
- **Create** `docs/architecture/source-status.md` — architecture note.

---

### Task 1: `status.rs` — the unified status model

**Files:**
- Create: `src/status.rs`
- Modify: `src/lib.rs` (add `pub mod status;`)
- Test: inline `#[cfg(test)]` in `src/status.rs`

- [ ] **Step 1: Register the module**

In `src/lib.rs`, add alongside the other `pub mod` declarations (e.g. near `pub mod budget;`):

```rust
pub mod status;
```

Run: `cargo build 2>&1 | tail -5`
Expected: FAIL — `file not found for module status` (file created next step).

- [ ] **Step 2: Write `src/status.rs` with the enum, badge, compute, and aggregator**

```rust
use crate::media::resolver::LiveStatus;

/// The single unified status of a source or playlist item. Replaces the separate
/// Active / Health / Live indicators. `Down` carries the failure reason; `Upcoming`
/// carries the scheduled-start unix timestamp when known.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceStatus {
    Disabled,
    Down(Option<String>),
    Live,
    Ok,
    Upcoming(Option<i64>),
    Recorded,
    Offline,
    Unchecked,
}

/// Rendering parts for a status. `class` + `color` cover both surfaces: the guide
/// uses an inline `color`, the admin rows use `color` for the inline glyph, and
/// `class` is available for future CSS. `title` is the hover tooltip.
pub struct StatusBadge {
    pub class: &'static str,
    pub color: &'static str,
    pub glyph: &'static str,
    pub label: &'static str,
    pub title: String,
}

/// Computes the unified status. `live` is the cached `LiveStatus` for a
/// `youtube_live` source (pass `None` for other kinds, or when the live cache is
/// cold). Precedence: Disabled (manual intent) first; then for youtube_live the
/// live status; otherwise the persisted health (`last_status`).
///
/// Display `Down` = any `last_status == "error"`; the auto-disable threshold is
/// NOT consulted here (it governs only the tune-skip query).
pub fn compute(
    is_active: bool,
    kind: &str,
    last_status: Option<&str>,
    failure_reason: Option<&str>,
    live: Option<LiveStatus>,
) -> SourceStatus {
    if !is_active {
        return SourceStatus::Disabled;
    }
    if kind == "youtube_live" {
        return match live {
            Some(LiveStatus::Live) => SourceStatus::Live,
            Some(LiveStatus::Upcoming(ts)) => SourceStatus::Upcoming(ts),
            Some(LiveStatus::WasLive) | Some(LiveStatus::PostLive) => SourceStatus::Recorded,
            Some(LiveStatus::Offline) | Some(LiveStatus::NotLive) => SourceStatus::Offline,
            Some(LiveStatus::Unknown) | None => SourceStatus::Unchecked,
        };
    }
    match last_status {
        Some("error") => SourceStatus::Down(failure_reason.map(|s| s.to_string())),
        Some("ok") => SourceStatus::Ok,
        _ => SourceStatus::Unchecked,
    }
}

/// Maps a status to its renderable parts.
pub fn status_badge(s: &SourceStatus) -> StatusBadge {
    match s {
        SourceStatus::Disabled => StatusBadge {
            class: "status-disabled",
            color: "#888",
            glyph: "⏸",
            label: "disabled",
            title: "Manually disabled".to_string(),
        },
        SourceStatus::Down(reason) => StatusBadge {
            class: "status-down",
            color: "#e94560",
            glyph: "✕",
            label: "down",
            title: match reason {
                Some(r) => format!("Down — {r}"),
                None => "Last check failed".to_string(),
            },
        },
        SourceStatus::Live => StatusBadge {
            class: "status-live",
            color: "#4caf50",
            glyph: "●",
            label: "live",
            title: "Currently live".to_string(),
        },
        SourceStatus::Ok => StatusBadge {
            class: "status-ok",
            color: "#4caf50",
            glyph: "●",
            label: "ok",
            title: "Reachable".to_string(),
        },
        SourceStatus::Upcoming(ts) => {
            let ts = *ts; // Option<i64> is Copy — copy out of the &SourceStatus borrow
            let title = ts
                .filter(|t| *t > 0)
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                .map(|dt| format!("Scheduled — starts {}", crate::media::format_utc_short(dt)))
                .unwrap_or_else(|| "Scheduled, start time unknown".to_string());
            StatusBadge {
                class: "status-upcoming",
                color: "#db4",
                glyph: "◷",
                label: "upcoming",
                title,
            }
        }
        SourceStatus::Recorded => StatusBadge {
            class: "status-recorded",
            color: "#88f",
            glyph: "⏺",
            label: "recorded",
            title: "Finished broadcast — next tune converts the channel to VOD".to_string(),
        },
        SourceStatus::Offline => StatusBadge {
            class: "status-offline",
            color: "#888",
            glyph: "○",
            label: "offline",
            title: "Not currently live".to_string(),
        },
        SourceStatus::Unchecked => StatusBadge {
            class: "status-unchecked",
            color: "#666",
            glyph: "·",
            label: "?",
            title: "Not yet checked".to_string(),
        },
    }
}

/// Optimism rank, lower = better (more optimistic). Used to aggregate a channel's
/// per-source statuses into one guide badge.
pub fn rank(s: &SourceStatus) -> u8 {
    match s {
        SourceStatus::Live | SourceStatus::Ok => 0,
        SourceStatus::Upcoming(_) => 1,
        SourceStatus::Recorded => 2,
        SourceStatus::Offline => 3,
        SourceStatus::Unchecked => 4,
        SourceStatus::Down(_) => 5,
        SourceStatus::Disabled => 6,
    }
}

/// The most-optimistic (best-case) status across an iterator. Empty → Unchecked.
pub fn most_optimistic<I: IntoIterator<Item = SourceStatus>>(statuses: I) -> SourceStatus {
    statuses
        .into_iter()
        .min_by_key(rank)
        .unwrap_or(SourceStatus::Unchecked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_wins_regardless_of_health_or_live() {
        assert_eq!(
            compute(false, "hls", Some("ok"), None, None),
            SourceStatus::Disabled
        );
        assert_eq!(
            compute(false, "youtube_live", None, None, Some(LiveStatus::Live)),
            SourceStatus::Disabled
        );
    }

    #[test]
    fn regular_source_maps_health() {
        assert_eq!(compute(true, "hls", Some("ok"), None, None), SourceStatus::Ok);
        assert_eq!(
            compute(true, "hls", Some("error"), Some("timeout"), None),
            SourceStatus::Down(Some("timeout".to_string()))
        );
        assert_eq!(compute(true, "hls", None, None, None), SourceStatus::Unchecked);
    }

    #[test]
    fn youtube_live_maps_live_status_not_health() {
        // Offline recorded as last_status='error' must still show Offline, never Down.
        assert_eq!(
            compute(true, "youtube_live", Some("error"), Some("not currently live"), Some(LiveStatus::Offline)),
            SourceStatus::Offline
        );
        assert_eq!(
            compute(true, "youtube_live", None, None, Some(LiveStatus::Live)),
            SourceStatus::Live
        );
        assert_eq!(
            compute(true, "youtube_live", None, None, Some(LiveStatus::Upcoming(Some(123)))),
            SourceStatus::Upcoming(Some(123))
        );
        assert_eq!(
            compute(true, "youtube_live", None, None, Some(LiveStatus::WasLive)),
            SourceStatus::Recorded
        );
        assert_eq!(
            compute(true, "youtube_live", None, None, Some(LiveStatus::PostLive)),
            SourceStatus::Recorded
        );
        // Cold cache → Unchecked, never Down.
        assert_eq!(
            compute(true, "youtube_live", Some("error"), None, None),
            SourceStatus::Unchecked
        );
    }

    #[test]
    fn badge_glyphs_and_colors() {
        assert_eq!(status_badge(&SourceStatus::Live).glyph, "●");
        assert_eq!(status_badge(&SourceStatus::Live).color, "#4caf50");
        assert_eq!(status_badge(&SourceStatus::Disabled).glyph, "⏸");
        assert_eq!(status_badge(&SourceStatus::Down(None)).glyph, "✕");
        assert_eq!(
            status_badge(&SourceStatus::Down(Some("boom".into()))).title,
            "Down — boom"
        );
        assert_eq!(status_badge(&SourceStatus::Offline).glyph, "○");
        assert_eq!(status_badge(&SourceStatus::Upcoming(None)).title, "Scheduled, start time unknown");
    }

    #[test]
    fn most_optimistic_picks_best_case() {
        assert_eq!(
            most_optimistic([SourceStatus::Down(None), SourceStatus::Live, SourceStatus::Disabled]),
            SourceStatus::Live
        );
        assert_eq!(
            most_optimistic([SourceStatus::Down(None), SourceStatus::Disabled]),
            SourceStatus::Down(None)
        );
        assert_eq!(most_optimistic([SourceStatus::Disabled]), SourceStatus::Disabled);
        assert_eq!(most_optimistic(std::iter::empty()), SourceStatus::Unchecked);
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test --lib status:: 2>&1 | tail -15`
Expected: PASS — all `status::tests::*` green.

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add src/status.rs src/lib.rs
git commit -m "feat(status): SourceStatus model, badge, and most-optimistic aggregator"
```

---

### Task 2: Decouple the health checker from `is_active`

**Files:**
- Modify: `src/health.rs`
- Test: inline `#[cfg(test)]` in `src/health.rs`

- [ ] **Step 1: Replace `process_result` with `process_failures` and remove `HealthAction`**

In `src/health.rs`, delete the `HealthAction` enum (lines ~15-19) and replace `process_result` (the `fn process_result(...)` near the bottom) with:

```rust
/// New consecutive-failure count after one check. `is_active` is no longer
/// consulted here — the checker never changes `is_active` (manual intent only).
fn process_failures(consecutive_failures: i64, ok: bool) -> i64 {
    if ok {
        0
    } else {
        consecutive_failures + 1
    }
}
```

- [ ] **Step 2: Simplify `run_check` — drop `is_active`, `manage_lifecycle`, and `is_active_change`**

Replace the `run_check` function (the `#[allow(clippy::too_many_arguments)] async fn run_check ...` block) with:

```rust
async fn run_check<F, Fut>(
    client: &reqwest::Client,
    url: &str,
    kind: &str,
    consecutive_failures: i64,
    live_cache: Option<&LiveStatusCache>,
    update: F,
) -> bool
where
    F: FnOnce(&'static str, Option<String>, i64) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let (ok, reason) = do_http_check(client, url, kind, live_cache).await;
    let new_failures = process_failures(consecutive_failures, ok);
    let status: &'static str = if ok { "ok" } else { "error" };
    if let Err(e) = update(status, reason, new_failures).await {
        tracing::error!("health: failed to update {url}: {e}");
        return false;
    }
    ok
}
```

- [ ] **Step 3: Update the four `run_check` callers**

In `check_source`, `probe_source`, `check_playlist_item`, `probe_playlist_item`, change each call to:
- drop the `is_active` arg (was `src.is_active` / `item.is_active`),
- drop the `manage_lifecycle` bool (`true`/`false`),
- change the closure to take `|status, reason, failures|` and call `update_health` with `None` for the `is_active` parameter.

Example — `check_source` becomes:

```rust
async fn check_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    live_cache: &LiveStatusCache,
    src: &Source,
) -> bool {
    run_check(
        client,
        &src.url,
        &src.kind,
        src.consecutive_failures,
        Some(live_cache),
        |status, reason, failures| async move {
            source::update_health(pool, src.id, status, reason.as_deref(), failures, None).await
        },
    )
    .await
}
```

Apply the same shape to `probe_source` (keep its trailing `if ok { probe_and_cache_cors(...) }`), `check_playlist_item` (uses `crate::model::playlist_item::update_health`, `live_cache` arg = `None`), and `probe_playlist_item` (keep its trailing CORS probe).

- [ ] **Step 4: Simplify `record_source_liveness` — never change `is_active`**

Replace `record_source_liveness` with:

```rust
/// Records a single liveness probe result against a source's health. `ok = true`
/// resets failures (status "ok"); `ok = false` counts a failure (status "error",
/// reason "not currently live"). Never changes `is_active` — manual intent is the
/// admin's alone; the unified Status badge reflects liveness separately. Used by
/// the interactive tune path so an active poll doubles as a liveness signal.
pub async fn record_source_liveness(pool: &SqlitePool, src: &Source, ok: bool) {
    let new_failures = process_failures(src.consecutive_failures, ok);
    let status: &'static str = if ok { "ok" } else { "error" };
    let reason = if ok { None } else { Some("not currently live") };
    if let Err(e) =
        source::update_health(pool, src.id, status, reason, new_failures, None).await
    {
        tracing::error!("health: failed to record liveness for {}: {e}", src.url);
    }
}
```

- [ ] **Step 5: Rewrite/remove the affected unit tests**

In `src/health.rs` `#[cfg(test)]`:

DELETE these tests (the behavior they assert no longer exists):
- `test_process_result_ok_resets_failures`
- `test_process_result_error_increments_failures`
- `test_process_result_triggers_disable_at_threshold`
- `test_process_result_already_inactive_not_disabled_again`
- `test_process_result_reenables_inactive_source_on_success`
- `test_process_result_active_source_ok_no_action`
- `test_run_check_probe_mode_never_changes_is_active` (the `manage_lifecycle` distinction is gone)

ADD these replacements:

```rust
#[test]
fn test_process_failures_resets_on_ok() {
    assert_eq!(process_failures(2, true), 0);
}

#[test]
fn test_process_failures_increments_on_error() {
    assert_eq!(process_failures(1, false), 2);
}
```

REPLACE `test_record_source_liveness_disables_then_reenables` with:

```rust
#[tokio::test]
async fn test_record_source_liveness_never_changes_is_active() {
    use crate::model::{channel, source};
    let pool = crate::db::connect("sqlite::memory:").await.unwrap();
    let ch = channel::create(
        &pool,
        channel::NewChannel {
            name: "T".into(),
            category: "t".into(),
            logo_url: None,
            channel_type: channel::ChannelType::Live,
            sort_order: 0,
            loop_anchor: None,
        },
    )
    .await
    .unwrap();
    let mut src = source::create(
        &pool,
        source::NewSource {
            channel_id: ch.id,
            kind: source::SourceKind::YoutubeLive,
            url: "https://youtube.com/watch?v=x".into(),
            priority: 1,
        },
    )
    .await
    .unwrap();

    // Many offline probes must NOT disable the source.
    for _ in 0..(FAILURE_THRESHOLD + 2) {
        record_source_liveness(&pool, &src, false).await;
        src = source::get(&pool, src.id).await.unwrap().unwrap();
    }
    assert!(src.is_active, "liveness probes must never disable a source");
    assert_eq!(src.consecutive_failures, FAILURE_THRESHOLD + 2);
    assert_eq!(src.last_status.as_deref(), Some("error"));

    // A success resets failures, still without touching is_active.
    record_source_liveness(&pool, &src, true).await;
    let after = source::get(&pool, src.id).await.unwrap().unwrap();
    assert!(after.is_active);
    assert_eq!(after.consecutive_failures, 0);
    assert_eq!(after.last_status.as_deref(), Some("ok"));
}
```

KEEP unchanged: `test_live_status_health_mapping`, `probe_source_does_not_reenable_disabled_source`, `probe_playlist_item_does_not_reenable_disabled_item`, `test_probed_hosts_dedup_same_cdn`, `test_check_all_health_checks_each_item_independently`, the CORS tests. (`probe_*_does_not_reenable_*` still pass: `is_active` is now never changed by any path.)

- [ ] **Step 6: Run the health tests + clippy**

Run: `cargo test --lib health:: 2>&1 | tail -20`
Expected: PASS — all remaining `health::tests::*` green, no references to `HealthAction`/`process_result`.

Run: `cargo clippy --lib 2>&1 | tail -15`
Expected: no warnings (no unused `is_active` params, no dead `HealthAction`).

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/health.rs
git commit -m "refactor(health): stop mutating is_active; checker records health only"
```

---

### Task 3: `list_tunable_for_channel` + wire into the tune path

**Files:**
- Modify: `src/model/source.rs`
- Modify: `src/routes/player.rs:161`
- Test: inline `#[cfg(test)]` in `src/model/source.rs`

- [ ] **Step 1: Write the failing test in `src/model/source.rs`**

Add inside `#[cfg(test)] mod tests`:

```rust
#[tokio::test]
async fn test_list_tunable_skips_down_regular_keeps_youtube_and_disabled_excluded() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;

    // 1: active, healthy HLS → tunable
    let ok = create(&pool, hls(ch.id, "https://ok.example.com/s.m3u8", 1)).await.unwrap();
    // 2: active HLS but Down past threshold → skipped
    let down = create(&pool, hls(ch.id, "https://down.example.com/s.m3u8", 2)).await.unwrap();
    update_health(&pool, down.id, "error", Some("dead"), FAILURE_DOWN_THRESHOLD, None).await.unwrap();
    // 3: active HLS errored but BELOW threshold → still tunable
    let flaky = create(&pool, hls(ch.id, "https://flaky.example.com/s.m3u8", 3)).await.unwrap();
    update_health(&pool, flaky.id, "error", Some("blip"), 1, None).await.unwrap();
    // 4: youtube_live recorded as error past threshold → KEPT (waiting/#38 lane)
    let yt = create(&pool, NewSource {
        channel_id: ch.id,
        kind: SourceKind::YoutubeLive,
        url: "https://youtube.com/watch?v=z".into(),
        priority: 4,
    }).await.unwrap();
    update_health(&pool, yt.id, "error", Some("not currently live"), FAILURE_DOWN_THRESHOLD, None).await.unwrap();
    // 5: manually disabled → excluded
    let off = create(&pool, hls(ch.id, "https://off.example.com/s.m3u8", 5)).await.unwrap();
    set_active(&pool, off.id, false).await.unwrap();
    let _ = ok; // ids referenced below by url

    let tunable = list_tunable_for_channel(&pool, ch.id).await.unwrap();
    let urls: Vec<&str> = tunable.iter().map(|s| s.url.as_str()).collect();
    assert_eq!(
        urls,
        vec![
            "https://ok.example.com/s.m3u8",
            "https://flaky.example.com/s.m3u8",
            "https://youtube.com/watch?v=z",
        ],
        "down regular skipped; below-threshold and youtube_live kept; disabled excluded"
    );
}
```

Add this constant near the top of the `tests` module (the threshold matches `health::FAILURE_THRESHOLD`, re-declared locally to avoid a cross-module test dep):

```rust
const FAILURE_DOWN_THRESHOLD: i64 = 3;
```

- [ ] **Step 2: Run it to verify failure**

Run: `cargo test --lib model::source::tests::test_list_tunable 2>&1 | tail -10`
Expected: FAIL — `cannot find function list_tunable_for_channel`.

- [ ] **Step 3: Add the query**

In `src/model/source.rs`, after `list_active_for_channel`, add:

```rust
/// Sources the tune path may try, ordered by priority: active and not
/// observed-Down. A regular source is Down once `last_status='error'` and
/// `consecutive_failures >= 3`; `youtube_live` sources are exempt (kept in
/// rotation so the resolve-time waiting/backoff for idea #38 can fire). `is_active`
/// is the manual gate and is never mutated by health.
pub async fn list_tunable_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>(
        "SELECT * FROM sources \
         WHERE channel_id = ? AND is_active = 1 \
           AND NOT (kind != 'youtube_live' AND last_status = 'error' AND consecutive_failures >= 3) \
         ORDER BY priority ASC",
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}
```

(SQLite note: when `last_status` is NULL, `last_status = 'error'` is NULL, the inner `AND` is falsy, `NOT(...)` is true → the row is kept. Unchecked sources stay tunable.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib model::source::tests::test_list_tunable 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Wire `next_live` to use it**

In `src/routes/player.rs`, change line ~161 from:

```rust
    let sources = source::list_active_for_channel(&state.pool, ch.id)
```

to:

```rust
    let sources = source::list_tunable_for_channel(&state.pool, ch.id)
```

(Leave the VOD path at line ~298 — `playlist_item::list_active_for_channel` — unchanged: VOD position is time-based over active items and must not skip on health.)

- [ ] **Step 6: Run the player tests**

Run: `cargo test --lib routes::player:: 2>&1 | tail -15`
Expected: PASS — existing `next_live`/`tune` tests still green (seed data has no Down sources, so behavior is unchanged for them).

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/model/source.rs src/routes/player.rs
git commit -m "feat(tune): skip observed-Down regular sources; keep youtube_live in rotation"
```

---

### Task 4: Admin source row — one Status column

**Files:**
- Modify: `src/routes/admin/mod.rs` (`AdminSourceRow` + its `From` impl)
- Modify: `src/routes/admin/live_status.rs` (repurpose `/admin/live-status`)
- Modify: `templates/admin/partials/source_row.html`
- Modify: `templates/admin/channel_detail.html` (source table header)
- Test: `tests/http.rs`

- [ ] **Step 1: Add status fields to `AdminSourceRow`**

In `src/routes/admin/mod.rs`, extend the `AdminSourceRow` struct (after `failure_reason`) with:

```rust
    pub status_color: &'static str,
    pub status_glyph: &'static str,
    pub status_title: String,
    /// True only for an active `youtube_live` source: the row lazy-loads its
    /// Status from `/admin/live-status` (yt-dlp probe) instead of rendering inline.
    pub status_lazy: bool,
```

(Keep the existing `budget_badge_class` / `budget_badge_char`.)

- [ ] **Step 2: Compute them in `From<source::Source>`**

Replace the `impl From<source::Source> for AdminSourceRow` body with:

```rust
impl From<source::Source> for AdminSourceRow {
    fn from(s: source::Source) -> Self {
        let (budget_badge_class, budget_badge_char) =
            crate::budget::budget_badge(crate::budget::BudgetStatus::Unknown);
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
            last_status: s.last_status,
            consecutive_failures: s.consecutive_failures,
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
```

- [ ] **Step 3: Repurpose `/admin/live-status` to return the unified Status badge**

In `src/routes/admin/live_status.rs`, replace `badge_parts` and `live_status_badge` so the endpoint computes a `SourceStatus` (active youtube_live + probed live status) and renders it. Keep the `LiveStatusBadgeTemplate` struct and template, populating its `symbol/color/label/title` from `status_badge`:

```rust
fn badge_parts(status: LiveStatus) -> LiveStatusBadgeTemplate {
    let s = crate::status::compute(true, "youtube_live", None, None, Some(status));
    let b = crate::status::status_badge(&s);
    LiveStatusBadgeTemplate {
        symbol: b.glyph,
        color: b.color,
        label: b.label,
        title: b.title,
    }
}
```

(`live_status_badge` handler is unchanged — it still calls `cached_live_status` then `render(badge_parts(status))`.) Update the existing `badge_parts_maps_every_state` test in that file to the new labels:

```rust
#[test]
fn badge_parts_maps_every_state() {
    assert_eq!(badge_parts(LiveStatus::Live).label, "live");
    assert_eq!(badge_parts(LiveStatus::Upcoming(None)).label, "upcoming");
    assert_eq!(
        badge_parts(LiveStatus::Upcoming(None)).title,
        "Scheduled, start time unknown"
    );
    assert_eq!(badge_parts(LiveStatus::PostLive).label, "recorded");
    assert_eq!(badge_parts(LiveStatus::WasLive).label, "recorded");
    assert_eq!(badge_parts(LiveStatus::NotLive).label, "offline");
    assert_eq!(badge_parts(LiveStatus::Offline).label, "offline");
    assert_eq!(badge_parts(LiveStatus::Unknown).label, "?");
}
```

- [ ] **Step 4: Update `source_row.html`**

Replace the three `<td>` blocks for Active (lines ~5-11), Health (~12-29), and Live (~37-44) with a single Status `<td>`:

```html
  <td>
    {% if src.status_lazy %}
    <span hx-get="/admin/live-status?url={{ src.url|urlencode }}"
          hx-trigger="load" hx-swap="outerHTML" style="color:#666">checking…</span>
    {% else %}
    <span style="color:{{ src.status_color }}" title="{{ src.status_title }}">{{ src.status_glyph }}</span>
    {% match src.failure_reason %}
    {% when Some(reason) %}
    {% if src.is_active %}
    <div style="font-size:0.7rem;color:#e94560;margin-top:2px">{{ reason }}</div>
    {% endif %}
    {% when None %}
    {% endmatch %}
    {% endif %}
  </td>
```

(For a non-lazy row, an active source with a `failure_reason` is necessarily Down — disabled rows are `is_active=false`, and active `youtube_live` rows are lazy — so gating the reason line on `src.is_active` is correct without re-checking `last_status`. Leave the Budget `<td>` and the actions `<td>` — including the Enable/Disable toggle button — exactly as they are.)

- [ ] **Step 5: Update the source table header in `channel_detail.html`**

Line ~30: change

```html
      <tr><th>Kind</th><th>URL</th><th>Priority</th><th>Active</th><th>Health</th><th>Budget</th><th>Live</th><th></th></tr>
```

to

```html
      <tr><th>Kind</th><th>URL</th><th>Priority</th><th>Status</th><th>Budget</th><th></th></tr>
```

- [ ] **Step 6: Build to verify templates compile**

Run: `cargo build 2>&1 | tail -10`
Expected: PASS (Askama compiles `source_row.html` into the binary).

- [ ] **Step 7: Add an integration test for the Status column**

In `tests/http.rs`, add (the seed has source id 1 on channel 1, an active HLS source — find the existing channel-detail test for the exact route/auth pattern and mirror it):

```rust
#[tokio::test]
async fn channel_detail_renders_status_column() {
    let app = app().await;
    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin/channels/1")
                .header("authorization", basic_auth_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().to_vec(),
    )
    .unwrap();
    assert!(body.contains("<th>Status</th>"), "source table has a Status header");
    assert!(!body.contains("<th>Live</th>"), "old Live header removed");
}
```

If a `basic_auth_header()` / `app()` helper with a different name already exists in `tests/http.rs`, reuse the existing helpers and admin-auth pattern instead of inventing new ones (check the existing `admin_channel_detail_*` tests).

- [ ] **Step 8: Run the integration test**

Run: `cargo test --test http channel_detail_renders_status_column 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
cargo fmt
git add src/routes/admin/mod.rs src/routes/admin/live_status.rs templates/admin/partials/source_row.html templates/admin/channel_detail.html tests/http.rs
git commit -m "feat(admin): single Status column for source rows"
```

---

### Task 5: Admin playlist-item row — one Status column

**Files:**
- Modify: `src/routes/admin/mod.rs` (`AdminPlaylistItemRow` + its `From` impl)
- Modify: `templates/admin/partials/playlist_item_row.html`
- Modify: `templates/admin/channel_detail.html` (playlist table header)
- Test: `tests/http.rs`

- [ ] **Step 1: Add status fields to `AdminPlaylistItemRow`**

In `src/routes/admin/mod.rs`, extend `AdminPlaylistItemRow` (after `failure_reason`) with:

```rust
    pub status_color: &'static str,
    pub status_glyph: &'static str,
    pub status_title: String,
```

(No `status_lazy` — playlist items are never `youtube_live` for liveness purposes; their kind is detected per-URL and they always render inline.)

- [ ] **Step 2: Compute them in `From<playlist_item::PlaylistItem>`**

Replace the `impl From<playlist_item::PlaylistItem> for AdminPlaylistItemRow` body to compute the status (always inline; live = `None`, kind = `"hls"` placeholder so it never takes the youtube branch):

```rust
impl From<playlist_item::PlaylistItem> for AdminPlaylistItemRow {
    fn from(i: playlist_item::PlaylistItem) -> Self {
        let (budget_badge_class, budget_badge_char) =
            crate::budget::budget_badge(crate::budget::BudgetStatus::Unknown);
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
            last_status: i.last_status,
            consecutive_failures: i.consecutive_failures,
            failure_reason: i.failure_reason,
            status_color: badge.color,
            status_glyph: badge.glyph,
            status_title: badge.title,
        }
    }
}
```

- [ ] **Step 3: Update `playlist_item_row.html`**

Replace the Active (lines ~6-12) and Health (~13-32) `<td>` blocks with one Status `<td>`:

```html
  <td>
    <span style="color:{{ item.status_color }}" title="{{ item.status_title }}">{{ item.status_glyph }}</span>
    {% match item.failure_reason %}
    {% when Some(reason) %}
    {% if item.is_active %}
    <div style="font-size:0.7rem;color:#e94560;margin-top:2px">{{ reason }}</div>
    {% endif %}
    {% when None %}
    {% endmatch %}
  </td>
```

(Leave the Budget `<td>` and the actions `<td>` unchanged.)

- [ ] **Step 4: Update the playlist table header in `channel_detail.html`**

Line ~101: change

```html
      <tr><th>#</th><th>Title</th><th>URL</th><th>Duration</th><th>Active</th><th>Health</th><th>Budget</th><th></th></tr>
```

to

```html
      <tr><th>#</th><th>Title</th><th>URL</th><th>Duration</th><th>Status</th><th>Budget</th><th></th></tr>
```

- [ ] **Step 5: Build to verify templates compile**

Run: `cargo build 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Add an integration test for a VOD channel's playlist Status column**

In `tests/http.rs` (seed channel 4 is `VOD Has Items`):

```rust
#[tokio::test]
async fn channel_detail_vod_renders_playlist_status_column() {
    let app = app().await;
    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin/channels/4")
                .header("authorization", basic_auth_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().to_vec(),
    )
    .unwrap();
    assert!(body.contains("<th>Status</th>"));
    assert!(!body.contains("<th>Active</th>"), "old Active header removed");
}
```

(Reuse the existing auth/app helpers as in Task 4 Step 7.)

- [ ] **Step 7: Run the test**

Run: `cargo test --test http channel_detail_vod_renders_playlist_status_column 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/routes/admin/mod.rs templates/admin/partials/playlist_item_row.html templates/admin/channel_detail.html tests/http.rs
git commit -m "feat(admin): single Status column for playlist-item rows"
```

---

### Task 6: Guide — most-optimistic channel Status

**Files:**
- Modify: `src/routes/guide/mod.rs` (snapshot live cache, pass to `build_guide_data`)
- Modify: `src/routes/guide/data.rs` (`ChannelRow` fields + per-channel status)
- Modify: `src/routes/guide/badges.rs` (add `derive_channel_status`; remove `derive_health_status`/`health_badge`/`HealthStatus`)
- Modify: `templates/partials/epg_content.html`
- Test: inline `#[cfg(test)]` in `src/routes/guide/badges.rs`

- [ ] **Step 1: Add `derive_channel_status` to `badges.rs` and remove the old health helpers**

In `src/routes/guide/badges.rs`:

1. DELETE `HealthStatus` enum, `derive_health_status`, `health_badge`, and their tests (`test_derive_health_status_*`).
2. Keep `category_icon`, `budget_for_url`, `vod_budget_url`.
3. ADD:

```rust
use crate::media::resolver::LiveStatus;
use crate::status::{self, SourceStatus};

/// Minimal per-source facts the guide needs to compute a channel's status.
pub(super) struct SourceFacts {
    pub kind: String,
    pub is_active: bool,
    pub last_status: Option<String>,
    pub failure_reason: Option<String>,
}

/// The channel's aggregated Status = the most-optimistic status across its
/// sources. For `youtube_live` sources the live status comes from the warm cache
/// snapshot (cold → Unchecked, never Down). VOD channels (no sources) are `Ok`
/// (reachable) — matching the prior "VOD always healthy" behavior.
pub(super) fn derive_channel_status(
    channel_type: &ChannelType,
    sources: &[SourceFacts],
    live_snapshot: &std::collections::HashMap<String, LiveStatus>,
    source_urls: &[String],
) -> SourceStatus {
    match channel_type {
        ChannelType::VodLoop => SourceStatus::Ok,
        ChannelType::Live => {
            if sources.is_empty() {
                return SourceStatus::Unchecked;
            }
            status::most_optimistic(sources.iter().zip(source_urls).map(|(f, url)| {
                let live = if f.kind == "youtube_live" {
                    live_snapshot.get(url).copied()
                } else {
                    None
                };
                status::compute(
                    f.is_active,
                    &f.kind,
                    f.last_status.as_deref(),
                    f.failure_reason.as_deref(),
                    live,
                )
            }))
        }
    }
}
```

ADD a test:

```rust
#[test]
fn test_derive_channel_status_most_optimistic() {
    use std::collections::HashMap;
    let snapshot: HashMap<String, LiveStatus> = HashMap::new();
    let sources = vec![
        SourceFacts { kind: "hls".into(), is_active: true, last_status: Some("error".into()), failure_reason: Some("dead".into()) },
        SourceFacts { kind: "hls".into(), is_active: true, last_status: Some("ok".into()), failure_reason: None },
    ];
    let urls = vec!["https://a/s.m3u8".to_string(), "https://b/s.m3u8".to_string()];
    assert_eq!(
        derive_channel_status(&ChannelType::Live, &sources, &snapshot, &urls),
        SourceStatus::Ok,
        "one OK source beats a Down sibling"
    );

    let no_sources: Vec<SourceFacts> = vec![];
    assert_eq!(
        derive_channel_status(&ChannelType::Live, &no_sources, &snapshot, &[]),
        SourceStatus::Unchecked
    );
    assert_eq!(
        derive_channel_status(&ChannelType::VodLoop, &no_sources, &snapshot, &[]),
        SourceStatus::Ok
    );
}
```

- [ ] **Step 2: Update `ChannelRow` and `build_guide_data` in `data.rs`**

In `src/routes/guide/data.rs`:

1. Change the `ChannelRow` struct fields `health_badge_class` / `health_badge_char` to:

```rust
    pub status_color: &'static str,
    pub status_glyph: &'static str,
    pub status_title: String,
```

2. Update the `use super::badges::{...}` import: drop `derive_health_status, health_badge`; add `derive_channel_status, SourceFacts`.

3. Change the function signature to accept the live snapshot:

```rust
pub(super) async fn build_guide_data(
    pool: &SqlitePool,
    cors_cache: &std::collections::HashMap<String, bool>,
    live_snapshot: &std::collections::HashMap<String, crate::media::resolver::LiveStatus>,
    category: &str,
    offset_hours: i64,
) -> anyhow::Result<GuideData> {
```

4. Replace the source-id-set fetch (`all_source_ids` / `active_source_ids` via `channel_ids_with_*`) and the `SourceUrlRow` query with a single fetch of all sources grouped by channel. Replace lines ~73-93 with:

```rust
    let sources_by_channel: std::collections::HashMap<i64, Vec<source::Source>> =
        source::list_all(pool).await?.into_iter().fold(
            std::collections::HashMap::new(),
            |mut acc, s| {
                acc.entry(s.channel_id).or_default().push(s);
                acc
            },
        );
```

5. Inside the `for ch in &channels` loop, derive `budget_url` for Live channels from the channel's first **active** source (preserving today's budget behavior) and compute the status. Replace the `ChannelType::Live` arm of the `match ch.channel_type()` and the badge-building block. The Live arm becomes:

```rust
            ChannelType::Live => {
                let first_active_url = sources_by_channel
                    .get(&ch.id)
                    .and_then(|v| v.iter().find(|s| s.is_active).map(|s| s.url.clone()));
                (
                    vec![epg::live_entry(ch.id, &ch.name, window_start, window_end)],
                    first_active_url,
                )
            }
```

Then replace the status/badge computation (the `let health = derive_health_status(...)` block through the `health_badge`/`budget_badge` lines) with:

```rust
        let empty: Vec<source::Source> = Vec::new();
        let chan_sources = sources_by_channel.get(&ch.id).unwrap_or(&empty);
        let facts: Vec<super::badges::SourceFacts> = chan_sources
            .iter()
            .map(|s| super::badges::SourceFacts {
                kind: s.kind.clone(),
                is_active: s.is_active,
                last_status: s.last_status.clone(),
                failure_reason: s.failure_reason.clone(),
            })
            .collect();
        let source_urls: Vec<String> = chan_sources.iter().map(|s| s.url.clone()).collect();
        let status = super::badges::derive_channel_status(
            &ch.channel_type(),
            &facts,
            live_snapshot,
            &source_urls,
        );
        let status_badge = crate::status::status_badge(&status);
        let budget = budget_for_url(budget_url.as_deref(), cors_cache);
        let (budget_badge_class, budget_badge_char) = budget_badge(budget);
        rows.push(ChannelRow {
            name: ch.name.clone(),
            category_icon: category_icon(&ch.category),
            status_color: status_badge.color,
            status_glyph: status_badge.glyph,
            status_title: status_badge.title,
            budget_badge_class,
            budget_badge_char,
            programs,
        });
```

(Remove the now-unused `source::channel_ids_with_any_sources` / `channel_ids_with_active_sources` imports/calls. Leave those two functions in `model/source.rs` only if still referenced elsewhere; if `cargo clippy` flags them as dead, delete them in this task.)

- [ ] **Step 3: Pass the live snapshot from the guide handler**

In `src/routes/guide/mod.rs`, around line 75-76, change:

```rust
    let cors_snapshot = state.cors_cache.read().await.clone();
    build_guide_data(&state.pool, &cors_snapshot, &category, offset_hours)
```

to:

```rust
    let cors_snapshot = state.cors_cache.read().await.clone();
    let live_snapshot: std::collections::HashMap<String, crate::media::resolver::LiveStatus> = state
        .live_cache
        .read()
        .await
        .iter()
        .map(|(url, (status, _))| (url.clone(), *status))
        .collect();
    build_guide_data(&state.pool, &cors_snapshot, &live_snapshot, &category, offset_hours)
```

- [ ] **Step 4: Update `epg_content.html`**

Line ~43: change the health badge span to a status badge with inline color. Replace:

```html
<span class="status-badge {{ row.health_badge_class }}">{{ row.health_badge_char }}</span>
```

with:

```html
<span class="status-badge" style="color:{{ row.status_color }}" title="{{ row.status_title }}">{{ row.status_glyph }}</span>
```

(Leave the budget badge span and the rest of the line unchanged.)

- [ ] **Step 5: Build + run guide tests**

Run: `cargo build 2>&1 | tail -10`
Expected: PASS.

Run: `cargo test --lib routes::guide:: 2>&1 | tail -15`
Expected: PASS — `test_derive_channel_status_most_optimistic` + retained `category_icon`/`budget`/`vod_budget_url` tests green.

- [ ] **Step 6: Run the guide integration tests**

Run: `cargo test --test http guide 2>&1 | tail -15`
Expected: PASS — `test_guide_returns_200`, `test_guide_partial_returns_200`, `test_guide_renders_*_budget_badge_*`, `guide_embeds_epg_channels_json` still green. If any asserted on the old `health-ok`/`health-down` class strings, update those assertions to check the status glyph (e.g. `●`/`○`) instead.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/routes/guide/
git add templates/partials/epg_content.html
git commit -m "feat(guide): channel Status = most-optimistic across sources"
```

---

### Task 7: Architecture doc + ideas log

**Files:**
- Create: `docs/architecture/source-status.md`
- Modify: `docs/ideas.md`

- [ ] **Step 1: Write the architecture note**

Create `docs/architecture/source-status.md`:

```markdown
# Source Status model

MyTV shows two indicators per source/item: **Status** and **Budget**.

## Two axes
- **Status** (`src/status.rs`, `SourceStatus`) — *availability*: is this source usable right now? Folds the former Active + Health + Live indicators into one value.
- **Budget** (`src/budget.rs`) — *network cost*: can the browser reach the CDN directly (CORS ⚡) or must we proxy (☁)?

## Intent vs observation
- `is_active` is **manual intent only** — the admin's switch. The health checker never mutates it (`health::process_failures` records `last_status`/`consecutive_failures`/`failure_reason`; it does not disable or re-enable).
- Observed availability comes from persisted health (regular sources) and the cached `LiveStatus` (`youtube_live`).

## Status precedence (`status::compute`)
1. `Disabled` — `is_active = false`.
2. `youtube_live` → from cached `LiveStatus`: Live / Upcoming / Recorded (was/post-live) / Offline / Unchecked (cold cache or Unknown). Never `Down`.
3. regular / VOD → from `last_status`: `Down` (any `error`, with reason) / `Ok` / `Unchecked`.

## Tune gating (`source::list_tunable_for_channel`)
`is_active = 1 AND NOT (kind != 'youtube_live' AND last_status = 'error' AND consecutive_failures >= 3)`. Regular Down sources are skipped at read time (rejoin automatically on recovery, no `is_active` write); `youtube_live` stays in rotation so the resolve-time waiting/backoff (idea #38) can fire.

## Guide aggregation (`badges::derive_channel_status`)
The channel badge is the **most-optimistic** status across its sources: `Live`=`Ok` > `Upcoming` > `Recorded` > `Offline` > `Unchecked` > `Down` > `Disabled`. The guide reads only persisted health + the warm live-status cache; it never probes.
```

- [ ] **Step 2: Mark the idea done in `docs/ideas.md`**

Add a new entry at the end of `docs/ideas.md` (next number after the current last, #41):

```markdown
42. ~~**Unify source status indicators (Active/Health/Live → Status)**~~ — done: new `src/status.rs` (`SourceStatus` + `compute`/`status_badge`/`most_optimistic`) collapses Active+Health+Live into one Status badge; Budget stays separate. `is_active` is now pure manual intent — the health checker records health but never disables/re-enables (`process_failures`); the tune path skips observed-Down regular sources via `source::list_tunable_for_channel` while `youtube_live` stays in rotation for #38. Admin source + playlist rows and the guide render the unified Status (guide = most-optimistic across sources). No migration. Spec: `docs/superpowers/specs/2026-06-11-unified-source-status-design.md`; arch: `docs/architecture/source-status.md`.
```

- [ ] **Step 3: Commit**

```bash
git add docs/architecture/source-status.md docs/ideas.md
git commit -m "docs: source-status architecture note; mark idea #42 done"
```

---

### Task 8: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Format check**

Run: `cargo fmt --check`
Expected: no output (clean).

- [ ] **Step 2: Clippy with warnings denied (CI parity)**

Run: `cargo clippy --all-targets -- -D warnings 2>&1 | tail -20`
Expected: no warnings. Fix any dead-code (e.g. removed `channel_ids_with_*_sources`, removed imports) inline, then re-run.

- [ ] **Step 3: Full test suite**

Run: `cargo test 2>&1 | tail -25`
Expected: all unit + integration tests pass (ignored tests still ignored). The total count changes vs the documented 336 because health/source/guide tests were rewritten — that is expected.

- [ ] **Step 4: Release build (Askama + binary)**

Run: `cargo build 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 5: Update the test count in `CLAUDE.md`**

Read the new total from Step 3's output and update the `cargo test` line in `CLAUDE.md` (currently "336 tests: 266 unit + 70 integration (7 ignored)") to the new numbers.

- [ ] **Step 6: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update test count after status unification"
```

---

## Notes for the implementer

- **VOD tune path is deliberately untouched.** Only `next_live` (live sources) switches to `list_tunable_for_channel`. VOD position is time-based over `list_active_for_channel` and must not skip items on health.
- **`youtube_live` never shows `Down`.** Its status derives from the live cache; offline is `Offline` (recoverable). A cold cache shows `Unchecked`, not `Down`.
- **Display `Down` = any error**, matching today's red-dot UI; the `>= 3` threshold lives only in `list_tunable_for_channel`.
- **The Enable/Disable toggle button stays** in the admin actions column — it is the `is_active` control.
- If `tests/http.rs` helper names differ from `app()` / `basic_auth_header()`, reuse whatever the existing `admin_channel_detail_*` tests use; do not invent new helpers.

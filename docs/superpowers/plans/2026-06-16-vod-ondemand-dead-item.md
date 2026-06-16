# Idea #53 — VOD-on-demand dead-item handling — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A dead VOD-on-demand item (e.g. its R2 object was deleted) is automatically disabled after `FAILURE_THRESHOLD` failures, surfaced with its reason in admin, and skipped in the player instead of stranding the viewer.

**Architecture:** A pure `playlist_item::is_dead` predicate (reusing `source::FAILURE_THRESHOLD`) and a single owner `playlist_item::apply_health_result` that counts a probe result and applies the disable rule. Both the 15-min health loop and the interactive tune handler call it. Disabled items already drop out of `list_active_for_channel`, so they vanish from `/playlist` and `/item` (steady-state skipping is free). The player additionally auto-skips an item that dies mid-session (covering both the `/item` fetch failure and the `<video>` 404 paths). Admin shows the disable reason on inactive rows.

**Tech Stack:** Rust 1.96, SQLx 0.7 (runtime `query_as`), Tokio, Askama templates, vanilla JS. `cargo fmt` + `cargo clippy -- -D warnings` + `cargo test` gate every commit.

**Spec:** `docs/superpowers/specs/2026-06-16-vod-ondemand-dead-item-design.md`

---

### Task 1: Pure `is_dead` predicate

**Files:**
- Modify: `src/model/playlist_item.rs` (add fn after `update_health`, ~line 193; add test in the `#[cfg(test)] mod tests` at the end of the file)

- [ ] **Step 1: Write the failing truth-table test**

Add inside `mod tests` (after the existing tests, before the closing `}` of the module). It tests a pure fn — no DB:

```rust
    #[test]
    fn test_is_dead_truth_table() {
        let t = crate::model::source::FAILURE_THRESHOLD;
        // ok / null status → never dead, even past threshold
        assert!(!is_dead(Some("ok"), t + 5));
        assert!(!is_dead(None, t + 5));
        // errored but below threshold → not dead
        assert!(!is_dead(Some("error"), t - 1));
        // errored exactly at threshold → dead
        assert!(is_dead(Some("error"), t));
        // errored above threshold → dead
        assert!(is_dead(Some("error"), t + 1));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p mytv --lib model::playlist_item::tests::test_is_dead_truth_table`
Expected: FAIL — `cannot find function ` is_dead`` (does not compile yet).

- [ ] **Step 3: Implement `is_dead`**

Add to `src/model/playlist_item.rs` immediately after the `update_health` fn (after line 193):

```rust
/// An on-demand/VOD playlist item is "dead" when its last health probe errored
/// and it has failed at least `source::FAILURE_THRESHOLD` consecutive times.
/// Unlike `source::is_observed_down`, there is no `youtube_live` exemption: a
/// playlist item is never a live broadcast, so an errored item past threshold is
/// always dead (a deleted R2 object never recovers on its own).
pub fn is_dead(last_status: Option<&str>, consecutive_failures: i64) -> bool {
    last_status == Some("error")
        && consecutive_failures >= crate::model::source::FAILURE_THRESHOLD
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p mytv --lib model::playlist_item::tests::test_is_dead_truth_table`
Expected: PASS.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/model/playlist_item.rs
git commit -m "feat(playlist): pure is_dead predicate (idea #53)"
```

---

### Task 2: `apply_health_result` — the single disable owner

**Files:**
- Modify: `src/model/playlist_item.rs` (add fn after `is_dead`; add tests in `mod tests`)

- [ ] **Step 1: Write the failing behaviour tests**

Add inside `mod tests`. These use the existing `test_pool()`, `make_channel()`, `item()`, `create()` helpers already in the module:

```rust
    #[tokio::test]
    async fn apply_health_result_disables_only_at_threshold() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "dead", 60, 0)).await.unwrap();

        // Fail up to (threshold - 1): stays active.
        let mut cur = it.clone();
        for _ in 0..(crate::model::source::FAILURE_THRESHOLD - 1) {
            apply_health_result(&pool, &cur, false, Some("HTTP 404"))
                .await
                .unwrap();
            cur = get(&pool, it.id).await.unwrap().unwrap();
            assert!(cur.is_active, "must stay active below threshold");
        }

        // The failure that reaches threshold disables it.
        apply_health_result(&pool, &cur, false, Some("HTTP 404"))
            .await
            .unwrap();
        let after = get(&pool, it.id).await.unwrap().unwrap();
        assert!(!after.is_active, "must be disabled at threshold");
        assert_eq!(after.last_status.as_deref(), Some("error"));
        assert_eq!(after.failure_reason.as_deref(), Some("HTTP 404"));

        // A disabled item is gone from the active list → skipped on playback.
        let active = list_active_for_channel(&pool, ch.id).await.unwrap();
        assert!(active.iter().all(|i| i.id != it.id));
    }

    #[tokio::test]
    async fn apply_health_result_recovery_resets_failures_but_not_active() {
        let pool = test_pool().await;
        let ch = make_channel(&pool).await;
        let it = create(&pool, item(ch.id, "recovers", 60, 0)).await.unwrap();

        // Drive it dead.
        let mut cur = it.clone();
        for _ in 0..crate::model::source::FAILURE_THRESHOLD {
            apply_health_result(&pool, &cur, false, Some("HTTP 404"))
                .await
                .unwrap();
            cur = get(&pool, it.id).await.unwrap().unwrap();
        }
        assert!(!cur.is_active);

        // A later OK probe resets failures/status but does NOT auto-re-enable.
        apply_health_result(&pool, &cur, true, None).await.unwrap();
        let after = get(&pool, it.id).await.unwrap().unwrap();
        assert_eq!(after.consecutive_failures, 0);
        assert_eq!(after.last_status.as_deref(), Some("ok"));
        assert!(!after.is_active, "recovery never re-enables; admin does that");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p mytv --lib model::playlist_item::tests::apply_health_result`
Expected: FAIL — `cannot find function ` apply_health_result``.

- [ ] **Step 3: Implement `apply_health_result`**

Add to `src/model/playlist_item.rs` immediately after `is_dead`:

```rust
/// Records one health-probe result against an item and applies the auto-disable
/// rule. This is the single owner of the disable decision: both the background
/// health loop and the interactive tune path call it, so the rule lives in one
/// place even with two writers.
///
/// - `ok == true` resets failures (status "ok"); never re-enables — re-enabling
///   a disabled item is a manual admin action.
/// - `ok == false` counts a failure (status "error"); disables once `is_dead`.
///
/// Returns `ok` for the caller's convenience.
pub async fn apply_health_result(
    pool: &SqlitePool,
    item: &PlaylistItem,
    ok: bool,
    reason: Option<&str>,
) -> Result<bool> {
    let new_failures = if ok { 0 } else { item.consecutive_failures + 1 };
    let status = if ok { "ok" } else { "error" };
    let is_active = if is_dead(Some(status), new_failures) {
        Some(false)
    } else {
        None
    };
    update_health(pool, item.id, status, reason, new_failures, is_active).await?;
    Ok(ok)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p mytv --lib model::playlist_item::tests::apply_health_result`
Expected: PASS (both tests).

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/model/playlist_item.rs
git commit -m "feat(playlist): apply_health_result owns the auto-disable rule (idea #53)"
```

---

### Task 3: Wire the background health loop to auto-disable

**Files:**
- Modify: `src/health.rs` — `check_playlist_item` (lines 138-163)

The current `check_playlist_item` routes through `run_check`, whose closure passes `is_active = None` (never disables). Replace it so it calls `do_http_check` directly, then `apply_health_result`. `run_check` is left untouched (still used by `check_source`). The admin "Test" button path (`probe` → `ProbeTarget::PlaylistItem`) is deliberately **not** changed — a single manual diagnostic probe should not disable an item; only the looping checker and the tune path enforce the threshold.

- [ ] **Step 1: Replace the body of `check_playlist_item`**

Replace lines 138-163 (`async fn check_playlist_item ... }`) with:

```rust
async fn check_playlist_item(
    pool: &SqlitePool,
    client: &reqwest::Client,
    item: &crate::model::playlist_item::PlaylistItem,
) -> bool {
    let kind = crate::model::source::SourceKind::detect(&item.url);
    let (ok, reason) = do_http_check(client, &item.url, kind.as_str(), None).await;
    match crate::model::playlist_item::apply_health_result(pool, item, ok, reason.as_deref())
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("health: failed to update {}: {e}", item.url);
            false
        }
    }
}
```

- [ ] **Step 2: Verify it compiles and the full suite passes**

Run: `cargo test -p mytv --lib health`
Expected: PASS — existing health tests still pass. (The disable behaviour itself is covered by Task 2's model tests; this task is mechanical wiring, since `do_http_check` performs real network I/O and cannot be driven deterministically in a unit test.)

- [ ] **Step 3: Full build + lint**

Run: `cargo clippy -- -D warnings && cargo test`
Expected: PASS.

- [ ] **Step 4: Format, commit**

```bash
cargo fmt
git add src/health.rs
git commit -m "feat(health): on-demand items auto-disable past threshold (idea #53)"
```

---

### Task 4: Wire the interactive tune handler to count failures

**Files:**
- Modify: `src/routes/player.rs` — the `item` handler's resolver-error arm (lines 111-114)

When a youtube-VOD item fails to resolve, the tune handler hits its `Err` arm. Record that failure so repeated viewer attempts contribute to the disable threshold. (A plain self-hosted MP4 resolves to `Ok` without network, so its dead state is caught by the health loop and the player's `<video>` 404 path, not here — see Task 6.)

- [ ] **Step 1: Update the error arm**

Replace lines 111-114:

```rust
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
```

with:

```rust
        Err(e) => {
            tracing::warn!(url = %item.url, error = %e, "resolver failed for vod item");
            let _ =
                playlist_item::apply_health_result(&state.pool, item, false, Some(&e.to_string()))
                    .await;
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
```

(`item` here is the `&PlaylistItem` bound at line 97-100; `playlist_item` is already imported at line 12.)

- [ ] **Step 2: Verify the existing on-demand tune test still passes**

Run: `cargo test -p mytv --test http test_tune_on_demand_returns_first_item`
Expected: PASS — the success path is unchanged.

- [ ] **Step 3: Full build + lint**

Run: `cargo clippy -- -D warnings && cargo test`
Expected: PASS.

- [ ] **Step 4: Format, commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "feat(player): failed item tune counts toward auto-disable (idea #53)"
```

---

### Task 5: Surface the disable reason in admin (with HTTP test)

**Files:**
- Modify: `tests/fixtures/seed.sql` (add a disabled, errored on-demand item)
- Modify: `templates/admin/partials/playlist_item_row.html` (lines 8-14)
- Test: `tests/http.rs` (new test)

Today the row hides `failure_reason` when `!is_active` (line 10), so an auto-disabled item looks identical to a manually-disabled one. Show the reason on inactive rows too, prefixed "auto-disabled —".

- [ ] **Step 1: Add a dead on-demand item to the seed**

Append to `tests/fixtures/seed.sql` (after the channel-6 on-demand items at the end):

```sql
-- Auto-disabled on-demand item (idea #53): dead R2 object, past FAILURE_THRESHOLD.
INSERT INTO playlist_items
  (channel_id, title, url, duration_secs, sort_order, is_active, last_status, consecutive_failures, failure_reason)
VALUES
  (6, 'Dead Item', 'https://vod.example.com/gone.mp4', 90, 3, 0, 'error', 3, 'HTTP 404');
```

- [ ] **Step 2: Write the failing admin-render test**

Add to `tests/http.rs`. The channel-detail page (`channel_detail`, route `GET /admin/channels/{id}`, template `admin/channel_detail.html`) renders the playlist-item rows. The GET-with-auth helper already in the file is `authed(uri)`; body bytes are read with `axum::body::to_bytes` (as at `tests/http.rs:435`). Use channel 6:

```rust
#[tokio::test]
async fn admin_channel_detail_shows_auto_disabled_reason() {
    let response = app()
        .await
        .oneshot(authed("/admin/channels/6"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&bytes);
    // The dead item's reason shows even though it is inactive, marked auto-disabled.
    assert!(
        body.contains("auto-disabled — HTTP 404"),
        "admin should surface the auto-disable reason on a disabled item; got: {body}"
    );
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p mytv --test http admin_channel_detail_shows_auto_disabled_reason`
Expected: FAIL — the reason is currently hidden on inactive rows, so the body does not contain `auto-disabled — HTTP 404`.

- [ ] **Step 4: Update the template**

Replace lines 8-14 of `templates/admin/partials/playlist_item_row.html`:

```html
    {% match item.failure_reason %}
    {% when Some(reason) %}
    {% if item.is_active %}
    <div style="font-size:0.7rem;color:#e94560;margin-top:2px">{{ reason }}</div>
    {% endif %}
    {% when None %}
    {% endmatch %}
```

with:

```html
    {% match item.failure_reason %}
    {% when Some(reason) %}
    {% if item.is_active %}
    <div style="font-size:0.7rem;color:#e94560;margin-top:2px">{{ reason }}</div>
    {% else %}
    <div style="font-size:0.7rem;color:#e94560;margin-top:2px">auto-disabled — {{ reason }}</div>
    {% endif %}
    {% when None %}
    {% endmatch %}
```

(Heuristic note: a manually-toggled item carries no `failure_reason` because `set_active` does not set one, so it shows no line; only a health/tune-recorded failure produces the "auto-disabled —" line. This is acceptable for the admin diagnostic.)

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p mytv --test http admin_channel_detail_shows_auto_disabled_reason`
Expected: PASS.

- [ ] **Step 6: Run the full suite (seed change touches many tests)**

Run: `cargo test`
Expected: PASS — the new seed row is `is_active = 0`, so existing channel-6 on-demand tests (which use `list_active_for_channel`) are unaffected. If any test asserts an exact on-demand item count for channel 6, update it to account for the new inactive row (inactive rows are excluded from `/playlist`, so `/playlist`-based assertions should not change).

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add tests/fixtures/seed.sql templates/admin/partials/playlist_item_row.html tests/http.rs
git commit -m "feat(admin): surface auto-disable reason on inactive items (idea #53)"
```

---

### Task 6: Player auto-skip + toast for a dead item

**Files:**
- Modify: `templates/guide.html` (add a toast element after line 28)
- Modify: `templates/base.html` (add `#player-toast` CSS near line 28; add `showToast` + `odSkipFrom`; wire the two failure sites)

The on-demand player has two failure surfaces, both currently ending in `showPlayerError()`:
1. **`odPlayIndex` fetch failure** (`base.html` ~line 555/567) — `/item/:id` returns non-OK (youtube-VOD resolve fail, or an already-disabled item → 422). The failing index is the requested `i`.
2. **`<video>` playback error** (`base.html` lines 363-373) — a self-hosted MP4 whose object 404s; for on-demand this hits line 366. The failing item is the loaded `odIndex`.

Both should toast the dead item's title and advance to the next item; if none remains, fall back to `showPlayerError()`. Advance is **forward-only** (no wrap), guaranteeing termination if every item is dead. This is JS with no automated harness in this repo — verify manually (Step 6).

- [ ] **Step 1: Add the toast element**

In `templates/guide.html`, insert after line 28 (`<div id="player-ended">…</div>`):

```html
  <div id="player-toast" hidden></div>
```

- [ ] **Step 2: Add toast CSS**

In `templates/base.html`, near the existing `#player-error{…}` rule (line 28), add:

```css
    #player-toast{position:absolute;bottom:120px;left:50%;transform:translateX(-50%);
      z-index:9;background:rgba(0,0,0,.85);color:#fff;padding:8px 14px;border-radius:6px;
      font-size:0.85rem;max-width:80%;text-align:center}
```

- [ ] **Step 3: Add `showToast` and `odSkipFrom` helpers**

In `templates/base.html`, add these two functions next to `showPlayerError` (after line 410, the close of `hidePlayerError`):

```javascript
      var _toastTimer = null;
      function showToast(msg) {
        var el = document.getElementById('player-toast');
        if (!el) return;
        el.textContent = msg;
        el.hidden = false;
        if (_toastTimer) clearTimeout(_toastTimer);
        _toastTimer = setTimeout(function () { el.hidden = true; }, 2500);
      }

      // Advance past a dead on-demand item. `failedIndex` is the item that just
      // failed (to play or to resolve). Forward-only: if nothing follows, show
      // the error overlay. Never wraps, so an all-dead playlist terminates.
      function odSkipFrom(failedIndex) {
        var failed = odItems[failedIndex];
        if (failed) showToast(failed.title + ' unavailable — skipping…');
        if (failedIndex + 1 < odItems.length) {
          odPlayIndex(failedIndex + 1, 0);
        } else {
          showPlayerError();
        }
      }
```

- [ ] **Step 4: Wire the `odPlayIndex` fetch-failure path**

In `templates/base.html`, in `odPlayIndex` (the `.catch` at ~line 567), replace:

```javascript
          .catch(function(err) {
            if (gen !== odPlayGen || chanAtRequest !== odChannelId) return;
            if (typeof debugLog === 'function') debugLog('error', 'on-demand item: ' + err);
            showPlayerError();
          });
```

with:

```javascript
          .catch(function(err) {
            if (gen !== odPlayGen || chanAtRequest !== odChannelId) return;
            if (typeof debugLog === 'function') debugLog('error', 'on-demand item: ' + err);
            odSkipFrom(i);
          });
```

- [ ] **Step 5: Wire the `<video>` on-demand error path**

In `templates/base.html`, in the direct-MP4 `video.onerror` handler (lines 363-373), replace the on-demand branch at line 366:

```javascript
            if (odChannelId === currentChannelId) { showPlayerError(); return; }
```

with:

```javascript
            if (odChannelId === currentChannelId) { odSkipFrom(odIndex); return; }
```

- [ ] **Step 6: Manual verification**

Templates are not unit-tested in this repo. Verify by hand:

```bash
cargo run
```

Then, against `http://localhost:3000`:
1. In admin, add a `vod_on_demand` channel with three items where item 2's URL points at a non-existent file (e.g. `https://vod.example.com/missing.mp4`) and items 1 & 3 are playable.
2. Open the channel in the player, click item 2.
3. Expected: a toast "*…* unavailable — skipping…" appears and the player advances to item 3 (not the error overlay). Confirm an all-dead playlist eventually shows the error overlay rather than looping.

- [ ] **Step 7: Format, commit**

```bash
cargo fmt
git add templates/guide.html templates/base.html
git commit -m "feat(player): auto-skip dead on-demand items with a toast (idea #53)"
```

---

### Task 7: Mark idea #53 done

**Files:**
- Modify: `docs/IDEAS.md` (move #53 from Open to Done)
- Modify: `docs/CHANGELOG.md` (add #53 entry)

- [ ] **Step 1: Update `docs/IDEAS.md`**

Remove the `53.` entry from the `## Open` section, and update the `## Done` line count/range to include #53 (currently: "48 completed ideas (foundational work + backlog #9–#42, #44–#49, #52)" → add #53).

- [ ] **Step 2: Add a `docs/CHANGELOG.md` entry**

Add an entry for idea #53 mirroring the format of the existing entries (one short paragraph: dead on-demand items auto-disable after `FAILURE_THRESHOLD` failures via `playlist_item::apply_health_result`, surfaced in admin and auto-skipped in the player; reuses the shared threshold, no migration).

- [ ] **Step 3: Commit**

```bash
git add docs/IDEAS.md docs/CHANGELOG.md
git commit -m "docs: mark idea #53 done (VOD-on-demand dead-item handling)"
```

---

## Final verification

- [ ] Run the whole gate once more:

```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```
Expected: all green (399+ tests, plus the new `is_dead`, `apply_health_result`, and admin-render tests).

- [ ] Confirm the branch is `idea-53-vod-dead-item` and review the diff before integrating (see finishing-a-development-branch).

## Notes / deliberate boundaries

- **No migration** — reuses `consecutive_failures`, `is_active`, `failure_reason`.
- **Admin "Test" button does not disable** — it is a single manual diagnostic (`probe`/`ProbeTarget::PlaylistItem` keeps using `run_check` with `is_active = None`). Only the health loop and the tune path enforce the threshold.
- **R2-mp4 dead items** are disabled by the health loop (≤15 min) and skipped immediately client-side; the tune-path counting (Task 4) mainly accelerates youtube-VOD items whose resolve fails. A client→server "playback failed" report endpoint was considered and rejected (YAGNI).
- **No auto-re-enable** — recovery resets failures/status; re-enabling is a manual admin toggle.

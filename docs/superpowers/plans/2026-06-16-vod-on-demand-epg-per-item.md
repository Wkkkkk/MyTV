# Per-item EPG blocks for VOD-on-demand — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the guide, render each active VOD-on-demand playlist item as its own clickable block (fixed-width, clipped to the window edge) that tunes the channel and jumps straight to that item.

**Architecture:** On-demand items have no schedule, so positioning is pure presentation and lives in `src/routes/guide/layout.rs` (new `on_demand_slots`), not the time-domain `src/epg.rs`. `ProgramSlot` gains an `item_id: Option<i64>`; the template passes it to a `tune(channelId, itemId)` JS call that reuses the existing `/channel/:id/item/:id` per-item playback path.

**Tech Stack:** Rust / Axum, Askama templates, vanilla JS (HTMX guide), SQLx (SQLite), `cargo test`.

**Spec:** `docs/superpowers/specs/2026-06-16-vod-on-demand-epg-per-item-design.md`

---

## File structure

- `src/routes/guide/layout.rs` — add `ProgramSlot.item_id`, the `ON_DEMAND_ITEM_WIDTH_PCT` constant, and the `on_demand_slots` layout fn + its unit tests. **Responsibility:** translate program data into positioned, clickable guide blocks.
- `src/epg.rs` — remove the now-unused `on_demand_entry` fn and its test. **Responsibility:** time-domain EPG entry computation (live + vod_loop only).
- `src/routes/guide/data.rs` — `VodOnDemand` match arm builds slots via `on_demand_slots`; the match now yields `(Vec<ProgramSlot>, Option<String>)` directly. **Responsibility:** aggregate per-channel guide rows.
- `templates/partials/epg_content.html` — per-item `onclick`/`onkeydown`. **Responsibility:** render the guide grid.
- `templates/base.html` — `tune`/`odTune` accept an optional item id. **Responsibility:** player control JS.
- `tests/http.rs` — integration test for per-item on-demand rendering.

**Note — no CSS task:** `.program-title` already has `white-space:nowrap;overflow:hidden;text-overflow:ellipsis` and `.program` already has `overflow:hidden` (`templates/base.html:108,117`). On-demand blocks reuse the same `.program`/`.program-title` markup, so the "cap title width for on-demand and vod_loop uniformly" requirement is already satisfied. No CSS change is needed.

---

## Task 1: `ProgramSlot.item_id` + `on_demand_slots` layout fn

**Files:**
- Modify: `src/routes/guide/layout.rs`

- [ ] **Step 1: Add `item_id` to the `ProgramSlot` struct**

In `src/routes/guide/layout.rs`, change the struct (currently lines 5–11):

```rust
pub(super) struct ProgramSlot {
    pub title: String,
    pub is_live: bool,
    pub left_pct: f64,
    pub width_pct: f64,
    pub channel_id: i64,
    pub item_id: Option<i64>,
}
```

- [ ] **Step 2: Set `item_id: None` in `entry_to_slot`**

In the existing `entry_to_slot` fn, the `Some(ProgramSlot { ... })` literal (around line 39) must set the new field. Add `item_id: None,` as the last field:

```rust
    Some(ProgramSlot {
        title: entry.title.clone(),
        is_live: entry.is_live,
        left_pct: (left_secs / window_secs * 100.0).clamp(0.0, 100.0),
        width_pct: (width_secs / window_secs * 100.0).clamp(0.0, 100.0),
        channel_id: entry.channel_id,
        item_id: None,
    })
```

- [ ] **Step 3: Add the import and constant**

At the top of `src/routes/guide/layout.rs`, the imports are currently:

```rust
use chrono::{DateTime, Utc};

use crate::epg;
```

Add the `PlaylistItem` import and a module-level constant below the imports:

```rust
use chrono::{DateTime, Utc};

use crate::epg;
use crate::model::playlist_item::PlaylistItem;

/// Fixed width (percent of the guide window) of each VOD-on-demand item block.
/// On-demand items have no schedule, so they are laid out left-to-right at this
/// width and clipped at the window edge (~4 visible). Off-edge items remain
/// reachable via the player's playlist panel.
pub(super) const ON_DEMAND_ITEM_WIDTH_PCT: f64 = 25.0;
```

- [ ] **Step 4: Write the failing unit tests**

Add to the `#[cfg(test)] mod tests` block in `src/routes/guide/layout.rs` (alongside the existing tests). First add a `PlaylistItem` builder helper inside the tests module, then the test fns:

```rust
    fn pl_item(id: i64, title: &str) -> PlaylistItem {
        PlaylistItem {
            id,
            channel_id: 6,
            title: title.to_string(),
            url: format!("https://example.com/{}.mp4", id),
            duration_secs: 120,
            sort_order: id,
            is_active: true,
            last_checked_at: None,
            last_status: None,
            consecutive_failures: 0,
            failure_reason: None,
        }
    }

    #[test]
    fn test_on_demand_slots_empty_returns_single_fallback() {
        let slots = on_demand_slots(6, "On Demand", &[], ON_DEMAND_ITEM_WIDTH_PCT);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].item_id, None);
        assert!(slots[0].title.contains("On Demand"));
        assert!((slots[0].left_pct - 0.0).abs() < 0.01);
        assert!((slots[0].width_pct - 100.0).abs() < 0.01);
        assert!(!slots[0].is_live);
    }

    #[test]
    fn test_on_demand_slots_three_items_evenly_placed() {
        let items = vec![pl_item(1, "A"), pl_item(2, "B"), pl_item(3, "C")];
        let slots = on_demand_slots(6, "On Demand", &items, 25.0);
        assert_eq!(slots.len(), 3);
        assert!((slots[0].left_pct - 0.0).abs() < 0.01);
        assert!((slots[1].left_pct - 25.0).abs() < 0.01);
        assert!((slots[2].left_pct - 50.0).abs() < 0.01);
        for s in &slots {
            assert!((s.width_pct - 25.0).abs() < 0.01);
            assert!(!s.is_live);
        }
        assert_eq!(slots[0].item_id, Some(1));
        assert_eq!(slots[1].item_id, Some(2));
        assert_eq!(slots[2].item_id, Some(3));
        assert_eq!(slots[0].title, "A");
    }

    #[test]
    fn test_on_demand_slots_clips_at_window_edge() {
        // 6 items at 25% -> the 5th would start at left=100 -> stop. 4 visible.
        let items: Vec<PlaylistItem> = (1..=6).map(|i| pl_item(i, "x")).collect();
        let slots = on_demand_slots(6, "On Demand", &items, 25.0);
        assert_eq!(slots.len(), 4);
        for s in &slots {
            assert!(s.left_pct < 100.0);
            assert!(s.left_pct + s.width_pct <= 100.0 + 0.01);
        }
        assert!((slots[3].left_pct - 75.0).abs() < 0.01);
        assert!((slots[3].width_pct - 25.0).abs() < 0.01);
    }

    #[test]
    fn test_on_demand_slots_clamps_last_partial_block() {
        // 4 items at 30% -> left 0/30/60/90; last clamped to width 10.
        let items: Vec<PlaylistItem> = (1..=4).map(|i| pl_item(i, "x")).collect();
        let slots = on_demand_slots(6, "On Demand", &items, 30.0);
        assert_eq!(slots.len(), 4);
        assert!((slots[3].left_pct - 90.0).abs() < 0.01);
        assert!((slots[3].width_pct - 10.0).abs() < 0.01);
    }
```

- [ ] **Step 5: Run the tests to verify they fail**

Run: `cargo test --lib on_demand_slots`
Expected: FAIL — compile error `cannot find function on_demand_slots in this scope`.

- [ ] **Step 6: Implement `on_demand_slots`**

Add the fn to `src/routes/guide/layout.rs` (after `entry_to_slot`, before the tests module):

```rust
/// Lays out VOD-on-demand items as fixed-width clickable blocks, left-to-right,
/// clipped at the window edge. Empty playlist -> one full-width fallback block
/// that tunes the channel (keeps the row tunable from the guide).
pub(super) fn on_demand_slots(
    channel_id: i64,
    name: &str,
    items: &[PlaylistItem],
    width_pct: f64,
) -> Vec<ProgramSlot> {
    if items.is_empty() {
        return vec![ProgramSlot {
            title: format!("{} — On demand", name),
            is_live: false,
            left_pct: 0.0,
            width_pct: 100.0,
            channel_id,
            item_id: None,
        }];
    }

    let mut slots = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let left = i as f64 * width_pct;
        if left >= 100.0 {
            break;
        }
        slots.push(ProgramSlot {
            title: item.title.clone(),
            is_live: false,
            left_pct: left,
            width_pct: width_pct.min(100.0 - left),
            channel_id,
            item_id: Some(item.id),
        });
    }
    slots
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib layout`
Expected: PASS — the four new tests plus all existing `layout` tests pass.

- [ ] **Step 8: Format and commit**

```bash
cargo fmt
git add src/routes/guide/layout.rs
git commit -m "feat(guide): ProgramSlot.item_id + on_demand_slots layout"
```

---

## Task 2: Wire `data.rs` to `on_demand_slots`; retire `on_demand_entry`

**Files:**
- Modify: `src/routes/guide/data.rs`
- Modify: `src/epg.rs`

- [ ] **Step 1: Extend the `layout` import in `data.rs`**

In `src/routes/guide/data.rs`, the import (currently lines 16–18) is:

```rust
use super::layout::{
    compute_window, entry_to_slot, now_line_pct, time_labels, ProgramSlot, TimeLabel,
};
```

Change it to:

```rust
use super::layout::{
    compute_window, entry_to_slot, now_line_pct, on_demand_slots, time_labels,
    ProgramSlot, TimeLabel, ON_DEMAND_ITEM_WIDTH_PCT,
};
```

- [ ] **Step 2: Rework the per-channel match to yield slots directly**

In `build_guide_data`, replace the block that currently starts `let (entries, budget_url) = match ch.channel_type() {` (line 96) and ends with the `programs` collection (line 141) with the following. The `Live` and `VodLoop` arms map their time entries through `entry_to_slot` via a local closure; the `VodOnDemand` arm uses `on_demand_slots`:

```rust
        let to_slots = |entries: Vec<epg::ProgramEntry>| -> Vec<ProgramSlot> {
            entries
                .iter()
                .filter_map(|e| entry_to_slot(e, window_start, window_end))
                .collect()
        };

        let (programs, budget_url): (Vec<ProgramSlot>, Option<String>) = match ch.channel_type() {
            ChannelType::Live => {
                let first_active_url = sources_by_channel
                    .get(&ch.id)
                    .and_then(|v| v.iter().find(|s| s.is_active).map(|s| s.url.clone()));
                (
                    to_slots(vec![epg::live_entry(ch.id, &ch.name, window_start, window_end)]),
                    first_active_url,
                )
            }
            ChannelType::VodLoop => {
                let items = all_playlist_items.get(&ch.id).cloned().unwrap_or_default();
                let entries = match ch.loop_anchor {
                    Some(anchor) => epg::vod_schedule(
                        ch.id,
                        &items,
                        anchor.timestamp(),
                        window_start,
                        window_end,
                    ),
                    None => vec![],
                };
                let budget_url = vod_budget_url(&items, ch.loop_anchor, now);
                (to_slots(entries), budget_url)
            }
            ChannelType::VodOnDemand => {
                let items = all_playlist_items.get(&ch.id).cloned().unwrap_or_default();
                let budget_url = vod_budget_url(&items, None, now);
                let programs =
                    on_demand_slots(ch.id, &ch.name, &items, ON_DEMAND_ITEM_WIDTH_PCT);
                (programs, budget_url)
            }
        };
```

Then **delete** the now-redundant standalone mapping that followed the old match:

```rust
        let programs: Vec<ProgramSlot> = entries
            .iter()
            .filter_map(|e| entry_to_slot(e, window_start, window_end))
            .collect();
```

(The `programs` binding now comes from the match.)

- [ ] **Step 3: Remove `on_demand_entry` and its test from `epg.rs`**

In `src/epg.rs`, delete the entire `on_demand_entry` fn (the doc comment + fn, currently lines 35–54) and delete the test `test_on_demand_entry_spans_window_not_live` (currently lines 139–149).

- [ ] **Step 4: Build and run the full test suite**

Run: `cargo build && cargo test`
Expected: PASS — compiles with no unused-import/unused-fn warnings; all tests pass. (No test references `on_demand_entry` or the "On demand" text — verified.)

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/routes/guide/data.rs src/epg.rs
git commit -m "feat(guide): render per-item slots for vod_on_demand; drop on_demand_entry"
```

---

## Task 3: Per-item `onclick`/`onkeydown` in the template + integration test

**Files:**
- Modify: `templates/partials/epg_content.html`
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing integration test**

Add to `tests/http.rs` (near the other guide tests, e.g. after `test_guide_excludes_inactive_playlist_items`). Seed channel 6 ("On Demand", `vod_on_demand`) has two active items — `点播 First` (id 4) and `On-Demand 2` (id 5):

```rust
#[tokio::test]
async fn test_guide_renders_on_demand_items_as_per_item_blocks() {
    // Channel 6 ("On Demand", vod_on_demand) has two active items in seed.sql.
    // The guide must render one clickable block per item (with its title and a
    // per-item tune(channel, item) handler), not a single "— On demand" block.
    let response = app().await.oneshot(req("/guide")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("点播 First"),
        "guide should render the first on-demand item title"
    );
    assert!(
        body.contains("On-Demand 2"),
        "guide should render the second on-demand item title"
    );
    assert!(
        body.contains("tune(6, 4)"),
        "first on-demand block should tune channel 6 -> item 4"
    );
    assert!(
        body.contains("tune(6, 5)"),
        "second on-demand block should tune channel 6 -> item 5"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test http test_guide_renders_on_demand_items_as_per_item_blocks`
Expected: FAIL — the template still emits `tune(6)` (no item arg), so `tune(6, 4)` is absent. (Item titles may already render after Task 2, but the per-item `tune(...)` assertion fails.)

- [ ] **Step 3: Update the template click handlers**

In `templates/partials/epg_content.html`, the program block (currently lines 51–58) uses `onclick="tune({{ prog.channel_id }})"` and a matching `onkeydown`. Replace both call sites so they append the item id when present (Askama `match` per project convention for `Option`):

```html
        {% for prog in row.programs %}
        <div class="program{% if prog.is_live %} live{% endif %}"
             style="left: {{ prog.left_pct }}%; width: {{ prog.width_pct }}%"
             role="button" tabindex="0"
             onclick="tune({{ prog.channel_id }}{% match prog.item_id %}{% when Some with (id) %}, {{ id }}{% when None %}{% endmatch %})"
             onkeydown="if(event.key==='Enter'||event.key===' '){event.preventDefault();tune({{ prog.channel_id }}{% match prog.item_id %}{% when Some with (id) %}, {{ id }}{% when None %}{% endmatch %})}">
          {% if prog.is_live %}<span class="live-badge">LIVE</span>{% endif %}
          <span class="program-title">{{ prog.title }}</span>
        </div>
        {% endfor %}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test http test_guide_renders_on_demand_items_as_per_item_blocks`
Expected: PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add templates/partials/epg_content.html tests/http.rs
git commit -m "feat(guide): per-item tune() in on-demand guide blocks"
```

---

## Task 4: Frontend `tune`/`odTune` accept an optional item id

**Files:**
- Modify: `templates/base.html`

No JS test harness exists; this task is verified by the Task 3 integration test (template emits the right call) plus manual smoke (Verification section). All existing `tune(...)` callers pass a single argument, so the optional second parameter is backward-compatible.

- [ ] **Step 1: Thread the item id through `tune`**

In `templates/base.html`, change the `tune` fn signature (currently line 613 `function tune(channelId) {`) and the on-demand branch (lines 616–620) to forward an optional `itemId`:

```javascript
      function tune(channelId, itemId) {
        currentChannelId = channelId;
        currentUrl = null;
        if (odChannelType(channelId) === 'vod_on_demand') {
          hidePlayerError();
          odTune(channelId, itemId);
          return;
        }
```

(The rest of `tune` is unchanged.)

- [ ] **Step 2: Honor a start item in `odTune`**

In `templates/base.html`, change `odTune` (currently line 574 `function odTune(channelId) {`). Replace its signature and the cursor-selection block inside the `.then(function(items){...})` (currently lines 584–597) so an explicit `startItemId` overrides the saved cursor:

```javascript
      function odTune(channelId, startItemId) {
        odChannelId = channelId;
        odItems = [];
        odIndex = -1;
        var btn = document.getElementById('ov-playlist');
        if (btn) btn.hidden = false;
        document.getElementById('player-panel').style.display = 'block';
        fetch('/channel/' + channelId + '/playlist')
          .then(function(r) { if (!r.ok) throw new Error('playlist ' + r.status); return r.json(); })
          .then(function(items) {
            odItems = items || [];
            if (!odItems.length) { showPlayerError(); return; }
            var start = 0, offset = 0;
            if (startItemId != null) {
              for (var i = 0; i < odItems.length; i++) {
                if (odItems[i].id === startItemId) { start = i; offset = 0; break; }
              }
            } else {
              var cursor = odLoadCursor(channelId);
              if (cursor) {
                for (var i = 0; i < odItems.length; i++) {
                  if (odItems[i].id === cursor.itemId) { start = i; offset = cursor.offset || 0; break; }
                }
              }
            }
            odRenderList();
            // Open the playlist by default so items are immediately clickable.
            var box = document.getElementById('player-playlist');
            if (box) box.hidden = false;
            odPlayIndex(start, offset);
          })
          .catch(function(err) {
            if (typeof debugLog === 'function') debugLog('error', 'on-demand tune: ' + err);
            showPlayerError();
          });
      }
```

- [ ] **Step 3: Build to confirm the template still compiles**

Run: `cargo build`
Expected: PASS (Askama compiles `base.html`; no template syntax errors).

- [ ] **Step 4: Commit**

```bash
git add templates/base.html
git commit -m "feat(player): tune/odTune accept optional start item id"
```

---

## Task 5: Final verification + docs

**Files:**
- Modify: `docs/IDEAS.md`
- Modify: `docs/CHANGELOG.md`

- [ ] **Step 1: Full gate**

Run: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
Expected: clean format, no clippy warnings, all tests pass.

- [ ] **Step 2: Manual smoke (player JS)**

Run: `cargo run`, open `http://localhost:3000/guide`. Confirm: the "On Demand" channel row shows one block per item with its title; clicking the second item's block opens the player on that item (▶ marker on the right row in the `☰` panel); clicking the channel's first block still starts from the saved cursor / first item.

- [ ] **Step 3: Move idea #52 to done**

In `docs/IDEAS.md`, remove the `52.` entry from the `## Open` section. In `docs/CHANGELOG.md`, add a one-line entry under the appropriate heading describing idea #52 (per-item clickable on-demand guide blocks, fixed-width clipped layout, per-item tune). Update the count in the `## Done` line of `docs/IDEAS.md` if it tallies completed ideas.

- [ ] **Step 4: Commit**

```bash
git add docs/IDEAS.md docs/CHANGELOG.md
git commit -m "docs: mark idea #52 done (per-item on-demand guide blocks)"
```

---

## Self-review notes

- **Spec coverage:** ProgramSlot.item_id (Task 1) · on_demand_slots fixed-width/clip/empty-fallback (Task 1) · retire on_demand_entry (Task 2) · data.rs VodOnDemand arm (Task 2) · template per-item onclick (Task 3) · tune/odTune JS (Task 4) · title cap CSS (already present — documented, no task) · unit + integration tests (Tasks 1, 3) · docs (Task 5). All spec sections mapped.
- **Type consistency:** `on_demand_slots(channel_id, name, items, width_pct)` and `ON_DEMAND_ITEM_WIDTH_PCT` are defined in Task 1 and used with matching names/arity in Task 2. `ProgramSlot.item_id: Option<i64>` is read in the template via `{% match %}` (Task 3). `odTune(channelId, startItemId)` defined and called consistently (Tasks 3 template → 4 JS).
- **No placeholders:** every code step shows full content; commands include expected output.

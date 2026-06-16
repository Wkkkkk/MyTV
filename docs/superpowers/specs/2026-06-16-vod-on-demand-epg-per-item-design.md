# Per-item EPG blocks for VOD-on-demand channels (idea #52)

**Date:** 2026-06-16
**Status:** Design approved, pending implementation plan

## Problem

In the guide, a `vod_loop` channel renders one clickable `.program` block per
item (positioned by schedule time via `epg::vod_schedule`), but a
`vod_on_demand` channel renders a *single* `{name} — On demand` block spanning
the whole window (`epg::on_demand_entry`, `src/routes/guide/data.rs`). Clicking
it only tunes the channel; individual items are reachable solely through the
player's `☰` playlist panel.

We want each active on-demand item to render as its own clickable guide block
showing its title, so a viewer can click straight to a specific item — the same
affordance vod_loop already has. The per-item tune path already exists
(`GET /channel/:id/item/:item_id` → frontend `odPlayIndex`).

## Layout decision

On-demand items have **no time schedule**, so they cannot be positioned
proportionally on the timeline. They are laid out as **fixed-width blocks,
left-to-right, clipped at the window edge**: each item gets a fixed readable
width; blocks beyond the 4h window edge are clipped and remain reachable only
via the player's `☰` panel (which opens by default on tune). No `+N more`
affordance, no horizontal scroll — deliberately excluded.

For consistency, the same title-width cap (ellipsis truncation) is applied to
`vod_loop` item blocks too.

## Design

### 1. Data model — `ProgramSlot`

Add `item_id: Option<i64>` to `ProgramSlot` (`src/routes/guide/layout.rs`):

- `None` for live and vod_loop blocks, and the empty-channel fallback block.
- `Some(item.id)` for on-demand item blocks.

This field is what lets a click target a specific playlist item rather than just
the channel.

### 2. Layout — new `on_demand_slots`

On-demand positioning is pure presentation (no time), so it lives in
`layout.rs`, not the time-domain `epg.rs`:

```rust
pub(super) fn on_demand_slots(
    channel_id: i64,
    name: &str,
    items: &[PlaylistItem],
    width_pct: f64,
) -> Vec<ProgramSlot>
```

Behavior:

- **Empty `items`** → one full-width slot (`left_pct: 0`, `width_pct: 100`),
  `title: "{name} — On demand"`, `item_id: None`. Keeps an empty/misconfigured
  on-demand channel tunable from the guide (preserves today's behavior).
- **Non-empty** → one slot per item at `left_pct = i * width_pct`,
  `width_pct = width_pct`. Stop emitting once `left_pct >= 100`. Clamp the last
  visible slot so `left_pct + width_pct <= 100`. Each slot carries the item's
  `title` and `item_id: Some(item.id)`. `is_live: false`.

`width_pct` comes from a named constant `ON_DEMAND_ITEM_WIDTH_PCT = 25.0`
(≈4 items visible), defined in `layout.rs` so it is tunable in one place.

### 3. `src/epg.rs`

Retire `on_demand_entry` and its test (`test_on_demand_entry_spans_window_not_live`)
— replaced by `layout::on_demand_slots`. `live_entry` and `vod_schedule` are
unchanged.

### 4. `src/routes/guide/data.rs`

The `ChannelType::VodOnDemand` arm builds its `programs: Vec<ProgramSlot>`
directly via `layout::on_demand_slots(ch.id, &ch.name, &items, ON_DEMAND_ITEM_WIDTH_PCT)`,
bypassing `entry_to_slot` (which is time-based). The `budget_url` computation
(`vod_budget_url(&items, None, now)`) is unchanged.

The `Live` and `VodLoop` arms keep producing time `ProgramEntry`s mapped through
`entry_to_slot`; those slots now set `item_id: None`.

### 5. Template — `templates/partials/epg_content.html`

The program block's click handlers pass the item id when present:

```
onclick="tune({{ prog.channel_id }}{% match prog.item_id %}{% when Some with (id) %}, {{ id }}{% when None %}{% endmatch %})"
```

(and the matching `onkeydown` Enter/Space handler).

### 6. Frontend — `templates/base.html`

- `tune(channelId, itemId)` gains an optional `itemId`. When the channel is
  `vod_on_demand` and `itemId` is provided, call `odTune(channelId, itemId)`.
  Channel-name / no-item clicks keep current cursor behavior.
- `odTune(channelId, startItemId)` gains an optional `startItemId`: after
  fetching the playlist, if `startItemId` matches an item, start at that index
  (`odPlayIndex(idx, 0)`) instead of the saved-cursor item; otherwise unchanged.

No backend route changes — reuses the existing `/channel/:id/item/:item_id`
handler.

### 7. Title cap — CSS in `templates/base.html`

Add to `.program-title`: `overflow: hidden; text-overflow: ellipsis;
white-space: nowrap; max-width: 100%`. Applied to all blocks, so vod_loop titles
truncate uniformly. No server-side string truncation.

## Testing

- **Unit (`layout.rs`):**
  - empty items → single fallback slot, `item_id == None`, title contains the
    channel name, full width.
  - 3 items → 3 slots at left 0/25/50, each `width_pct == 25`, correct
    `item_id`/`title`, `is_live == false`.
  - many items (e.g. 6 at 25%) → slots clipped: none with `left_pct >= 100`, the
    last visible slot clamped so `left_pct + width_pct <= 100`.
- **Integration (`tests/http.rs`):**
  - `/guide/partial` for an on-demand channel with N active items renders one
    block per visible item with a per-item `onclick="tune(<chan>, <item>)"`.
  - an on-demand channel with no active items renders the single fallback block.
- **Frontend JS** change verified manually (no JS test harness). The
  `/channel/:id/item/:id` handler it drives already has integration coverage.

## Scope guards (YAGNI)

Excluded by the layout decision: `+N more` overflow affordance, horizontal
scroll, any schedule/time semantics for on-demand items, new backend routes.

## Touched files

- `src/routes/guide/layout.rs` — `ProgramSlot.item_id`, `on_demand_slots`, const.
- `src/epg.rs` — remove `on_demand_entry` + its test.
- `src/routes/guide/data.rs` — VodOnDemand arm uses `on_demand_slots`.
- `templates/partials/epg_content.html` — per-item `onclick`/`onkeydown`.
- `templates/base.html` — `tune`/`odTune` optional item id; `.program-title` CSS.
- `docs/IDEAS.md` / `docs/CHANGELOG.md` — move idea #52 to done on completion.

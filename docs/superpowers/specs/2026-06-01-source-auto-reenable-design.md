# Source Auto-Re-enable Design

**Date:** 2026-06-01
**Status:** approved

## Problem

Once a source is auto-disabled by the health checker (after 3 consecutive failures), it stays disabled forever. Recovery requires manual action via the admin toggle. This means a source that was temporarily down — a CDN hiccup, a brief server restart — never comes back without operator intervention.

## Goal

When the health checker runs a check on a disabled source and it passes, automatically re-enable that source and log the event.

## Constraints

- No schema changes — `is_active` already exists on `sources`
- No new timer or separate recovery loop — piggyback on the existing 15-minute check cycle
- `list_all` already fetches all sources (active and inactive), so disabled sources are already being checked; we just do nothing useful with a successful result today

## Design

### `HealthAction` enum (`health.rs`, private)

Replace the `set_inactive: bool` return from `process_result` with an enum:

```rust
enum HealthAction {
    Disable,
    Reenable,
    None,
}
```

This makes the two mutually exclusive outcomes explicit and readable in `match` arms.

### `process_result` (`health.rs`)

New signature: `fn process_result(src: &Source, ok: bool) -> (i64, HealthAction)`

Logic:

| Condition | Action |
|-----------|--------|
| `ok && !src.is_active` | `Reenable` |
| `!ok && failures >= 3 && src.is_active` | `Disable` |
| everything else | `None` |

### `check_one` (`health.rs`)

Matches on the action to produce the `is_active: Option<bool>` passed to `update_health`, and emits the appropriate log:

- `Disable` → `Some(false)`, `tracing::warn!("source N auto-disabled after N consecutive failures")`
- `Reenable` → `Some(true)`, `tracing::info!("source N auto-re-enabled after passing health check")`
- `None` → `None`

### `update_health` (`src/model/source.rs`)

Replace `set_inactive: bool` parameter with `is_active: Option<bool>`:

- `None` — do not include `is_active` in the UPDATE
- `Some(v)` — set `is_active = v`

This collapses the two SQL branches into one. The enum stays in `health.rs`; the model layer only sees a plain `Option<bool>`, avoiding any circular dependency.

## Tests

### `process_result` unit tests (`health.rs`)

Update existing tests for the new return type. Add two new cases:

| Case | Input | Expected action |
|------|-------|-----------------|
| Inactive source passes check | `is_active=false, ok=true` | `Reenable`, `failures=0` |
| Active source passes check | `is_active=true, ok=true` | `None`, `failures=0` |

### `update_health` model tests (`source.rs`)

Add one new case: call `update_health` with `is_active=Some(true)` on a disabled source and assert `is_active` becomes `true`.

## Documentation update

`docs/architecture/health-checker.md`:

1. **Flowchart** — add branch from `process_result`: `ok && is_active=false → set is_active = 1`
2. **State machine** — change `Disabled → Disabled : check runs — no state change` to `Disabled → Active : check ok — auto re-enabled`
3. **Notes** — replace the "No auto-re-enable" paragraph with a description of the new behavior

## Files changed

| File | Change |
|------|--------|
| `src/health.rs` | Add `HealthAction` enum; update `process_result` return type; update `check_one` |
| `src/model/source.rs` | Change `update_health` signature; update SQL; update tests |
| `docs/architecture/health-checker.md` | Update flowchart, state machine, notes |

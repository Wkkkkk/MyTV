# Idea #49 — One coercion source of truth for intake DTOs — Design

**Status:** Approved (design)
**Date:** 2026-06-16
**Backlog:** idea #49 (`docs/IDEAS.md`), architecture-deepening round #47–#50

## Problem

The 2026-06-12 intake-validation work centralized *validation* into
`*Input::validate_new/validate_update` (`src/model/{channel,source,playlist_item}.rs`),
but *coercive parsing of numeric fields* stayed in the route handlers and now differs
between the HTML-form admin path (`src/routes/admin/*`) and the JSON-API path
(`src/routes/api/*`):

- **`channel.sort_order`** — form: `parse_sort_order` (trim, blank→0, garbage→422);
  API: typed `i64` via serde (required, 422 on bad/missing).
- **`source.priority`** — form: `form.priority.trim().parse().unwrap_or(1)`
  (garbage **silently → 1**); API: `req.priority.unwrap_or(1)`. The default `1` is
  hardcoded in two places.
- **`playlist_item.sort_order`** — form derives from the DB max, then `.unwrap_or(0)`;
  API: `req.sort_order.unwrap_or(0)`. The default `0` is hardcoded in two places.
- **Redundant trim** — `admin/playlist.rs` does `form.url.trim()` even though the DTO's
  `validate_title_url` already trims url.

Two consequences: (1) the default literals (`0`, `1`) can drift between doors, and a
test of one door says nothing about the other; (2) the *form* path is internally
inconsistent on bad input — `parse_sort_order("abc")` → 422 but
`priority.parse().unwrap_or(1)` → silently 1.

**Already unified (not in scope):** enum parsing (`channel_type`, `source_kind`) and
name/category/url trimming already live in `validate_*`. Numeric coercion is the gap.

## Approach (B — de-duplicate, keep the typed JSON contract)

Keep the JSON API typed: serde staying strict (422 on a malformed number) is the correct
behavior for a typed contract, and `mytvctl`/e2e clients depend on it. Do **not** make
the JSON path lenient and do **not** change DTO field types. Instead, extract the
scattered default literals and parse-with-default logic into one shared, unit-tested
model-layer coercion layer that *both* doors call.

Rejected alternatives:
- **A — string-typed `*Input`, DTO owns all parsing.** Full behavioral parity, but
  changes the JSON contract (`mytvctl`/e2e send string numbers), loses serde's free
  type-checking, and the auto-fetch-duration network call still can't live in a pure DTO.
- **C — two-layer `*RawInput → coerce() → *Input`.** Adds a third struct per entity,
  pushing toward the very triplication idea #50 wants to reduce. YAGNI.

## Design

### 1. Shared coercion layer in `src/model/mod.rs`

Add next to `IntakeError` (the existing shared intake home):

```rust
/// Default source priority when the intake field is blank/absent.
pub const DEFAULT_PRIORITY: i64 = 1;
/// Default sort order when the intake field is blank/absent.
pub const DEFAULT_SORT_ORDER: i64 = 0;

/// Coerce a form numeric field to i64: trimmed-blank/absent → `default`;
/// present-but-unparseable → `IntakeError` (strict — surfaced as 422 by the adapter).
pub fn coerce_i64(raw: &str, default: i64) -> Result<i64, IntakeError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(default)
    } else {
        trimmed
            .parse()
            .map_err(|_| IntakeError(format!("expected an integer, got {trimmed:?}")))
    }
}
```

This is the single definition of the two defaults and the one parse-with-default rule.

### 2. Handler changes

All four doors point at the shared layer; the default literals appear once.

- `src/routes/admin/channels.rs`: delete the local `parse_sort_order` fn; both
  create/edit handlers call `model::coerce_i64(&form.sort_order, model::DEFAULT_SORT_ORDER)?`.
  Map the `IntakeError` to `422` the same way the existing intake errors are mapped
  (the handler already returns `StatusCode`; convert via the existing pattern —
  `.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?`).
- `src/routes/admin/sources.rs`: replace `form.priority.trim().parse().unwrap_or(1)`
  with `model::coerce_i64(&form.priority, model::DEFAULT_PRIORITY)?`.
- `src/routes/api/sources.rs`: `req.priority.unwrap_or(model::DEFAULT_PRIORITY)`.
- `src/routes/api/playlist.rs`: `req.sort_order.unwrap_or(model::DEFAULT_SORT_ORDER)`.
- `src/routes/admin/playlist.rs`: drop the redundant `form.url.trim()` — pass the raw
  url to the DTO, which trims it. **Duration auto-fetch stays in this handler** (it does
  network I/O via `media::fetch_duration`; not pure coercion).

### Behavior decision: strict on bad numeric input

`coerce_i64` returns `Err` (→ 422) on a present-but-unparseable value. This matches the
existing `parse_sort_order` and the JSON serde path, so all four code paths finally
agree. The one behavior change: form `source.priority="abc"` now 422s instead of
silently becoming `1`.

## Testing (TDD)

- **Unit (`src/model/mod.rs` `#[cfg(test)]`):** `coerce_i64("5", 0)` → `Ok(5)`;
  `coerce_i64("", 7)` / `coerce_i64("  ", 7)` → `Ok(7)`; `coerce_i64("abc", 0)` → `Err`.
- **Constants wired:** assert `DEFAULT_PRIORITY == 1`, `DEFAULT_SORT_ORDER == 0` (guards
  against an accidental change drifting the contract silently).
- **New behavior guard (`tests/http.rs`):** `POST /admin/sources` with `priority=abc`
  → `422 UNPROCESSABLE_ENTITY`.
- **Regression:** existing `tests/http.rs` + `tests/api.rs` stay green — blank form
  `sort_order` → 0, omitted API `priority` → 1 unchanged.

## Out of scope

- The JSON API contract (field types stay typed; serde stays strict).
- Auto-fetch-duration (stays in `admin/playlist.rs`).
- Playlist `sort_order` *semantics* (form derives from DB max, API takes from request) —
  a legitimate behavioral difference, not coercion drift.
- Model-layer CRUD triplication (idea #50, separate spec).

## Verification gate

`cargo fmt --check` → `cargo clippy --all-targets -- -D warnings` → `cargo test`
(suite stays at its current count plus the new `coerce_i64` unit tests and the one
`priority=abc` integration test).

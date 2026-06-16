# Idea #48 — Explicit source-availability classification + threshold unification

*Design — 2026-06-16. Architecture deepening (rated "Strong"). Supersedes the
literal framing of idea #48 after a brainstorming pass corrected its premise.*

## Premise correction

Idea #48 as originally written said: add a `HealthAction { Disable, Reenable,
None }` enum, "translate the variant into the right `update_health` call," and
"realign code with its own docs" — taking `docs/architecture/health-checker.md`
as the source of truth.

That premise is stale. The code has **deliberately moved away** from health ever
mutating `is_active`:

- Every production `update_health` call from `health.rs` passes `is_active =
  None` (lines 116, 134, 153, 198, 219). The `Some(true/false)` calls exist only
  in model-layer tests.
- `record_source_liveness` and `check_source` both carry the comment *"Never
  changes `is_active` — manual intent is the source of truth."*
- "Down" is no longer a stored flag. It is **computed at tune time** by the SQL
  predicate in `list_tunable_for_channel` (`src/model/source.rs:144`):
  `NOT (kind != 'youtube_live' AND last_status = 'error' AND consecutive_failures >= 3)`.

So `health-checker.md` is the stale artifact, not the code. Its title
("auto-disables sources… automatically re-enabled") and its
`HealthAction::{Disable,Reenable}` / `process_result` narrative all describe
**removed behavior**. Implementing #48 literally would *re-introduce* a
regression.

**Decision:** realign to the code, not the doc. Keep the "manual intent is the
source of truth" model. Do the genuinely valuable parts of #48 — kill the
threshold-skew hazard and make the tune-time classification explicit, pure, and
truth-table testable — and rewrite the stale doc to match.

## The real hazard

The failure threshold `3` lives in three disconnected places:

| Location | Form | Scope |
|----------|------|-------|
| `src/health.rs:14` | `const FAILURE_THRESHOLD: i64 = 3` | `#[cfg(test)]`-gated — test-only! |
| `src/model/source.rs:144` | hand-typed `consecutive_failures >= 3` | **production** tune SQL |
| `src/model/source.rs:312` | `const FAILURE_DOWN_THRESHOLD: i64 = 3` | source tests |

Bump one and the system silently skews, exactly as the idea warns. The
production rule is expressed only as a literal inside a SQL string.

## Design

### 1. One threshold constant

```rust
// src/model/source.rs
pub const FAILURE_THRESHOLD: i64 = 3;
```

The model layer owns the "Down" concept, so the constant lives there. Delete the
`#[cfg(test)]` `FAILURE_THRESHOLD` in `health.rs` and `FAILURE_DOWN_THRESHOLD` in
`source.rs` tests; both reference `source::FAILURE_THRESHOLD`.

### 2. One pure Down predicate

```rust
/// A source the tune path skips: a non-`youtube_live` source observed in error
/// past the failure threshold. `youtube_live` stays in rotation so resolve-time
/// waiting/backoff (idea #38) can fire. `is_active` is the manual gate and is
/// handled separately by `list_active_for_channel`.
pub fn is_observed_down(
    kind: &str,
    last_status: Option<&str>,
    consecutive_failures: i64,
) -> bool {
    kind != SourceKind::YoutubeLive.as_str()
        && last_status == Some("error")
        && consecutive_failures >= FAILURE_THRESHOLD
}
```

Exhaustive truth-table unit tests:

- `youtube_live` → never down (even at error past threshold).
- `last_status` of `"ok"` or `None` → not down.
- non-yt + error, `consecutive_failures < FAILURE_THRESHOLD` → not down.
- non-yt + error, `consecutive_failures == FAILURE_THRESHOLD` → down (boundary).
- non-yt + error, `consecutive_failures > FAILURE_THRESHOLD` → down.

### 3. `list_tunable_for_channel` filters in Rust

Reuse the existing `list_active_for_channel`, then filter:

```rust
pub async fn list_tunable_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    Ok(list_active_for_channel(pool, channel_id)
        .await?
        .into_iter()
        .filter(|s| !is_observed_down(&s.kind, s.last_status.as_deref(), s.consecutive_failures))
        .collect())
}
```

The bespoke SQL `AND NOT (...)` predicate is **deleted** — `is_observed_down`
becomes the single source of truth, with zero possibility of Rust/SQL skew.

Cost: filtering 1–3 sources per channel in memory. `list_tunable_for_channel`
has exactly one production caller (`src/routes/player.rs:206`); it is a
per-channel query returning a handful of rows, not a hot loop — no performance
concern. The existing integration test
(`test_list_tunable_skips_down_regular_keeps_youtube_and_disabled_excluded`)
stays green and proves behavior is unchanged.

### 4. Doc rewrite — `docs/architecture/health-checker.md`

Rewrite to the real model:

- The background checker records `last_status`/`consecutive_failures` and
  **never mutates `is_active`**.
- "Down" is computed at *tune time* by `is_observed_down`, mirrored in
  `list_tunable_for_channel`.
- `is_active` is purely the manual admin gate.
- `youtube_live` is exempt from Down (resolve-time waiting/backoff lane).

Remove the `HealthAction` / `process_result` auto-disable/re-enable narrative
(notes §"Auto-re-enable") and the obsolete state-machine diagram. Replace with
the failure-counting + tune-time-filter model. Update the title/intro to drop
"auto-disables… automatically re-enabled."

### 5. Backlog bookkeeping

- Mark #48 done in `docs/IDEAS.md`; move to `docs/CHANGELOG.md` with rationale,
  including the corrected framing (doc was stale, not code).
- In #53, note that the shared `FAILURE_THRESHOLD` it references now exists.

## Testing

TDD: write `is_observed_down` truth-table tests first (red), implement, then
refactor `list_tunable_for_channel` while keeping the existing integration test
green. Run `cargo fmt`, `cargo clippy -- -D warnings`, and the full `cargo test`
before committing.

## Out of scope

- Playlist-item auto-cleanup / removal (idea #53).
- Any `HealthAction` enum or health-driven `is_active` mutation.
- Any change to *what* the background checker probes or how it counts failures
  (`process_failures` is already pure and tested; it stays as-is).

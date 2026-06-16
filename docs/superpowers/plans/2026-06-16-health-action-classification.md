# Idea #48 — Source-Availability Classification + Threshold Unification — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the failure-threshold `3` (currently in three disconnected places) into one shared constant and one pure, truth-table-testable predicate, delete the bespoke SQL Down-predicate, and rewrite the stale health-checker doc to match the code's actual "manual intent is source of truth" model.

**Architecture:** The model layer (`src/model/source.rs`) gains `pub const FAILURE_THRESHOLD: i64 = 3` and a pure `is_observed_down(kind, last_status, consecutive_failures) -> bool`. `list_tunable_for_channel` stops embedding the rule in SQL — it reuses `list_active_for_channel` and filters in Rust via `is_observed_down`, making that function the single source of truth. No health-driven `is_active` mutation is added (the doc, not the code, was stale).

**Tech Stack:** Rust 1.96, SQLx 0.7 (runtime `query_as`), Tokio. Tests are `#[tokio::test]` / `#[test]` in-module. `cargo fmt` + `cargo clippy -- -D warnings` + `cargo test` gate every commit.

**Spec:** `docs/superpowers/specs/2026-06-16-health-action-classification-design.md`

---

### Task 1: Pure `is_observed_down` predicate + shared `FAILURE_THRESHOLD`

**Files:**
- Modify: `src/model/source.rs` (add const + fn near the `list_tunable_for_channel` region, ~line 134; add tests in the `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing truth-table test**

Add to the `tests` module in `src/model/source.rs` (these test a pure fn, no DB):

```rust
#[test]
fn test_is_observed_down_truth_table() {
    let t = FAILURE_THRESHOLD;
    let yt = SourceKind::YoutubeLive.as_str();
    let hls = SourceKind::Hls.as_str();

    // youtube_live is never Down, even errored past threshold
    assert!(!is_observed_down(yt, Some("error"), t + 5));
    // ok / null status → never Down
    assert!(!is_observed_down(hls, Some("ok"), t + 5));
    assert!(!is_observed_down(hls, None, t + 5));
    // non-yt errored but BELOW threshold → not Down
    assert!(!is_observed_down(hls, Some("error"), t - 1));
    // boundary: exactly at threshold → Down
    assert!(is_observed_down(hls, Some("error"), t));
    // above threshold → Down
    assert!(is_observed_down(hls, Some("error"), t + 1));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib model::source::tests::test_is_observed_down_truth_table`
Expected: FAIL — `cannot find value FAILURE_THRESHOLD` / `cannot find function is_observed_down`.

- [ ] **Step 3: Add the constant and predicate**

Insert into `src/model/source.rs` immediately before `list_tunable_for_channel` (currently ~line 135, after `list_active_for_channel`):

```rust
/// Consecutive failed health checks before a non-`youtube_live` source is
/// considered Down and excluded from the tune path. The single source of truth
/// for this threshold — `is_observed_down` is the only consumer.
pub const FAILURE_THRESHOLD: i64 = 3;

/// Whether a source is currently "Down": excluded from the tune path because it
/// has failed health checks past `FAILURE_THRESHOLD`. `youtube_live` sources are
/// exempt — they stay in rotation so resolve-time waiting/backoff (idea #38) can
/// fire. `is_active` is the *manual* gate and is handled separately (by
/// `list_active_for_channel`); health never mutates it.
pub fn is_observed_down(kind: &str, last_status: Option<&str>, consecutive_failures: i64) -> bool {
    kind != SourceKind::YoutubeLive.as_str()
        && last_status == Some("error")
        && consecutive_failures >= FAILURE_THRESHOLD
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib model::source::tests::test_is_observed_down_truth_table`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/model/source.rs
git commit -m "feat(source): pure is_observed_down predicate + shared FAILURE_THRESHOLD

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `list_tunable_for_channel` filters in Rust (delete the SQL predicate)

**Files:**
- Modify: `src/model/source.rs:135-151` (the `list_tunable_for_channel` fn) and its doc comment
- Modify: `src/model/source.rs:312` (replace local `FAILURE_DOWN_THRESHOLD` with the shared const in the existing test)

- [ ] **Step 1: Point the existing integration test at the shared constant**

This existing test is the behavior guard — it must stay green and now reference the one true constant. In `src/model/source.rs`, delete the line:

```rust
    const FAILURE_DOWN_THRESHOLD: i64 = 3;
```

and replace the two `FAILURE_DOWN_THRESHOLD` uses inside `test_list_tunable_skips_down_regular_keeps_youtube_and_disabled_excluded` (currently ~lines 332 and 361) with `FAILURE_THRESHOLD`.

- [ ] **Step 2: Run the guard test to confirm it still passes against the OLD SQL impl**

Run: `cargo test --lib model::source::tests::test_list_tunable_skips_down_regular_keeps_youtube_and_disabled_excluded`
Expected: PASS (rename only — behavior unchanged; the SQL still has the literal `>= 3`).

- [ ] **Step 3: Rewrite `list_tunable_for_channel` to filter in Rust**

Replace the whole function (currently `src/model/source.rs:135-151`) — doc comment included — with:

```rust
/// Sources the tune path may try, ordered by priority: active and not
/// observed-Down. "Down" is computed in Rust by `is_observed_down` (the single
/// source of truth), not in SQL — a non-`youtube_live` source is Down once
/// `last_status='error'` and `consecutive_failures >= FAILURE_THRESHOLD`.
/// `is_active` is the manual gate and is never mutated by health.
pub async fn list_tunable_for_channel(pool: &SqlitePool, channel_id: i64) -> Result<Vec<Source>> {
    Ok(list_active_for_channel(pool, channel_id)
        .await?
        .into_iter()
        .filter(|s| !is_observed_down(&s.kind, s.last_status.as_deref(), s.consecutive_failures))
        .collect())
}
```

`list_active_for_channel` already orders by `priority ASC`, so ordering is preserved.

- [ ] **Step 4: Run the guard test + full source-model tests to verify behavior is unchanged**

Run: `cargo test --lib model::source`
Expected: PASS — including `test_list_tunable_skips_down_regular_keeps_youtube_and_disabled_excluded` (down regular skipped; below-threshold + youtube_live kept; disabled excluded).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/model/source.rs
git commit -m "refactor(source): compute Down in Rust via is_observed_down, drop SQL predicate

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Remove the test-only `FAILURE_THRESHOLD` in `health.rs`

**Files:**
- Modify: `src/health.rs:13-14` (delete the `#[cfg(test)] const FAILURE_THRESHOLD`)
- Modify: `src/health.rs:720,725` (the `test_record_source_liveness_never_changes_is_active` test references it)

- [ ] **Step 1: Delete the test-only constant**

In `src/health.rs`, remove these two lines (currently 13-14):

```rust
#[cfg(test)]
const FAILURE_THRESHOLD: i64 = 3;
```

- [ ] **Step 2: Point the health test at the shared constant**

The deletion breaks `test_record_source_liveness_never_changes_is_active` (uses `FAILURE_THRESHOLD` at lines ~720 and ~725). That test is in `mod tests`, which already has `use super::*;`. Add an explicit import of the model constant at the top of the test module body — find `mod tests {` (line 366) and its `use super::*;` (line 368), and add directly after it:

```rust
    use crate::model::source::FAILURE_THRESHOLD;
```

The two existing uses (`for _ in 0..(FAILURE_THRESHOLD + 2)` and `assert_eq!(src.consecutive_failures, FAILURE_THRESHOLD + 2)`) stay unchanged.

- [ ] **Step 3: Run the health test to verify it compiles and passes**

Run: `cargo test --lib health::tests::test_record_source_liveness_never_changes_is_active`
Expected: PASS.

- [ ] **Step 4: Verify no stray references remain**

Run: `grep -rn "FAILURE_DOWN_THRESHOLD\|#\[cfg(test)\]\s*$" src/health.rs; grep -rn "FAILURE_THRESHOLD" src/`
Expected: the only `FAILURE_THRESHOLD` definition is `pub const` in `src/model/source.rs`; all other hits are `source::FAILURE_THRESHOLD` references (health test) or `FAILURE_THRESHOLD` inside `source.rs`. No `FAILURE_DOWN_THRESHOLD` anywhere.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/health.rs
git commit -m "refactor(health): drop test-only FAILURE_THRESHOLD, use the shared model const

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Rewrite the stale `health-checker.md` doc

**Files:**
- Modify: `docs/architecture/health-checker.md` (full rewrite)

No tests — documentation only. Verify by reading.

- [ ] **Step 1: Replace the document with the corrected model**

Overwrite `docs/architecture/health-checker.md` with content describing the *actual* behavior. The rewrite MUST:

1. **Title/intro:** drop "auto-disables sources… automatically re-enabled". Instead: "A background Tokio task checks every source/playlist URL every 15 minutes and records `last_status`/`consecutive_failures`. It never changes `is_active` — that flag is the admin's manual gate. Whether a source is *tunable* is computed at tune time."
2. **Tick loop diagram:** keep the spawn / first-tick / 15-min `MissedTickBehavior::Skip` / `list_all` / HTTP-or-chunk / youtube_live-shortcut / CORS-probe structure, but replace the `process_result → reset/reenable/inc/disable → set is_active` branch with: `process_failures → failures = 0 (ok) | failures++ (fail)` then `update_health (is_active untouched)`.
3. **Replace the "Source State Machine" `stateDiagram`** (which shows `Active --> Disabled` auto-transitions) with a "Tune-time availability" section: a source is **Tunable** iff `is_active = 1` AND NOT `is_observed_down`; `is_observed_down` = non-`youtube_live` AND `last_status='error'` AND `consecutive_failures >= FAILURE_THRESHOLD` (`src/model/source.rs`); `youtube_live` is exempt (resolve-time waiting/backoff, idea #38). `is_active` changes only via admin toggle or the ended-live→VOD `deactivate_all_for_channel` flip.
4. **Delete the "Auto-re-enable" note** (the `process_result`/`HealthAction::{Disable,Reenable,None}` paragraph) entirely. There is no `HealthAction` enum and no auto-disable/re-enable.
5. **Keep** the accurate notes: `HealthClients`, CORS probing, `probe` vs `check_source` (both pass `is_active = None`), `MissedTickBehavior::Skip`, first-tick-consumed, youtube_live shortcut. Adjust the `probe` vs `check_source` note so it no longer claims `check_source` "is the only path that can auto-disable or auto-re-enable sources" — neither path changes `is_active`; the difference is now only that `probe` additionally warms the CORS cache for the manual Test button.

- [ ] **Step 2: Verify the doc no longer describes removed behavior**

Run: `grep -ni "auto-disable\|auto-re-enable\|re-enabled\|HealthAction\|process_result\|set is_active" docs/architecture/health-checker.md`
Expected: no matches (or only matches that explicitly say the behavior does NOT exist).

- [ ] **Step 3: Commit**

```bash
git add docs/architecture/health-checker.md
git commit -m "docs(health): rewrite health-checker doc to match the code (no is_active mutation)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Backlog bookkeeping (IDEAS.md / CHANGELOG.md)

**Files:**
- Modify: `docs/IDEAS.md` (remove #48 from Open; bump the Done count; touch the #53 cross-ref)
- Modify: `docs/CHANGELOG.md` (add the #48 entry)

- [ ] **Step 1: Inspect the CHANGELOG format**

Run: `sed -n '1,40p' docs/CHANGELOG.md`
Expected: see the existing entry format (heading style, numbering) to match it.

- [ ] **Step 2: Add the #48 CHANGELOG entry**

Append a #48 entry to `docs/CHANGELOG.md` following the existing format. Content to convey:

> **#48 — Source-availability classification + threshold unification.** Unified the failure threshold (`3`) — previously a test-only `FAILURE_THRESHOLD` in `health.rs`, a hand-typed `>= 3` in the tune SQL, and a `FAILURE_DOWN_THRESHOLD` in tests — into one `pub const FAILURE_THRESHOLD` in `model/source.rs`. Introduced a pure, truth-table-tested `is_observed_down` predicate and made `list_tunable_for_channel` filter in Rust, deleting the bespoke SQL Down-predicate so there is one source of truth and no skew. **Premise correction:** the original idea read `health-checker.md` as truth and proposed re-adding a `HealthAction` enum that flips `is_active`; investigation showed the code had deliberately moved to "manual intent is the source of truth" (health never mutates `is_active`; Down is computed at tune time). So the doc was the stale artifact — rewritten to match the code rather than the reverse.

- [ ] **Step 3: Remove #48 from `docs/IDEAS.md` Open and update counts/cross-ref**

In `docs/IDEAS.md`:
- Delete the entire `48.` block (lines ~25).
- Update the Done line (`See CHANGELOG.md — 45 completed ideas …`) to `46 completed ideas` and extend the range note to include #48.
- In the `53.` block, change "Interacts with #48 (the shared `FAILURE_THRESHOLD` / `HealthAction` work)." to "Interacts with #48 (now done) — the shared `FAILURE_THRESHOLD` / `is_observed_down` it referenced now exists in `model/source.rs`; there is no `HealthAction` enum (health never mutates `is_active`)."

- [ ] **Step 4: Commit**

```bash
git add docs/IDEAS.md docs/CHANGELOG.md
git commit -m "docs: mark idea #48 done (threshold unification + Down classification)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Full verification gate

**Files:** none (verification only)

- [ ] **Step 1: Format check**

Run: `cargo fmt --check`
Expected: no output (clean). If it errors, run `cargo fmt` and amend the relevant commit.

- [ ] **Step 2: Clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings/errors.

- [ ] **Step 3: Full test suite**

Run: `cargo test`
Expected: all tests pass (the suite was 399 tests; this change keeps the same count plus the new `test_is_observed_down_truth_table`, and removes none).

- [ ] **Step 4: Confirm the single-source-of-truth invariant**

Run: `grep -rn "consecutive_failures >= \|>= 3\|FAILURE_THRESHOLD\|FAILURE_DOWN_THRESHOLD" src/`
Expected: the threshold appears only as `pub const FAILURE_THRESHOLD: i64 = 3` (definition) and `consecutive_failures >= FAILURE_THRESHOLD` inside `is_observed_down`, plus `source::FAILURE_THRESHOLD` / `FAILURE_THRESHOLD` references in tests. No literal `>= 3` in any SQL string; no `FAILURE_DOWN_THRESHOLD`.

---

## Self-Review

**Spec coverage:**
- Spec §1 (one threshold constant) → Task 1 (define) + Task 3 (remove health.rs dup) + Task 2 step 1 (remove test dup). ✓
- Spec §2 (pure Down predicate + truth-table tests) → Task 1. ✓
- Spec §3 (`list_tunable_for_channel` filters in Rust, SQL predicate deleted) → Task 2. ✓
- Spec §4 (doc rewrite) → Task 4. ✓
- Spec §5 (backlog bookkeeping) → Task 5. ✓
- Spec "Testing" (TDD, fmt/clippy/test gate) → per-task TDD + Task 6. ✓
- Spec "Out of scope" (no HealthAction, no is_active mutation, process_failures untouched) → honored; no task adds these. ✓

**Placeholder scan:** No TBD/TODO; every code step shows complete code; doc-rewrite task (4) enumerates exact required changes rather than "update appropriately". ✓

**Type consistency:** `is_observed_down(kind: &str, last_status: Option<&str>, consecutive_failures: i64) -> bool` and `FAILURE_THRESHOLD: i64` are used identically in Tasks 1–3 and the verification grep. `Source` fields used (`kind: String`, `last_status: Option<String>` → `.as_deref()`, `consecutive_failures: i64`) match the struct at `source.rs:62-75`. `list_active_for_channel` returns `Result<Vec<Source>>` ordered by priority — the `.into_iter().filter().collect()` chain in Task 2 matches. ✓

# Idea #49 — One Coercion Source of Truth for Intake DTOs — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the scattered numeric-coercion logic (the `0`/`1` default literals and string→i64 parsing) for intake fields into one shared, unit-tested model-layer layer that both the HTML-form admin path and the JSON-API path call, so the two front doors can't drift.

**Architecture:** Add `coerce_i64` plus `DEFAULT_PRIORITY`/`DEFAULT_SORT_ORDER` constants to `src/model/mod.rs` (next to the existing `IntakeError`). The form handlers call `coerce_i64` (strict: blank→default, garbage→`IntakeError`→422); the JSON handlers keep their typed serde fields but reference the same constants in their `.unwrap_or(...)`. The JSON contract is unchanged. Auto-fetch-duration stays in the playlist handler (it does network I/O, not pure coercion).

**Tech Stack:** Rust 1.96, Axum 0.7, SQLx 0.7, Tokio. Tests are in-module `#[test]` (pure) and `tests/http.rs` integration (`tower::ServiceExt::oneshot`). `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + `cargo test` gate every commit.

**Spec:** `docs/superpowers/specs/2026-06-16-intake-coercion-source-of-truth-design.md`

**Planning note (deviation from spec):** The spec also listed dropping the "redundant" `form.url.trim()` in `admin/playlist.rs`. On inspection that trim is *not* redundant — the handler needs the trimmed url for the `media::fetch_duration` network call before the DTO ever runs. The DTO's re-trim is idempotent and harmless. This cleanup is therefore **dropped** from the plan; the playlist handler keeps its `form.url.trim()`. All other spec items stand.

---

### Task 1: Shared coercion layer in `src/model/mod.rs`

**Files:**
- Modify: `src/model/mod.rs` (add constants + `coerce_i64` near `IntakeError` at line 11; add a `#[cfg(test)] mod tests` if none exists, else append)

- [ ] **Step 1: Write the failing unit test**

`src/model/mod.rs` has no `#[cfg(test)]` module today. Append this to the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coerce_i64_blank_and_whitespace_use_default() {
        assert_eq!(coerce_i64("", 7).unwrap(), 7);
        assert_eq!(coerce_i64("   ", 7).unwrap(), 7);
    }

    #[test]
    fn test_coerce_i64_parses_trimmed_value() {
        assert_eq!(coerce_i64("5", 0).unwrap(), 5);
        assert_eq!(coerce_i64("  42 ", 0).unwrap(), 42);
        assert_eq!(coerce_i64("-3", 0).unwrap(), -3);
    }

    #[test]
    fn test_coerce_i64_garbage_is_error() {
        assert!(coerce_i64("abc", 0).is_err());
        assert!(coerce_i64("1.5", 0).is_err());
    }

    #[test]
    fn test_default_constants() {
        assert_eq!(DEFAULT_PRIORITY, 1);
        assert_eq!(DEFAULT_SORT_ORDER, 0);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib model::tests`
Expected: FAIL — `cannot find function coerce_i64` / `cannot find value DEFAULT_PRIORITY` / `DEFAULT_SORT_ORDER`.

- [ ] **Step 3: Add the constants and the coercion fn**

In `src/model/mod.rs`, immediately after the `IntakeError` `Display` impl (after line ~17, before `update_health_sql`), insert:

```rust
/// Default source priority when the intake field is blank/absent.
pub const DEFAULT_PRIORITY: i64 = 1;
/// Default sort order when the intake field is blank/absent.
pub const DEFAULT_SORT_ORDER: i64 = 0;

/// Coerce a form numeric field to `i64`: trimmed-blank/absent → `default`;
/// present-but-unparseable → `IntakeError` (strict — the adapter surfaces it as 422).
/// The single source of truth for intake numeric coercion across the form and JSON doors.
pub fn coerce_i64(raw: &str, default: i64) -> std::result::Result<i64, IntakeError> {
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

Note: the return type is spelled `std::result::Result<...>` because the module's
`use anyhow::Result;` shadows `Result` to the one-arg anyhow alias.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib model::tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/model/mod.rs
git commit -m "feat(model): shared coerce_i64 + DEFAULT_PRIORITY/SORT_ORDER intake constants

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Wire `admin/channels.rs` to the shared coercion (delete `parse_sort_order`)

**Files:**
- Modify: `src/routes/admin/channels.rs:73-82` (delete the local `parse_sort_order` fn)
- Modify: `src/routes/admin/channels.rs:113` and `:166` (the two call sites in `channel_create` / `channel_edit_post`)

This is behavior-preserving: the old `parse_sort_order` already did blank→0 and garbage→422; `coerce_i64(.., DEFAULT_SORT_ORDER)` does the same. Existing tests are the guard.

- [ ] **Step 1: Confirm the relevant existing tests pass first**

Run: `cargo test --test http channel`
Expected: PASS (current channel create/edit tests are green against the old code).

- [ ] **Step 2: Delete the local helper**

In `src/routes/admin/channels.rs`, remove the entire `parse_sort_order` fn (currently lines 73-82):

```rust
fn parse_sort_order(s: &str) -> Result<i64, StatusCode> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Ok(0)
    } else {
        trimmed
            .parse()
            .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)
    }
}
```

- [ ] **Step 3: Update the `channel_create` call site**

In `channel_create`, replace the line (currently line 113):

```rust
    let sort_order = parse_sort_order(&form.sort_order)?;
```

with:

```rust
    let sort_order = crate::model::coerce_i64(&form.sort_order, crate::model::DEFAULT_SORT_ORDER)
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
```

- [ ] **Step 4: Update the `channel_edit_post` call site**

In `channel_edit_post`, replace the identical line (currently line 166) with the same replacement as Step 3.

- [ ] **Step 5: Run the channel tests to confirm behavior is unchanged**

Run: `cargo test --test http channel && cargo build`
Expected: PASS, and build is clean (no unused-fn warning for the now-deleted `parse_sort_order`).

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/routes/admin/channels.rs
git commit -m "refactor(admin): channels use shared coerce_i64 for sort_order

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Wire `admin/sources.rs` to strict coercion + guard the new 422 behavior

**Files:**
- Test: `tests/http.rs` (add a `source_create_rejects_invalid_priority` test next to the existing source-create tests, ~line 870)
- Modify: `src/routes/admin/sources.rs:35` (the `priority` parse in `source_create`)

This task **changes behavior**: form `priority="abc"` currently → silently `1`; after this it → 422. The new test is written first (TDD) and fails against the old `unwrap_or(1)` code.

- [ ] **Step 1: Write the failing integration test**

Add to `tests/http.rs`, immediately after `source_create_rejects_invalid_kind` (the block ending ~line 859), using the existing `authed_form_post(uri, body)` helper:

```rust
#[tokio::test]
async fn source_create_rejects_invalid_priority() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/channels/1/sources",
            "kind=hls&url=https%3A%2F%2Fexample.com%2Ftest.m3u8&priority=abc",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test http source_create_rejects_invalid_priority`
Expected: FAIL — old code does `unwrap_or(1)`, so the request succeeds with `303 SEE_OTHER`, not `422`.

- [ ] **Step 3: Switch the handler to strict coercion**

In `src/routes/admin/sources.rs`, in `source_create`, replace the line (currently line 35):

```rust
    let priority: i64 = form.priority.trim().parse().unwrap_or(1);
```

with:

```rust
    let priority = crate::model::coerce_i64(&form.priority, crate::model::DEFAULT_PRIORITY)
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test http source_create`
Expected: PASS — the new `..._rejects_invalid_priority` (422) plus the existing `source_create_redirects_on_success` (priority=5, still 303) and the empty-url / invalid-kind tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/routes/admin/sources.rs tests/http.rs
git commit -m "refactor(admin): sources use strict coerce_i64 for priority (garbage -> 422)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Point the JSON-API defaults at the shared constants

**Files:**
- Modify: `src/routes/api/sources.rs:53` (the `.unwrap_or(1)`)
- Modify: `src/routes/api/playlist.rs:57` (the `.unwrap_or(0)`)

No behavior change — `1` and `0` are unchanged; this kills the duplicated literal so the default lives only in `model/mod.rs`. The existing `tests/api.rs` tests are the guard.

- [ ] **Step 1: Confirm the relevant API tests pass first**

Run: `cargo test --test api`
Expected: PASS (current API create tests green).

- [ ] **Step 2: Update `api/sources.rs`**

In `src/routes/api/sources.rs`, in `create`, replace (currently line 53):

```rust
        priority: req.priority.unwrap_or(1),
```

with:

```rust
        priority: req.priority.unwrap_or(crate::model::DEFAULT_PRIORITY),
```

- [ ] **Step 3: Update `api/playlist.rs`**

In `src/routes/api/playlist.rs`, in `create`, replace (currently line 57):

```rust
        sort_order: req.sort_order.unwrap_or(0),
```

with:

```rust
        sort_order: req.sort_order.unwrap_or(crate::model::DEFAULT_SORT_ORDER),
```

- [ ] **Step 4: Run the API tests to confirm behavior is unchanged**

Run: `cargo test --test api`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/routes/api/sources.rs src/routes/api/playlist.rs
git commit -m "refactor(api): reuse shared DEFAULT_PRIORITY/SORT_ORDER constants

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Backlog bookkeeping (IDEAS.md / CHANGELOG.md)

**Files:**
- Modify: `docs/IDEAS.md` (remove the #49 block from Open; bump the Done count)
- Modify: `docs/CHANGELOG.md` (add the #49 entry)

- [ ] **Step 1: Inspect the CHANGELOG format**

Run: `sed -n '1,40p' docs/CHANGELOG.md`
Expected: see the existing entry heading style / numbering to match it.

- [ ] **Step 2: Add the #49 CHANGELOG entry**

Append a #49 entry to `docs/CHANGELOG.md` following the existing format. Content to convey:

> **#49 — One coercion source of truth for intake DTOs.** The numeric-coercion logic for intake fields (the `0`/`1` default literals and string→i64 parsing) was duplicated across the HTML-form admin handlers and the JSON-API handlers and could drift. Collapsed it into one shared model-layer layer — `coerce_i64` + `DEFAULT_PRIORITY`/`DEFAULT_SORT_ORDER` in `model/mod.rs`. Form handlers now call `coerce_i64` (strict: blank→default, garbage→422); JSON handlers keep their typed serde fields but reference the same constants. Side effect: form `source.priority="abc"` now returns 422 instead of silently becoming `1`, making the form path internally consistent with `sort_order` and the JSON serde path. The JSON contract is unchanged; auto-fetch-duration stays in the playlist handler.

- [ ] **Step 3: Remove #49 from `docs/IDEAS.md` Open and bump the count**

In `docs/IDEAS.md`:
- Delete the entire `49.` block from the `## Open` section.
- Update the `## Done` line (`See CHANGELOG.md — 46 completed ideas …`) to `47 completed ideas` and extend the range note to include #49.

- [ ] **Step 4: Commit**

```bash
git add docs/IDEAS.md docs/CHANGELOG.md
git commit -m "docs: mark idea #49 done (intake coercion source of truth)

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
Expected: all tests pass. Count = previous total + 4 new `model::tests` unit tests + 1 new `source_create_rejects_invalid_priority` integration test; none removed.

- [ ] **Step 4: Confirm the single-source-of-truth invariant**

Run: `grep -rn "unwrap_or(1)\|unwrap_or(0)\|parse_sort_order\|\.parse().unwrap_or" src/routes/`
Expected: no numeric-default literals or `parse_sort_order` remain in the route layer — the only `.unwrap_or(...)` for these fields reference `DEFAULT_PRIORITY`/`DEFAULT_SORT_ORDER`. (The playlist handler's DB-max `sort_order` derivation may still end in `.unwrap_or(0)` — that is the next-sort-order computation, a distinct concern from intake coercion, and is out of scope.)

---

## Self-Review

**Spec coverage:**
- Spec §1 (shared coercion layer: `coerce_i64` + `DEFAULT_PRIORITY`/`DEFAULT_SORT_ORDER` in `model/mod.rs`) → Task 1. ✓
- Spec §2 (handler changes — channels, admin sources, api sources, api playlist) → Tasks 2, 3, 4. ✓
- Spec "behavior decision: strict" → Task 3 (the `priority=abc` → 422 guard). ✓
- Spec "Testing" (coerce_i64 unit tests, constants-wired test, new 422 integration guard, regression) → Task 1 (units + constants), Task 3 (422 guard), Tasks 2/4 (regression via existing suites), Task 6 (full run). ✓
- Spec "Out of scope" (JSON contract unchanged, auto-fetch stays, playlist sort_order semantics untouched, #50 separate) → honored; Task 4 only swaps literals for constants, Task 6 step 4 explicitly exempts the DB-max derivation. ✓
- Spec item "drop redundant `form.url.trim()`" → **intentionally dropped** with rationale (see Planning note); the trim feeds `fetch_duration`, so it is a handler concern, not redundant. ✓ (documented deviation)

**Placeholder scan:** No TBD/TODO; every code step shows complete code; the CHANGELOG step gives exact prose to convey rather than "update appropriately". ✓

**Type consistency:** `coerce_i64(raw: &str, default: i64) -> std::result::Result<i64, IntakeError>` is defined in Task 1 and called identically in Tasks 2 and 3 (`crate::model::coerce_i64(&form.x, crate::model::DEFAULT_*)`); the `.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?` adapter matches both handlers' `Result<_, StatusCode>` return type. `DEFAULT_PRIORITY: i64` / `DEFAULT_SORT_ORDER: i64` match the `i64` fields they default (`SourceInput.priority`, `ChannelInput.sort_order`, `PlaylistInput.sort_order`). ✓

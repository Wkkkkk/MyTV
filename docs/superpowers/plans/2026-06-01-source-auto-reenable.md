# Source Auto-Re-enable Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the health checker detects a disabled source passing a health check, automatically re-enable it and log the event.

**Architecture:** Add a private `HealthAction` enum to `health.rs` so `process_result` returns an explicit action instead of a bare bool. `check_one` matches on the action and passes `Option<bool>` to `update_health`. `update_health` in `source.rs` replaces its `set_inactive: bool` parameter with `is_active: Option<bool>` — `None` means don't touch the column, `Some(v)` sets it. No schema changes needed.

**Tech Stack:** Rust 1.96, Axum 0.7, SQLx 0.7 (SQLite), tokio

---

## File Map

| File | Change |
|------|--------|
| `src/health.rs` | Add `HealthAction` enum; update `process_result` return type and logic; update `check_one` |
| `src/model/source.rs` | Change `update_health` signature from `set_inactive: bool` to `is_active: Option<bool>`; update SQL; update tests |
| `docs/architecture/health-checker.md` | Update flowchart, state machine, and Notes section |

---

## Task 1: Update `process_result` and `check_one` in `health.rs`

**Files:**
- Modify: `src/health.rs`

These two functions are coupled — changing `process_result`'s return type requires updating `check_one` in the same step to keep the code compiling.

- [ ] **Step 1: Update the `process_result` tests**

Replace the four existing `process_result` tests and add two new ones. The key change is destructuring `(failures, action)` instead of `(failures, disable)` and asserting with `matches!`.

In `src/health.rs`, replace the entire `#[cfg(test)]` block with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn mock_source() -> Source {
        Source {
            id: 1,
            channel_id: 1,
            kind: "hls".to_string(),
            url: "https://example.com/stream.m3u8".to_string(),
            priority: 1,
            is_active: true,
            last_checked_at: None,
            last_status: None,
            consecutive_failures: 0,
            failure_reason: None,
        }
    }

    #[test]
    fn test_process_result_ok_resets_failures() {
        let src = Source {
            consecutive_failures: 2,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, true);
        assert_eq!(failures, 0);
        assert!(matches!(action, HealthAction::None));
    }

    #[test]
    fn test_process_result_error_increments_failures() {
        let src = Source {
            consecutive_failures: 1,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, false);
        assert_eq!(failures, 2);
        assert!(matches!(action, HealthAction::None));
    }

    #[test]
    fn test_process_result_triggers_disable_at_threshold() {
        let src = Source {
            consecutive_failures: 2,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(matches!(action, HealthAction::Disable));
    }

    #[test]
    fn test_process_result_already_inactive_not_disabled_again() {
        let src = Source {
            consecutive_failures: 2,
            is_active: false,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(matches!(action, HealthAction::None));
    }

    #[test]
    fn test_process_result_reenables_inactive_source_on_success() {
        let src = Source {
            is_active: false,
            consecutive_failures: 3,
            ..mock_source()
        };
        let (failures, action) = process_result(&src, true);
        assert_eq!(failures, 0);
        assert!(matches!(action, HealthAction::Reenable));
    }

    #[test]
    fn test_process_result_active_source_ok_no_action() {
        let src = mock_source(); // is_active: true, consecutive_failures: 0
        let (failures, action) = process_result(&src, true);
        assert_eq!(failures, 0);
        assert!(matches!(action, HealthAction::None));
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test health::tests 2>&1 | tail -20
```

Expected: compile errors or test failures — `HealthAction` doesn't exist yet.

- [ ] **Step 3: Add `HealthAction` enum, update `process_result`, update `check_one`**

Replace the content of `src/health.rs` (everything above the `#[cfg(test)]` block) with:

```rust
use std::time::Duration;

use sqlx::SqlitePool;

use crate::model::source::{self, Source};

const CHECK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const FAILURE_THRESHOLD: i64 = 3;

enum HealthAction {
    Disable,
    Reenable,
    None,
}

pub fn start(pool: SqlitePool, client: reqwest::Client) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            check_all(&pool, &client).await;
        }
    });
}

async fn check_all(pool: &SqlitePool, client: &reqwest::Client) {
    let sources = match source::list_all(pool).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("health: failed to fetch sources: {e}");
            return;
        }
    };
    for src in sources {
        check_one(pool, client, &src).await;
    }
}

async fn check_one(pool: &SqlitePool, client: &reqwest::Client, src: &Source) {
    let (ok, reason) = do_http_check(client, src).await;
    let (new_failures, action) = process_result(src, ok);

    let is_active = match action {
        HealthAction::Disable => Some(false),
        HealthAction::Reenable => Some(true),
        HealthAction::None => None,
    };

    if let Err(e) = source::update_health(
        pool,
        src.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        is_active,
    )
    .await
    {
        tracing::error!("health: failed to update source {}: {e}", src.id);
        return;
    }

    match action {
        HealthAction::Disable => tracing::warn!(
            "health: source {} auto-disabled after {} consecutive failures",
            src.id,
            new_failures
        ),
        HealthAction::Reenable => tracing::info!(
            "health: source {} auto-re-enabled after passing health check",
            src.id
        ),
        HealthAction::None => {}
    }
}

async fn do_http_check(client: &reqwest::Client, src: &Source) -> (bool, Option<String>) {
    let mut resp = match client.get(&src.url).timeout(HTTP_TIMEOUT).send().await {
        Ok(r) => r,
        Err(e) => return (false, Some(format!("request failed: {e}"))),
    };

    let status = resp.status();
    if !status.is_success() && !status.is_redirection() {
        return (false, Some(format!("HTTP {}", status.as_u16())));
    }

    if src.kind == "youtube_live" {
        return (true, None);
    }

    match resp.chunk().await {
        Ok(Some(_)) => (true, None),
        Ok(None) => (false, Some("stream returned no data".to_string())),
        Err(e) => (false, Some(format!("read failed: {e}"))),
    }
}

fn process_result(src: &Source, ok: bool) -> (i64, HealthAction) {
    let new_failures = if ok { 0 } else { src.consecutive_failures + 1 };
    let action = if ok && !src.is_active {
        HealthAction::Reenable
    } else if !ok && new_failures >= FAILURE_THRESHOLD && src.is_active {
        HealthAction::Disable
    } else {
        HealthAction::None
    };
    (new_failures, action)
}
```

Note: `check_one` now calls `source::update_health` with `is_active: Option<bool>` — this won't compile yet until Task 2 updates the signature. That is expected.

- [ ] **Step 4: Skip test run — crate won't compile yet**

`check_one` now passes `Option<bool>` to `update_health`, but `update_health` still takes `bool`. The crate won't compile until Task 2 updates the signature. Do not run `cargo test` here — full test run happens at the end of Task 2.

---

## Task 2: Update `update_health` in `source.rs`

**Files:**
- Modify: `src/model/source.rs`

- [ ] **Step 1: Add the new `update_health` test**

In `src/model/source.rs`, add this test inside the `#[cfg(test)] mod tests` block, after `test_update_health_disables_after_threshold`:

```rust
#[tokio::test]
async fn test_update_health_reenables_disabled_source() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;
    let src = create(
        &pool,
        hls(ch.id, "https://primary.example.com/stream.m3u8", 1),
    )
    .await
    .unwrap();

    // disable it first
    update_health(&pool, src.id, "error", Some("timeout"), 3, Some(false))
        .await
        .unwrap();
    let disabled = get(&pool, src.id).await.unwrap().unwrap();
    assert!(!disabled.is_active);

    // now re-enable it
    update_health(&pool, src.id, "ok", None, 0, Some(true))
        .await
        .unwrap();
    let reenabled = get(&pool, src.id).await.unwrap().unwrap();
    assert!(reenabled.is_active);
    assert_eq!(reenabled.consecutive_failures, 0);
    assert_eq!(reenabled.last_status.as_deref(), Some("ok"));
    assert!(reenabled.failure_reason.is_none());
}
```

- [ ] **Step 2: Update the `update_health` signature and SQL**

In `src/model/source.rs`, replace the entire `update_health` function with:

```rust
pub async fn update_health(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    reason: Option<&str>,
    consecutive_failures: i64,
    is_active: Option<bool>,
) -> Result<()> {
    if let Some(active) = is_active {
        sqlx::query(
            "UPDATE sources
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?,
                 is_active = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
        .bind(active)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE sources
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
        .bind(id)
        .execute(pool)
        .await?;
    }
    Ok(())
}
```

- [ ] **Step 3: Update the two existing `update_health` call sites in `source.rs` tests**

In the `test_update_health_ok_resets_failures` test, change the two `update_health` calls to:

```rust
update_health(&pool, src.id, "error", Some("timeout"), 2, None)
    .await
    .unwrap();
update_health(&pool, src.id, "ok", None, 0, None)
    .await
    .unwrap();
```

In the `test_update_health_disables_after_threshold` test, change the call to:

```rust
update_health(&pool, src.id, "error", Some("connection refused"), 3, Some(false))
    .await
    .unwrap();
```

- [ ] **Step 4: Run all tests**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests pass, no compile errors.

- [ ] **Step 5: Run fmt and clippy**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings 2>&1 | tail -20
```

Expected: no warnings, no errors.

- [ ] **Step 6: Commit**

```bash
git add src/health.rs src/model/source.rs
git commit -m "feat: auto-re-enable sources that pass health check after being disabled"
```

---

## Task 3: Update architecture docs

**Files:**
- Modify: `docs/architecture/health-checker.md`

- [ ] **Step 1: Replace the full content of `docs/architecture/health-checker.md`**

```markdown
# Health Checker & Source State Machine

A background Tokio task checks every source URL every 15 minutes and auto-disables sources that fail repeatedly. Sources that recover are automatically re-enabled on the next successful check.

## Tick Loop

\`\`\`mermaid
flowchart TD
    start(["health::start(pool, client)"]) --> spawn[Spawn detached Tokio task]
    spawn --> t1[Consume first tick\nno check at startup]
    t1 --> wait["Wait 15 min\nMissedTickBehavior::Skip"]
    wait --> fetch["list_all sources"]
    fetch --> loop{For each source}
    loop --> check["HTTP GET  (5s timeout)"]
    check -->|youtube_live| yt["HTTP 200/3xx sufficient\n(yt-dlp too slow for background)"]
    check -->|hls / iptv| chunk["Read one chunk\nverify bytes delivered"]
    yt --> result[process_result]
    chunk --> result
    result -->|"ok, is_active = true"| reset["failures = 0"]
    result -->|"ok, is_active = false"| reenable["failures = 0\nset is_active = 1"]
    result -->|"fail, failures < 3"| inc["failures++"]
    result -->|"fail, failures ≥ 3\nand is_active = true"| disable["set is_active = 0"]
    reset --> db[update_health in DB]
    reenable --> db
    inc --> db
    disable --> db
    db --> loop
    loop -->|all done| wait
\`\`\`

## Source State Machine

\`\`\`mermaid
stateDiagram-v2
    [*] --> Active : source created
    Active --> Active : check ok — failures reset to 0
    Active --> Active : check fails — failures < 3
    Active --> Disabled : check fails — failures reach 3
    Disabled --> Active : check ok — auto re-enabled
    Disabled --> Active : admin manually toggles on
\`\`\`

## Notes

**Auto-re-enable.** When a disabled source passes a health check, `process_result` returns `HealthAction::Reenable` and `update_health` sets `is_active = 1`. The source returns to active rotation immediately on the next check cycle — no cooldown period. The `HealthAction` enum (private to `health.rs`) makes the three outcomes — `Disable`, `Reenable`, `None` — mutually exclusive.

**Why `MissedTickBehavior::Skip`?** If a full check round (many sources all timing out at 5s each) takes longer than 15 minutes, any missed ticks are dropped rather than queued. This prevents a backlog of back-to-back check rounds after a slow cycle.

**First tick consumed.** The task calls `interval.tick().await` once immediately after spawning before entering the loop, which discards the initial zero-delay tick. Sources are not checked at startup.

**youtube_live shortcut.** Checking a YouTube live stream via yt-dlp takes several seconds and is too slow for a background health check. Instead, only the HTTP response status is checked (200 or 3xx is sufficient). The source is considered healthy if the page loads.
```

- [ ] **Step 2: Commit**

```bash
git add docs/architecture/health-checker.md
git commit -m "docs: update health checker architecture doc for auto-re-enable"
```

# Source Health Monitoring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a background Tokio task that checks every source's health every 15 minutes, flags failures in the admin panel, auto-disables sources after 3 consecutive failures, and shows a ⚠ warning in the EPG guide for channels with all sources down.

**Architecture:** New `migrations/002_source_health.sql` adds 4 health columns to `sources`. New `src/health.rs` owns the background loop logic and is spawned from `main.rs`. Admin UI and EPG guide template changes surface the results. No new crate dependencies.

**Tech Stack:** Rust, Tokio intervals, reqwest (already in use), sqlx, Askama templates

---

### Task 1: DB migration + Source model updates

**Files:**
- Create: `migrations/002_source_health.sql`
- Modify: `src/model/source.rs`

---

- [ ] **Step 1: Write the failing tests**

Add these tests inside the existing `#[cfg(test)] mod tests` block in `src/model/source.rs` (after the `test_set_active_toggles_source` test at line 195):

```rust
#[tokio::test]
async fn test_list_all_returns_sources_from_all_channels() {
    let pool = test_pool().await;
    let ch1 = make_channel(&pool).await;
    let ch2 = make_channel(&pool).await;
    create(&pool, hls(ch1.id, "https://a.example.com/stream.m3u8", 1))
        .await
        .unwrap();
    create(&pool, hls(ch2.id, "https://b.example.com/stream.m3u8", 1))
        .await
        .unwrap();
    let all = list_all(&pool).await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn test_update_health_ok_resets_failures() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;
    let src = create(&pool, hls(ch.id, "https://primary.example.com/stream.m3u8", 1))
        .await
        .unwrap();

    update_health(&pool, src.id, "error", Some("timeout"), 2, false)
        .await
        .unwrap();
    update_health(&pool, src.id, "ok", None, 0, false)
        .await
        .unwrap();

    let updated = get(&pool, src.id).await.unwrap().unwrap();
    assert_eq!(updated.last_status.as_deref(), Some("ok"));
    assert_eq!(updated.consecutive_failures, 0);
    assert!(updated.failure_reason.is_none());
    assert!(updated.is_active);
}

#[tokio::test]
async fn test_update_health_disables_after_threshold() {
    let pool = test_pool().await;
    let ch = make_channel(&pool).await;
    let src = create(&pool, hls(ch.id, "https://primary.example.com/stream.m3u8", 1))
        .await
        .unwrap();

    update_health(&pool, src.id, "error", Some("connection refused"), 3, true)
        .await
        .unwrap();

    let updated = get(&pool, src.id).await.unwrap().unwrap();
    assert!(!updated.is_active);
    assert_eq!(updated.consecutive_failures, 3);
    assert_eq!(updated.last_status.as_deref(), Some("error"));
    assert_eq!(updated.failure_reason.as_deref(), Some("connection refused"));
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_list_all test_update_health 2>&1
```

Expected: compile error — `list_all` and `update_health` are not defined yet.

- [ ] **Step 3: Create the migration file**

Create `migrations/002_source_health.sql`:

```sql
ALTER TABLE sources ADD COLUMN last_checked_at     INTEGER;
ALTER TABLE sources ADD COLUMN last_status          TEXT CHECK(last_status IN ('ok', 'error'));
ALTER TABLE sources ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sources ADD COLUMN failure_reason       TEXT;
```

- [ ] **Step 4: Update the `Source` struct**

In `src/model/source.rs`, replace lines 5–13 (the `Source` struct):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Source {
    pub id: i64,
    pub channel_id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
    pub is_active: bool,
    pub last_checked_at: Option<i64>,
    pub last_status: Option<String>,
    pub consecutive_failures: i64,
    pub failure_reason: Option<String>,
}
```

- [ ] **Step 5: Add `list_all` and `update_health` functions**

Add both functions after the `set_active` function (after line 84) and before the `#[cfg(test)]` block:

```rust
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<Source>> {
    sqlx::query_as::<_, Source>(
        "SELECT * FROM sources ORDER BY channel_id ASC, priority ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn update_health(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    reason: Option<&str>,
    consecutive_failures: i64,
    set_inactive: bool,
) -> Result<()> {
    if set_inactive {
        sqlx::query(
            "UPDATE sources
             SET last_checked_at = strftime('%s','now'),
                 last_status = ?,
                 failure_reason = ?,
                 consecutive_failures = ?,
                 is_active = 0
             WHERE id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(consecutive_failures)
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

- [ ] **Step 6: Run tests to confirm they pass**

```bash
cargo test 2>&1
```

Expected: all 94 tests pass (91 existing + 3 new). The existing source tests still pass because `SELECT *` now returns the new columns with their default values, and the `Source` struct accepts them.

- [ ] **Step 7: Commit**

```bash
git add migrations/002_source_health.sql src/model/source.rs
git commit -m "feat: add source health columns and model methods"
```

---

### Task 2: Health checker module

**Files:**
- Create: `src/health.rs`
- Modify: `src/main.rs`

---

- [ ] **Step 1: Write the failing tests**

Create `src/health.rs` with only the tests and the constants needed to make them compile:

```rust
use crate::model::source::Source;

pub(crate) const FAILURE_THRESHOLD: i64 = 3;

fn process_result(src: &Source, ok: bool) -> (i64, bool) {
    todo!()
}

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
        let src = Source { consecutive_failures: 2, ..mock_source() };
        let (failures, disable) = process_result(&src, true);
        assert_eq!(failures, 0);
        assert!(!disable);
    }

    #[test]
    fn test_process_result_error_increments_failures() {
        let src = Source { consecutive_failures: 1, ..mock_source() };
        let (failures, disable) = process_result(&src, false);
        assert_eq!(failures, 2);
        assert!(!disable);
    }

    #[test]
    fn test_process_result_triggers_disable_at_threshold() {
        let src = Source { consecutive_failures: 2, ..mock_source() };
        let (failures, disable) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(disable);
    }

    #[test]
    fn test_process_result_already_inactive_not_disabled_again() {
        let src = Source { consecutive_failures: 2, is_active: false, ..mock_source() };
        let (failures, disable) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(!disable);
    }
}
```

Also add `mod health;` to `src/main.rs` at line 6 (after `mod routes;`):

```rust
mod health;
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_process_result 2>&1
```

Expected: 4 test failures — all 4 hit `todo!()`.

- [ ] **Step 3: Implement `src/health.rs` in full**

Replace the entire content of `src/health.rs`:

```rust
use std::time::Duration;

use sqlx::SqlitePool;

use crate::model::source::{self, Source};

const CHECK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const FAILURE_THRESHOLD: i64 = 3;

pub fn start(pool: SqlitePool, client: reqwest::Client) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.tick().await; // consume the immediate first tick so we don't check at startup
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
    let (new_failures, set_inactive) = process_result(src, ok);

    if let Err(e) = source::update_health(
        pool,
        src.id,
        if ok { "ok" } else { "error" },
        reason.as_deref(),
        new_failures,
        set_inactive,
    )
    .await
    {
        tracing::error!("health: failed to update source {}: {e}", src.id);
        return;
    }

    if set_inactive {
        tracing::warn!(
            "health: source {} auto-disabled after {} consecutive failures",
            src.id,
            new_failures
        );
    }
}

async fn do_http_check(client: &reqwest::Client, src: &Source) -> (bool, Option<String>) {
    let mut resp = match client
        .get(&src.url)
        .timeout(HTTP_TIMEOUT)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return (false, Some(format!("request failed: {e}"))),
    };

    let status = resp.status();
    if !status.is_success() && !status.is_redirection() {
        return (false, Some(format!("HTTP {}", status.as_u16())));
    }

    // YouTube live: HTTP 200 is sufficient — yt-dlp resolution is too slow for background checks
    if src.kind == "youtube_live" {
        return (true, None);
    }

    // HLS / IPTV: read one chunk to verify the stream actually delivers bytes
    match resp.chunk().await {
        Ok(Some(_)) => (true, None),
        Ok(None) => (false, Some("stream returned no data".to_string())),
        Err(e) => (false, Some(format!("read failed: {e}"))),
    }
}

fn process_result(src: &Source, ok: bool) -> (i64, bool) {
    let new_failures = if ok { 0 } else { src.consecutive_failures + 1 };
    let set_inactive = !ok && new_failures >= FAILURE_THRESHOLD && src.is_active;
    (new_failures, set_inactive)
}

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
        let src = Source { consecutive_failures: 2, ..mock_source() };
        let (failures, disable) = process_result(&src, true);
        assert_eq!(failures, 0);
        assert!(!disable);
    }

    #[test]
    fn test_process_result_error_increments_failures() {
        let src = Source { consecutive_failures: 1, ..mock_source() };
        let (failures, disable) = process_result(&src, false);
        assert_eq!(failures, 2);
        assert!(!disable);
    }

    #[test]
    fn test_process_result_triggers_disable_at_threshold() {
        let src = Source { consecutive_failures: 2, ..mock_source() };
        let (failures, disable) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(disable);
    }

    #[test]
    fn test_process_result_already_inactive_not_disabled_again() {
        let src = Source { consecutive_failures: 2, is_active: false, ..mock_source() };
        let (failures, disable) = process_result(&src, false);
        assert_eq!(failures, 3);
        assert!(!disable);
    }
}
```

- [ ] **Step 4: Wire `health::start` into `main.rs`**

In `src/main.rs`, add the call to `health::start` after the `AppState` is built (after line 57, before the router setup). Insert these two lines:

```rust
    health::start(state.pool.clone(), state.http_client.clone());
```

Place it at line 59, between `};` (end of `AppState` construction) and `let admin_router`.

- [ ] **Step 5: Run tests to confirm all pass**

```bash
cargo test 2>&1
```

Expected: all 98 tests pass (94 from Task 1 + 4 new).

- [ ] **Step 6: Commit**

```bash
git add src/health.rs src/main.rs
git commit -m "feat: add source health checker background task"
```

---

### Task 3: Admin UI health badge

**Files:**
- Modify: `src/routes/admin/mod.rs`
- Modify: `templates/admin/channel_detail.html`

---

- [ ] **Step 1: Add health fields to `AdminSourceRow`**

In `src/routes/admin/mod.rs`, replace the `AdminSourceRow` struct (lines 41–47) and its `From` impl (lines 71–81):

```rust
pub struct AdminSourceRow {
    pub id: i64,
    pub kind: String,
    pub url: String,
    pub priority: i64,
    pub is_active: bool,
    pub last_status: Option<String>,
    pub consecutive_failures: i64,
    pub failure_reason: Option<String>,
}
```

```rust
impl From<source::Source> for AdminSourceRow {
    fn from(s: source::Source) -> Self {
        Self {
            id: s.id,
            kind: s.kind,
            url: s.url,
            priority: s.priority,
            is_active: s.is_active,
            last_status: s.last_status,
            consecutive_failures: s.consecutive_failures,
            failure_reason: s.failure_reason,
        }
    }
}
```

- [ ] **Step 2: Add the health column header**

In `templates/admin/channel_detail.html`, replace line 29:

```html
      <tr><th>Kind</th><th>URL</th><th>Priority</th><th>Active</th><th></th></tr>
```

with:

```html
      <tr><th>Kind</th><th>URL</th><th>Priority</th><th>Active</th><th>Health</th><th></th></tr>
```

- [ ] **Step 3: Add the health badge cell**

In `templates/admin/channel_detail.html`, add the health badge cell after the `<td>` block for Active (after the `{% endif %}` closing the `src.is_active` check, before the `<td style="white-space:nowrap">` actions cell). Insert after line 43:

```html
        <td>
          {% if src.last_status.is_none() %}
          <span style="color:#888" title="Never checked">○</span>
          {% else %}
          {% if src.last_status.as_deref() == Some("ok") %}
          <span style="color:#4caf50" title="Healthy">●</span>
          {% else %}
          <span style="color:#e94560" title="Last check failed">●</span>
          {% if let Some(reason) = src.failure_reason.as_ref() %}
          <div style="font-size:0.7rem;color:#e94560;margin-top:2px">{{ reason }}</div>
          {% endif %}
          {% if !src.is_active && src.consecutive_failures >= 3 %}
          <div style="font-size:0.7rem;color:#888">[auto-disabled]</div>
          {% endif %}
          {% endif %}
          {% endif %}
        </td>
```

- [ ] **Step 4: Build to verify template compiles**

```bash
cargo build 2>&1
```

Expected: `Finished` with no errors. Askama compiles templates at build time — any template variable mismatch fails here.

- [ ] **Step 5: Run full test suite**

```bash
cargo test 2>&1
```

Expected: all 98 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/routes/admin/mod.rs templates/admin/channel_detail.html
git commit -m "feat: show source health badge in admin channel detail"
```

---

### Task 4: EPG guide warning

**Files:**
- Modify: `src/routes/guide.rs`
- Modify: `templates/partials/epg_content.html`

---

- [ ] **Step 1: Write the failing tests**

Add these tests to the existing `#[cfg(test)] mod tests` block in `src/routes/guide.rs` (after `test_category_icon_known_categories`):

```rust
#[test]
fn test_all_sources_down_live_all_inactive() {
    use crate::model::channel::ChannelType;
    use std::collections::HashSet;
    let all: HashSet<i64> = [1i64].into_iter().collect();
    let active: HashSet<i64> = HashSet::new();
    assert!(is_all_sources_down(1, &ChannelType::Live, &all, &active));
}

#[test]
fn test_all_sources_down_live_has_active_source() {
    use crate::model::channel::ChannelType;
    use std::collections::HashSet;
    let all: HashSet<i64> = [1i64].into_iter().collect();
    let active: HashSet<i64> = [1i64].into_iter().collect();
    assert!(!is_all_sources_down(1, &ChannelType::Live, &all, &active));
}

#[test]
fn test_all_sources_down_vod_never_flagged() {
    use crate::model::channel::ChannelType;
    use std::collections::HashSet;
    let all: HashSet<i64> = [1i64].into_iter().collect();
    let active: HashSet<i64> = HashSet::new();
    assert!(!is_all_sources_down(1, &ChannelType::VodLoop, &all, &active));
}

#[test]
fn test_all_sources_down_no_sources_not_flagged() {
    use crate::model::channel::ChannelType;
    use std::collections::HashSet;
    let all: HashSet<i64> = HashSet::new();
    let active: HashSet<i64> = HashSet::new();
    assert!(!is_all_sources_down(1, &ChannelType::Live, &all, &active));
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test test_all_sources_down 2>&1
```

Expected: compile error — `is_all_sources_down` is not defined yet.

- [ ] **Step 3: Add `all_sources_down` to `ChannelRow`**

In `src/routes/guide.rs`, replace the `ChannelRow` struct (lines 35–39):

```rust
pub struct ChannelRow {
    pub name: String,
    pub category_icon: &'static str,
    pub all_sources_down: bool,
    pub programs: Vec<ProgramSlot>,
}
```

- [ ] **Step 4: Add the `is_all_sources_down` helper function**

Add this function directly after the `category_icon` function (after line 74), before `// ── template structs`:

```rust
fn is_all_sources_down(
    channel_id: i64,
    channel_type: &ChannelType,
    all_source_ids: &std::collections::HashSet<i64>,
    active_source_ids: &std::collections::HashSet<i64>,
) -> bool {
    matches!(channel_type, ChannelType::Live)
        && all_source_ids.contains(&channel_id)
        && !active_source_ids.contains(&channel_id)
}
```

- [ ] **Step 5: Update `build_guide_data`**

In `src/routes/guide.rs`, update `build_guide_data` to fetch source sets and pass `all_sources_down` to each row.

Add these two queries after `let channels: Vec<Channel> = ...` block (after line 215) and before `let mut rows`:

```rust
    let all_source_ids: std::collections::HashSet<i64> =
        sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();

    let active_source_ids: std::collections::HashSet<i64> =
        sqlx::query_scalar::<_, i64>(
            "SELECT DISTINCT channel_id FROM sources WHERE is_active = 1",
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
```

Replace the `rows.push(ChannelRow { ... })` call (lines 234–238) with:

```rust
        let all_sources_down =
            is_all_sources_down(ch.id, &ch.channel_type(), &all_source_ids, &active_source_ids);
        rows.push(ChannelRow {
            name: ch.name.clone(),
            category_icon: category_icon(&ch.category),
            all_sources_down,
            programs,
        });
```

- [ ] **Step 6: Update the EPG template**

In `templates/partials/epg_content.html`, replace line 42:

```html
      <div class="channel-col">{{ row.category_icon }} {{ row.name }}</div>
```

with:

```html
      <div class="channel-col">{% if row.all_sources_down %}⚠ {% endif %}{{ row.category_icon }} {{ row.name }}</div>
```

- [ ] **Step 7: Run full test suite**

```bash
cargo test 2>&1
```

Expected: all 102 tests pass (98 from previous tasks + 4 new).

- [ ] **Step 8: Commit**

```bash
git add src/routes/guide.rs templates/partials/epg_content.html
git commit -m "feat: show warning in EPG guide for channels with all sources down"
```

---

### Task 5: Deploy

**Files:** none

- [ ] **Step 1: Push to origin**

```bash
git push
```

Expected: pre-push hook runs fmt, clippy, tests — all pass.

- [ ] **Step 2: Deploy to Fly.io**

```bash
fly deploy --app kunstv
```

Expected: build completes, machine updates, `Visit your newly deployed app at https://kunstv.fly.dev/` in output.

- [ ] **Step 3: Verify health checker is running**

Check the live logs for the first health-check tick (fires 15 minutes after startup):

```bash
fly logs --app kunstv 2>&1 | grep health
```

Expected after ~15 minutes: log lines like `health: source 1 ...` or silence if all sources are healthy (no warnings logged for healthy sources).

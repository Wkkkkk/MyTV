# SQL Covering Indexes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two compound indexes to eliminate full-table scans and sort steps on the hot-path queries for `sources` and `playlist_items`.

**Architecture:** A single new SQLx migration file (`003_indexes.sql`) is all that's needed. `sqlx::migrate!("./migrations")` is a compile-time macro that embeds all files from `migrations/` in alphabetical order — adding `003_indexes.sql` makes it run automatically in both prod (`cargo run`) and tests (`cargo test`). No Rust code changes required.

**Tech Stack:** SQLite, SQLx 0.7 migrations

---

### Task 1: Add migration with compound indexes

**Files:**
- Create: `migrations/003_indexes.sql`

- [ ] **Step 1: Create the migration file**

Create `migrations/003_indexes.sql` with this exact content:

```sql
CREATE INDEX idx_sources_channel_priority ON sources(channel_id, priority);
CREATE INDEX idx_playlist_items_channel_sort ON playlist_items(channel_id, sort_order);
```

`idx_sources_channel_priority` covers three query patterns:
- `WHERE channel_id = ? ORDER BY priority ASC`
- `WHERE channel_id = ? AND is_active = 1 ORDER BY priority ASC` (narrows by channel via index, filters `is_active` in memory within the small result set, priority already in order)
- `ORDER BY channel_id ASC, priority ASC` (health checker list-all — index scan in order, no sort step)

`idx_playlist_items_channel_sort` covers:
- `WHERE channel_id = ? ORDER BY sort_order ASC`

- [ ] **Step 2: Run the full test suite**

```bash
cargo test
```

Expected: all 117 tests pass. The migration runs automatically for in-memory test DBs via `db::connect("sqlite::memory:")`. No test code changes needed — the indexed query paths are already exercised by existing integration tests.

- [ ] **Step 3: Run fmt and clippy**

```bash
cargo fmt && cargo clippy -- -D warnings
```

Expected: no warnings, no formatting diffs. (No Rust code was changed, but CI requires both to pass.)

- [ ] **Step 4: Commit**

```bash
git add migrations/003_indexes.sql
git commit -m "feat: add compound indexes on sources and playlist_items"
```

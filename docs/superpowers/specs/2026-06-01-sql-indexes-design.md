# SQL Covering Indexes — Design

**Date:** 2026-06-01
**Status:** Approved

## Problem

`sources.channel_id` and `playlist_items.channel_id` are unindexed foreign keys. Every lookup by channel requires a full table scan. The hot-path queries also include an ORDER BY on a second column, so without a compound index SQLite must sort after each scan.

Flagged by self-review check U10.

## Queries covered

| Table | Query | Index used |
|---|---|---|
| `sources` | `WHERE channel_id = ? ORDER BY priority ASC` | `idx_sources_channel_priority` |
| `sources` | `WHERE channel_id = ? AND is_active = 1 ORDER BY priority ASC` | `idx_sources_channel_priority` (narrows by channel, filters is_active in memory, priority already ordered) |
| `sources` | `ORDER BY channel_id ASC, priority ASC` (health checker list-all) | `idx_sources_channel_priority` |
| `playlist_items` | `WHERE channel_id = ? ORDER BY sort_order ASC` | `idx_playlist_items_channel_sort` |

## Solution

Add migration `003_indexes.sql`:

```sql
CREATE INDEX idx_sources_channel_priority ON sources(channel_id, priority);
CREATE INDEX idx_playlist_items_channel_sort ON playlist_items(channel_id, sort_order);
```

Two compound indexes. Each puts the FK column first (for WHERE equality), then the sort column second (so ORDER BY is served from the index without a separate sort step).

## What was considered and rejected

- **Simple FK indexes** (`channel_id` only) — would cover the WHERE but not eliminate the ORDER BY sort. Compound indexes are strictly better for the same write overhead.
- **Third index on `sources(is_active, channel_id)`** — would cover `SELECT DISTINCT channel_id FROM sources WHERE is_active = 1` in `guide.rs`, but this query runs once per guide load on a small table. Not worth the extra index.

## Testing

No new tests. Existing integration tests exercise all indexed query paths and will pass unchanged — indexes are transparent to SQLx.

## Implementation

One file to create: `migrations/003_indexes.sql`. No Rust code changes.

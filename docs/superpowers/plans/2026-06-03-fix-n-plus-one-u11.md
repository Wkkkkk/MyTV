# Fix N+1 Query in Guide: U11

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the per-channel `playlist_item::list_for_channel` call inside the guide render loop with a single `playlist_item::list_all` call, grouping items in memory by `channel_id`.

**Architecture:** Change is entirely in `src/routes/guide/data.rs`. `playlist_item::list_all` already exists in the model. No schema changes.

**Tech Stack:** Rust 1.96, sqlx

---

### Task 1: Replace N+1 with single query in `build_guide_data`

**Files:**
- Modify: `src/routes/guide/data.rs`

- [ ] **Step 1: Add `playlist_item::list_all` call before the loop**

In `src/routes/guide/data.rs`, after the `first_active_urls` HashMap is built (around line 104), add:

```rust
let all_playlist_items: std::collections::HashMap<i64, Vec<playlist_item::PlaylistItem>> =
    playlist_item::list_all(pool)
        .await?
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut acc, item| {
            acc.entry(item.channel_id).or_default().push(item);
            acc
        });
```

- [ ] **Step 2: Replace `list_for_channel` call in the loop**

In the `ChannelType::VodLoop` arm (around line 114), change:

```rust
ChannelType::VodLoop => {
    let items = playlist_item::list_for_channel(pool, ch.id).await?;
```

to:

```rust
ChannelType::VodLoop => {
    let items = all_playlist_items
        .get(&ch.id)
        .cloned()
        .unwrap_or_default();
```

- [ ] **Step 3: Remove `pool` argument from inner calls if now unused**

`pool` is still used for the other queries earlier in the function, so no change needed there.

- [ ] **Step 4: Run tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all tests pass. The guide integration tests (`test_guide_returns_200`, `test_guide_partial_returns_200`, `test_guide_renders_vod_budget_badge_from_cache`) should all pass unchanged.

- [ ] **Step 5: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add src/routes/guide/data.rs && git commit -m "perf: eliminate N+1 playlist_item queries in guide render"
```

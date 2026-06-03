# Code Quality Fixes: U3, U4, U5, U6, U9, U10

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Address six self-review findings: fix one `unwrap`, add model-layer validation, add minimal doc comments, update CLAUDE.md, fix HTML accessibility, add a SQL index migration.

**Architecture:** All changes are isolated — no new modules, no behaviour changes, no migrations that touch existing data. The SQL migration only adds an index.

**Tech Stack:** Rust 1.96, Askama templates, SQLite migrations

---

### Task 1: U3 — Fix `.unwrap()` in `playlist_item::current_position`

**Files:**
- Modify: `src/model/playlist_item.rs`

- [ ] **Step 1: Replace `.unwrap()` with `.expect()`**

In `src/model/playlist_item.rs`, change line 108:

```rust
Some((items.len() - 1, items.last().unwrap().duration_secs))
```

to:

```rust
Some((items.len() - 1, items.last().expect("non-empty: checked by is_empty guard above").duration_secs))
```

- [ ] **Step 2: Run tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add src/model/playlist_item.rs && git commit -m "fix: replace unwrap with expect in current_position"
```

---

### Task 2: U4 — Add validation in `channel::create` and `source::create`

**Files:**
- Modify: `src/model/channel.rs`
- Modify: `src/model/source.rs`

The route handlers already validate `channel_type` and `kind`, but the model functions accept any string. Add a defence-in-depth check so model callers can't bypass the constraint.

- [ ] **Step 1: Add validation to `channel::create` in `src/model/channel.rs`**

Find the `pub async fn create` function. At the top of the function body (before the `sqlx::query!`), add:

```rust
if !["live", "vod_loop"].contains(&input.channel_type.as_str()) {
    anyhow::bail!("invalid channel_type: {}", input.channel_type);
}
```

- [ ] **Step 2: Add validation to `source::create` in `src/model/source.rs`**

Find the `pub async fn create` function. At the top of the function body, add:

```rust
if !["hls", "youtube_live", "iptv"].contains(&input.kind.as_str()) {
    anyhow::bail!("invalid source kind: {}", input.kind);
}
```

- [ ] **Step 3: Run tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all tests pass (existing callers all pass valid values).

- [ ] **Step 4: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo fmt && git add src/model/channel.rs src/model/source.rs && git commit -m "fix: validate channel_type and source kind in model create functions"
```

---

### Task 3: U5 — Add minimal doc comments to key public items

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/ssrf.rs`
- Modify: `src/model/channel.rs`
- Modify: `src/model/source.rs`
- Modify: `src/model/playlist_item.rs`

Add a one-line `///` doc comment above each public item listed below. Do not add parameter descriptions or usage examples — one sentence is sufficient per the project's terse style.

- [ ] **Step 1: Add doc comments to `src/lib.rs`**

```rust
/// Shared in-memory cache mapping CDN host → CORS-allows-wildcard.
pub type CorsCache = Arc<RwLock<HashMap<String, bool>>>;

/// Shared application state cloned into every Axum handler.
#[derive(Clone)]
pub struct AppState { ... }

/// Build the Axum router with all routes and middleware wired up.
pub fn build_router(state: AppState) -> Router { ... }
```

- [ ] **Step 2: Add doc comments to `src/ssrf.rs`**

```rust
/// Errors returned by SSRF URL validation.
pub enum SsrfError { ... }

/// Returns `Ok` if `url` resolves to a public IP; `Err` if private/loopback/link-local.
pub async fn is_safe_url(url: &str) -> Result<(), SsrfError> { ... }

/// In-memory cache mapping hostname → time of last successful `is_safe_url` check.
pub type SsrfCache = Arc<RwLock<HashMap<String, std::time::Instant>>>;

/// Like `is_safe_url` but skips the DNS lookup if the hostname was validated within 60 s.
pub async fn is_safe_url_cached(url: &str, cache: &SsrfCache) -> Result<(), SsrfError> { ... }
```

- [ ] **Step 3: Add doc comments to `src/model/channel.rs`**

Add one-line `///` above each of these:

```rust
/// A channel row as stored in the database.
pub struct Channel { ... }

/// Channel playback mode.
pub enum ChannelType { ... }

/// Input for creating a new channel.
pub struct NewChannel { ... }

/// Input for updating an existing channel.
pub struct UpdateChannel { ... }

/// Insert a new channel and return it.
pub async fn create(...) { ... }

/// Update a channel by id; returns `None` if not found.
pub async fn update(...) { ... }

/// Fetch a channel by id.
pub async fn get(...) { ... }

/// List all channels ordered by sort_order.
pub async fn list(...) { ... }

/// Delete a channel by id; returns `true` if a row was removed.
pub async fn delete(...) { ... }

/// Return sorted, deduplicated category names from a channel list.
pub fn distinct_categories(...) { ... }
```

- [ ] **Step 4: Add doc comments to `src/model/source.rs` and `src/model/playlist_item.rs`**

Apply the same pattern — one-line `///` above each `pub struct`, `pub async fn`, `pub fn`. Keep each comment to one sentence describing what the item is or does.

- [ ] **Step 5: Run tests and commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test && cargo fmt && git add src/lib.rs src/ssrf.rs src/model/channel.rs src/model/source.rs src/model/playlist_item.rs && git commit -m "docs: add one-line doc comments to public API items"
```

---

### Task 4: U6 — Update CLAUDE.md project structure

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update the project structure section**

In `CLAUDE.md`, find the `src/` file listing and add the two missing modules:

```
  budget.rs       # CORS budget badge computation (⚡/☁) for guide display
  ssrf.rs         # SSRF URL validation and 60 s hostname cache
```

Find the `migrations/` listing and add the missing file:

```
migrations/       # 001_initial.sql, 002_source_health.sql, 003_indexes.sql
```

- [ ] **Step 2: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && git add CLAUDE.md && git commit -m "docs: add budget.rs, ssrf.rs, 003_indexes.sql to CLAUDE.md"
```

---

### Task 5: U9 — Fix HTML accessibility

**Files:**
- Modify: `templates/partials/epg_content.html`
- Modify: `templates/admin/channel_detail.html`
- Modify: `templates/admin/discover.html`

- [ ] **Step 1: Fix interactive `<div>` in `templates/partials/epg_content.html`**

Change (lines 49–54):

```html
<div class="program{% if prog.is_live %} live{% endif %}"
     style="left: {{ prog.left_pct }}%; width: {{ prog.width_pct }}%"
     onclick="tune({{ prog.channel_id }})">
```

to:

```html
<div class="program{% if prog.is_live %} live{% endif %}"
     style="left: {{ prog.left_pct }}%; width: {{ prog.width_pct }}%"
     role="button" tabindex="0"
     onclick="tune({{ prog.channel_id }})">
```

- [ ] **Step 2: Fix label/input associations in `templates/admin/channel_detail.html`**

Add `for`/`id` pairs to the add-source form (around lines 42–55) and add-playlist-item form (around lines 111–120). Change each unlinked label/input pair:

Source form — change:
```html
<label>URL</label>
<input type="text" name="url" required placeholder="https://...">
```
to:
```html
<label for="src-url">URL</label>
<input id="src-url" type="text" name="url" required placeholder="https://...">
```

And:
```html
<label>Priority</label>
<input type="number" name="priority" value="1" min="1">
```
to:
```html
<label for="src-priority">Priority</label>
<input id="src-priority" type="number" name="priority" value="1" min="1">
```

Playlist item form — change:
```html
<label>Title</label>
<input type="text" name="title" required placeholder="Episode title">
```
to:
```html
<label for="pl-title">Title</label>
<input id="pl-title" type="text" name="title" required placeholder="Episode title">
```

And:
```html
<label>URL</label>
<input type="text" name="url" required placeholder="https://...">
```
to:
```html
<label for="pl-url">URL</label>
<input id="pl-url" type="text" name="url" required placeholder="https://...">
```

And:
```html
<label>Duration (secs)</label>
<input type="number" name="duration_secs" min="0" placeholder="3600 (auto for YouTube)">
```
to:
```html
<label for="pl-duration">Duration (secs)</label>
<input id="pl-duration" type="number" name="duration_secs" min="0" placeholder="3600 (auto for YouTube)">
```

- [ ] **Step 3: Fix label/input associations in `templates/admin/discover.html`**

Add `for`/`id` pairs to the four unlinked inputs:

```html
<label for="disc-country">Country</label>
<input id="disc-country" type="text" name="country" placeholder="e.g. US">
```

```html
<label for="disc-group">Category / Group</label>
<input id="disc-group" type="text" name="group" placeholder="e.g. News">
```

```html
<label for="disc-keyword">Keyword</label>
<input id="disc-keyword" type="text" name="keyword" placeholder="search YouTube…">
```

```html
<label for="disc-url">Stream URL (HLS, IPTV, or YouTube)</label>
<input id="disc-url" type="text" name="url" placeholder="https://…" required>
```

- [ ] **Step 4: Run tests and commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test && git add templates/partials/epg_content.html templates/admin/channel_detail.html templates/admin/discover.html && git commit -m "fix: add role/tabindex to program div, add label for/id associations in admin forms"
```

---

### Task 6: U10 — Add SQL index on `sources.is_active`

**Files:**
- Create: `migrations/004_source_active_index.sql`

The guide page runs `WHERE is_active = 1` scans without an index. A compound index covering `(is_active, channel_id, priority)` covers both the filter and the ORDER BY used in guide queries.

- [ ] **Step 1: Create the migration file**

Create `migrations/004_source_active_index.sql` with:

```sql
CREATE INDEX idx_sources_is_active_channel_priority ON sources(is_active, channel_id, priority);
```

- [ ] **Step 2: Run tests**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && cargo test
```

Expected: all tests pass (sqlx auto-runs the new migration against in-memory test DBs).

- [ ] **Step 3: Commit**

```bash
cd /Users/kunwu/Workspace/playground/MyTV && git add migrations/004_source_active_index.sql && git commit -m "perf: add compound index on sources(is_active, channel_id, priority)"
```

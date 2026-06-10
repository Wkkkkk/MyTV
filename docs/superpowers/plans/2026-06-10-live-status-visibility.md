# Live-Status Visibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show fresh-on-view live status (Live / Offline / Unknown) for YouTube live sources across the admin discovery results and channel/source pages.

**Architecture:** A pure `interpret_is_live` + a `probe_live` (yt-dlp) primitive in `resolver.rs`, a 60s in-memory `LiveStatusCache` on `AppState`, and one lazy `GET /admin/live-status?url=…` endpoint that renders a badge partial. Three templates embed a lazy HTMX span that calls the endpoint on load.

**Tech Stack:** Rust, Axum 0.7, Askama 0.12 (built-in `urlencode` filter), yt-dlp, tokio, reqwest. Tests: `cargo test` (unit + `tower::ServiceExt::oneshot` integration).

---

## File Structure

- `src/media/resolver.rs` — `LiveStatus` enum, pure `interpret_is_live`, async `probe_live`, async `cached_live_status`.
- `src/lib.rs` — `LiveStatusCache` type alias, `live_cache` field on `AppState`, route registration.
- `src/main.rs` + `src/routes/player.rs` (test) + `tests/http.rs` — add `live_cache` to all `AppState` construction sites.
- `src/routes/admin/live_status.rs` (new) — `LiveStatusQuery`, `LiveStatusBadgeTemplate`, `badge_parts`, `live_status_badge` handler.
- `src/routes/admin/mod.rs` — declare module + export handler.
- `templates/admin/partials/live_status_badge.html` (new) — the badge markup.
- `templates/admin/partials/discover_manual_result.html`, `templates/admin/partials/discover_yt_results.html`, `templates/admin/partials/source_row.html`, `templates/admin/channel_detail.html` — embed lazy badge.
- `docs/IDEAS.md` — append deferred auto-resume idea.

---

## Task 1: Probe primitive (`LiveStatus`, `interpret_is_live`, `probe_live`)

**Files:**
- Modify: `src/media/resolver.rs`
- Test: `src/media/resolver.rs` (inline `#[cfg(test)]`)

`pub` items in this lib crate are reachable, so `probe_live` will not trigger `dead_code` even before it has callers.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/media/resolver.rs`:

```rust
    #[test]
    fn interpret_is_live_maps_all_cases() {
        assert_eq!(interpret_is_live(true, "True\n", ""), LiveStatus::Live);
        assert_eq!(interpret_is_live(true, "False\n", ""), LiveStatus::Offline);
        assert_eq!(
            interpret_is_live(false, "", "ERROR: [youtube:tab] UCxx: The channel is not currently live"),
            LiveStatus::Offline
        );
        assert_eq!(interpret_is_live(false, "", "ERROR: network unreachable"), LiveStatus::Unknown);
        assert_eq!(interpret_is_live(true, "", ""), LiveStatus::Unknown);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mytv interpret_is_live_maps_all_cases`
Expected: FAIL — `cannot find type LiveStatus` / `cannot find function interpret_is_live`.

- [ ] **Step 3: Implement the enum + pure interpreter + probe**

Add near the top of `src/media/resolver.rs` (after the existing `use` lines):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStatus {
    Live,
    Offline,
    Unknown,
}

/// Pure interpretation of a `yt-dlp --print is_live` invocation result.
pub fn interpret_is_live(success: bool, stdout: &str, stderr: &str) -> LiveStatus {
    let out = stdout.trim();
    if success && out == "True" {
        return LiveStatus::Live;
    }
    if success && out == "False" {
        return LiveStatus::Offline;
    }
    if stderr.to_ascii_lowercase().contains("not currently live") {
        return LiveStatus::Offline;
    }
    LiveStatus::Unknown
}

/// Probes whether a YouTube/Twitch live URL is currently broadcasting.
/// Times out after 8s; any spawn/timeout/parse failure yields `Unknown`.
pub async fn probe_live(url: &str) -> LiveStatus {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return LiveStatus::Unknown;
    }
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        Command::new("yt-dlp")
            .args(["--print", "is_live", "--no-playlist", "--", url])
            .output(),
    )
    .await;
    match result {
        Ok(Ok(output)) => interpret_is_live(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
        _ => LiveStatus::Unknown,
    }
}
```

(`Duration`, `Command` are already imported at the top of `resolver.rs`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p mytv interpret_is_live_maps_all_cases`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/media/resolver.rs
git commit -m "feat: add live-status probe primitive (yt-dlp is_live)"
```

---

## Task 2: `LiveStatusCache` + `cached_live_status` + `AppState` field

**Files:**
- Modify: `src/lib.rs` (type alias + `AppState` field)
- Modify: `src/media/resolver.rs` (`cached_live_status` helper)
- Modify: `src/main.rs`, `src/routes/player.rs`, `tests/http.rs` (all `AppState` construction sites)

This is a structural task — verification is a clean `cargo build` + `cargo test`. The behavior is covered by Task 3's endpoint test.

- [ ] **Step 1: Add the cache type alias and AppState field**

In `src/lib.rs`, just below the `CorsCache` type alias (`pub type CorsCache = ...`), add:

```rust
/// Shared in-memory cache mapping a source URL → (live status, when probed).
pub type LiveStatusCache =
    Arc<RwLock<HashMap<String, (crate::media::resolver::LiveStatus, std::time::Instant)>>>;
```

In the `AppState` struct, add the field after `pub metrics: Arc<metrics::Metrics>,`:

```rust
    pub live_cache: LiveStatusCache,
```

- [ ] **Step 2: Add the `cached_live_status` helper**

In `src/media/resolver.rs`, add:

```rust
/// Returns a cached live status if probed within the last 60s, otherwise probes
/// via `probe_live`, stores the result, and returns it.
pub async fn cached_live_status(cache: &crate::LiveStatusCache, url: &str) -> LiveStatus {
    {
        let map = cache.read().await;
        if let Some((status, at)) = map.get(url) {
            if at.elapsed() < Duration::from_secs(60) {
                return *status;
            }
        }
    }
    let status = probe_live(url).await;
    cache
        .write()
        .await
        .insert(url.to_string(), (status, std::time::Instant::now()));
    status
}
```

- [ ] **Step 3: Update every `AppState` construction site**

Add a `live_cache` field to each site, initialized exactly like the `cors_cache` field already is in that same block (an empty `Arc<RwLock<HashMap>>`). The six sites:

`src/main.rs` (the `let state = AppState { … }` around line 30) — add after `metrics: …,`:

```rust
        live_cache: Arc::new(RwLock::new(HashMap::new())),
```
(If `RwLock`/`HashMap` are not already imported in `main.rs`, mirror the exact path `cors_cache` uses in that file — copy its initializer and rename the field.)

`src/routes/player.rs` (the `test_state()` builder around line 482) — add after `metrics: …,`:

```rust
            live_cache: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
```

`tests/http.rs` — all four `AppState { … }` blocks (around lines 24, 56, 133, 163). In each, add after the `metrics: …,` line:

```rust
        live_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
```
(Match each block's existing `cors_cache:` initializer style — they already use `Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()))`.)

- [ ] **Step 4: Verify it builds and all tests pass**

Run: `cargo build && cargo test 2>&1 | grep -E "test result|FAILED|error\["`
Expected: clean build; existing tests still pass (233 unit + 64 integration), 0 failed.

- [ ] **Step 5: Commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/lib.rs src/media/resolver.rs src/main.rs src/routes/player.rs tests/http.rs
git commit -m "feat: add live-status cache to AppState"
```

---

## Task 3: Live-status endpoint + badge partial

**Files:**
- Create: `src/routes/admin/live_status.rs`
- Modify: `src/routes/admin/mod.rs` (declare module + export handler)
- Modify: `src/lib.rs` (register route in the admin sub-router)
- Create: `templates/admin/partials/live_status_badge.html`
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing integration tests**

Append to `tests/http.rs`:

```rust
#[tokio::test]
async fn admin_live_status_non_youtube_is_neutral() {
    let response = app()
        .await
        .oneshot(authed("/admin/live-status?url=https%3A%2F%2Fexample.com%2Ffoo"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    // Non-YouTube URL is never probed; renders the neutral "unknown" badge.
    assert!(body.contains("Live status unknown"));
    assert!(!body.contains("Currently live"));
}

#[tokio::test]
async fn admin_live_status_requires_auth() {
    let response = app()
        .await
        .oneshot(req("/admin/live-status?url=https%3A%2F%2Fexample.com%2Ffoo"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test http admin_live_status 2>&1 | tail -8`
Expected: FAIL — route not found (404), so the status assertions fail.

- [ ] **Step 3: Create the badge template**

Create `templates/admin/partials/live_status_badge.html`:

```html
<span style="color:{{ color }}" title="{{ title }}">{{ symbol }} {{ label }}</span>
```

- [ ] **Step 4: Create the handler module**

Create `src/routes/admin/live_status.rs`:

```rust
use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;

use crate::media::resolver::{self, LiveStatus};
use crate::routes::render;
use crate::AppState;

#[derive(Deserialize)]
pub struct LiveStatusQuery {
    pub url: String,
}

#[derive(Template)]
#[template(path = "admin/partials/live_status_badge.html")]
struct LiveStatusBadgeTemplate {
    symbol: &'static str,
    color: &'static str,
    label: &'static str,
    title: &'static str,
}

fn badge_parts(status: LiveStatus) -> LiveStatusBadgeTemplate {
    match status {
        LiveStatus::Live => LiveStatusBadgeTemplate {
            symbol: "●",
            color: "#4caf50",
            label: "live",
            title: "Currently live",
        },
        LiveStatus::Offline => LiveStatusBadgeTemplate {
            symbol: "○",
            color: "#888",
            label: "offline",
            title: "Not currently live",
        },
        LiveStatus::Unknown => LiveStatusBadgeTemplate {
            symbol: "·",
            color: "#666",
            label: "?",
            title: "Live status unknown",
        },
    }
}

pub async fn live_status_badge(
    State(state): State<AppState>,
    Query(q): Query<LiveStatusQuery>,
) -> Result<Html<String>, StatusCode> {
    let status = if resolver::needs_resolution(&q.url) {
        resolver::cached_live_status(&state.live_cache, &q.url).await
    } else {
        LiveStatus::Unknown
    };
    render(badge_parts(status))
}
```

- [ ] **Step 5: Declare the module and export the handler**

In `src/routes/admin/mod.rs`, add `pub mod live_status;` next to the other `pub mod` lines, and add `pub use live_status::live_status_badge;` near the other `pub use` lines.

- [ ] **Step 6: Register the route**

In `src/lib.rs`, inside the admin sub-router (where the other `.route("/discover/...", …)` entries are), add:

```rust
        .route("/live-status", get(routes::admin::live_status_badge))
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --test http admin_live_status 2>&1 | tail -8`
Expected: PASS (both tests).

- [ ] **Step 8: Commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add src/routes/admin/live_status.rs src/routes/admin/mod.rs src/lib.rs templates/admin/partials/live_status_badge.html tests/http.rs
git commit -m "feat: add /admin/live-status lazy badge endpoint"
```

---

## Task 4: Embed lazy badge in the three surfaces + ideas entry

**Files:**
- Modify: `templates/admin/partials/discover_manual_result.html`
- Modify: `templates/admin/partials/discover_yt_results.html`
- Modify: `templates/admin/partials/source_row.html`
- Modify: `templates/admin/channel_detail.html`
- Modify: `docs/IDEAS.md`
- Test: `tests/http.rs`

The channel-URL resolver path renders `discover_manual_result.html` with no API key and no network, so it is the deterministic regression test for the lazy-span markup. The `discover_yt_results.html` (needs API key + network) and `source_row.html` (needs a `youtube_live` seed source, which the fixture lacks) markup is verified by spec review + the manual checklist — noted here so the coverage gap is explicit, not silent.

- [ ] **Step 1: Write the failing test**

Append to `tests/http.rs`:

```rust
#[tokio::test]
async fn admin_channel_resolve_includes_live_badge() {
    let response = app()
        .await
        .oneshot(authed_form_post(
            "/admin/discover/channel/resolve",
            "url=%40NASA",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("hx-get=\"/admin/live-status?url="));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test http admin_channel_resolve_includes_live_badge 2>&1 | tail -5`
Expected: FAIL — the lazy span is not yet in the template.

- [ ] **Step 3: Add the lazy badge to the channel-URL resolver result**

In `templates/admin/partials/discover_manual_result.html`, inside the `<div style="display:flex;…">` row (after the `kind:` span on line 12, before the closing `</div>` on line 13), add:

```html
    {% if is_live && source_kind == "youtube_live" %}
    <span hx-get="/admin/live-status?url={{ url|urlencode }}"
          hx-trigger="load" hx-swap="outerHTML" style="color:#666">checking…</span>
    {% endif %}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test http admin_channel_resolve_includes_live_badge 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 5: Add the lazy badge to keyword channel results**

In `templates/admin/partials/discover_yt_results.html`, in the Type cell (the `<td>` that currently renders the LIVE/VOD badge, lines 13–19), append after the `{% endif %}` that closes the live/vod branch, still inside that `<td>`:

```html
        {% if row.is_live %}
        <span hx-get="/admin/live-status?url={{ row.url|urlencode }}"
              hx-trigger="load" hx-swap="outerHTML" style="color:#666;margin-left:6px">checking…</span>
        {% endif %}
```

- [ ] **Step 6: Add a "Live" column to source rows**

In `templates/admin/partials/source_row.html`, add a new `<td>` immediately after the budget-badge `<td>` (which closes on line 36, before the actions `<td>` on line 37):

```html
  <td>
    {% if src.kind == "youtube_live" %}
    <span hx-get="/admin/live-status?url={{ src.url|urlencode }}"
          hx-trigger="load" hx-swap="outerHTML" style="color:#666">checking…</span>
    {% else %}
    <span style="color:#444">—</span>
    {% endif %}
  </td>
```

In `templates/admin/channel_detail.html`, add a `<th>Live</th>` to the sources table header row, immediately after the budget column header (the sources `<thead>` row is just above the `{% for src in sources %}` loop on line 33 — add the header in the same column position as the new cell, i.e. after the budget `<th>` and before the actions `<th>`).

- [ ] **Step 7: Append the deferred idea**

In `docs/IDEAS.md`, append a new numbered idea (use the next number after the current highest):

```markdown
NN. **Auto-resume offline live channels** — when a YouTube live source is offline at tune time (yt-dlp: "not currently live"), the player currently returns a hard 503. Instead, show a "waiting for stream…" state and auto-retry on an interval, resuming playback automatically when the channel returns. Pairs with the live-status visibility work (docs/superpowers/specs/2026-06-10-live-status-visibility-design.md).
```

- [ ] **Step 8: Run the full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep -E "test result|FAILED|error\["`
Expected: clean; all tests pass (235 unit + 67 integration after this plan's additions), 0 failed.

- [ ] **Step 9: Commit**

```bash
git add templates/admin/partials/discover_manual_result.html templates/admin/partials/discover_yt_results.html templates/admin/partials/source_row.html templates/admin/channel_detail.html docs/IDEAS.md tests/http.rs
git commit -m "feat: show live-status badges in discovery and source rows"
```

---

## Final Verification

- [ ] **Run the CI-equivalent**

Run: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
Expected: all green.

- [ ] **Manual smoke test (key is in `.env`)**

`cargo run`, then at `/admin/discover` (YouTube tab):
- Resolve `@LofiGirl` → badge resolves to `● live` (Lofi is near-always live).
- Resolve `@NASA` or a NASA event channel → `○ offline` when idle.
- Channels keyword search `NASA` → each row shows a resolving live badge.
- Add a `youtube_live` source to a channel → its source row shows the live badge.

---

## Self-Review Notes

- **Spec coverage:** Section A → Tasks 1–2; Section B → Task 3; Section C consumers/tests/ideas → Task 4. The spec's enum-match-in-template detail was implemented instead as a `badge_parts` mapping to a string-field template struct (cleaner, avoids enum-in-template friction) — same rendered output, an acceptable implementation choice.
- **Type consistency:** `LiveStatus` (Task 1) used by `cached_live_status` (Task 2), `LiveStatusCache` (Task 2), and `live_status_badge` (Task 3). `cached_live_status(&crate::LiveStatusCache, &str)` signature matches the handler call site. `live_cache` field name consistent across all 6 construction sites and the handler.
- **Coverage gap (documented, not silent):** `discover_yt_results.html` (needs API key + network) and `source_row.html` (needs a `youtube_live` seed source) markup is verified by review + manual checklist; only the channel-resolve path has an automated markup test.
- **No placeholders:** every step carries full code/commands.

# UI Polish Cluster (#29–#33) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship five front-end improvements — CSS design tokens + color normalization, font-size bumps, HTMX spinners/skeletons, keyboard/accessibility polish, and a player overlay toolbar — as one cohesive change.

**Architecture:** A new shared stylesheet `static/app.css` (served at `/app.css`, mirroring the existing favicon/manifest static-route pattern) holds the `:root` design tokens and the deduplicated `.tabs`/`.tab` block. Both base templates link it and swap hardcoded hex for `var(--…)`. Everything else is template markup + vanilla JS. The pink-red `#e94560` brand accent is kept; only blue-tinted backgrounds are neutralized.

**Tech Stack:** Rust/Axum (one new route handler), Askama HTML templates, plain CSS + vanilla JS, HTMX. Tests via `tower::ServiceExt::oneshot` in `tests/http.rs`.

**Spec:** `docs/superpowers/specs/2026-06-13-ui-polish-cluster-design.md`

---

## File Structure

**Create:**
- `static/app.css` — `:root` design tokens + shared `.tabs`/`.tab`/`.tab:focus-visible` rules.

**Modify:**
- `src/routes/static_files.rs` — add `app_css()` handler.
- `src/lib.rs` — register `GET /app.css` route.
- `templates/base.html` — link `/app.css`; tokenize + normalize colors; font bumps; focus rings on `.program`/`.nav-btn`; skeleton + buffering-overlay CSS; player overlay toolbar JS.
- `templates/admin/base.html` — link `/app.css`; tokenize; remove the duplicated `.tabs`/`.tab` block (keep a one-line `margin-bottom` override).
- `templates/guide.html` — player overlay toolbar markup, shortcut-help panel, buffering overlay, EPG skeleton element; debug-button `aria-label`s.
- `templates/partials/epg_content.html` — tabs/time-nav `<a>` → `<button>`; `aria-selected`; program-block `keydown`; `hx-indicator` on nav buttons.
- `templates/admin/discover.html` — `htmx-indicator` spinner on the four search forms.
- `tests/http.rs` — assertions for `/app.css`, button tabs, skeleton element, overlay toolbar.

---

## Task 1: Shared stylesheet + `/app.css` route + design tokens

**Files:**
- Create: `static/app.css`
- Modify: `src/routes/static_files.rs`, `src/lib.rs:134-136`
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/http.rs` (near the other route tests, after `test_guide_partial_returns_200`):

```rust
#[tokio::test]
async fn test_app_css_returns_200_text_css() {
    let response = app().await.oneshot(req("/app.css")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/css"
    );
    let body = body_text(response).await;
    assert!(body.contains("--accent"), "app.css must define design tokens");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test http test_app_css_returns_200_text_css`
Expected: FAIL — route returns 404 (no `/app.css` registered).

- [ ] **Step 3: Create `static/app.css`**

```css
:root {
  /* surfaces */
  --bg: #0f0f0f;
  --surface-1: #111;
  --surface-2: #1a1a1a;
  --surface-nav: #141414;
  /* borders */
  --border: #222;
  --border-strong: #333;
  --border-subtle: #1c1c1c;
  /* text */
  --text: #e0e0e0;
  --text-muted: #999;
  --text-dim: #666;
  /* accent + semantic */
  --accent: #e94560;
  --accent-dark: #c73050;
  --live: #c0001a;
  --live-tint: #1a2a1a;
  --ok: #4caf50;
}

/* shared tab strip (guide EPG nav + admin sub-nav) */
.tabs { display: flex; gap: 6px; flex-wrap: wrap; }
.tab {
  padding: 4px 14px;
  border-radius: 3px;
  cursor: pointer;
  border: 1px solid var(--border-strong);
  background: var(--surface-2);
  color: var(--text-muted);
  font-size: 0.82rem;
  font-family: inherit;
  line-height: 1.5;
}
.tab.active, .tab:hover {
  background: var(--accent);
  color: #fff;
  border-color: var(--accent);
}
.tab:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
```

- [ ] **Step 4: Add the handler in `src/routes/static_files.rs`**

After the `MANIFEST` const (line 8), add:

```rust
const APP_CSS: &str = include_str!("../../static/app.css");
```

After `manifest_json()` (line 20), add:

```rust
pub async fn app_css() -> Response {
    ([(header::CONTENT_TYPE, "text/css")], APP_CSS).into_response()
}
```

- [ ] **Step 5: Register the route in `src/lib.rs`**

After line 136 (`.route("/favicon.ico", ...)`), add:

```rust
        .route("/app.css", get(routes::static_files::app_css))
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --test http test_app_css_returns_200_text_css`
Expected: PASS.

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add static/app.css src/routes/static_files.rs src/lib.rs tests/http.rs
git commit -m "feat(ui): add shared app.css with design tokens served at /app.css"
```

---

## Task 2: Tokenize + normalize colors + font bumps in `base.html`

This is a CSS refactor of the inline `<style>` block in `templates/base.html`. No automated test asserts colors; the existing `test_guide_returns_200` is the regression guard, and the values are verified manually in Task 9.

**Files:**
- Modify: `templates/base.html:1-117`

- [ ] **Step 1: Link the stylesheet**

In `<head>`, after line 7 (`<link rel="manifest" ...>`), add:

```html
  <link rel="stylesheet" href="/app.css">
```

- [ ] **Step 2: Normalize blue-tinted backgrounds + apply tokens in the `<style>` block**

Make these exact edits inside `templates/base.html`'s `<style>`:

- Line 15 `body`: `background:#0f0f0f;color:#e0e0e0` → `background:var(--bg);color:var(--text)`
- Line 19 `.site-header`: `background:#111` → `background:var(--surface-1)`; `border-bottom:1px solid #222` → `border-bottom:1px solid var(--border)`
- Line 20 `.site-header h1`: `color:#e94560` → `color:var(--accent)`
- Line 23 `#player-panel`: `border-bottom:2px solid #e94560` → `border-bottom:2px solid var(--accent)`
- Lines 25-27 `#player-error`: `color:#e94560` → `color:var(--accent)`
- Line 31 `#player-info`: `background:#111` → `background:var(--surface-1)`; `border-bottom:1px solid #1c1c1c` → `border-bottom:1px solid var(--border-subtle)`
- Line 40 `.pi-cat`: `color:#666` → `color:var(--text-dim)`
- Line 41 `.pi-live`: `background:#c0001a` → `background:var(--live)`
- Line 43 `.pi-pos`: `color:#555` → `color:var(--text-dim)`
- **Line 47 `.epg-nav` (BLUE → NEUTRAL):** `background:#141420;border-bottom:1px solid #222` → `background:var(--surface-nav);border-bottom:1px solid var(--border)`
- Lines 49-50 `.tab`: **delete these two `.tab`/`.tabs` rules entirely** (now provided by `app.css`). Keep the `.tabs` line 48 only if it differs — it is `display:flex;gap:6px;flex-wrap:wrap`, identical to app.css, so delete it too.
- Line 53 `.nav-btn`: `background:#222;color:#bbb;border:1px solid #333` → `background:var(--border);color:var(--text-muted);border:1px solid var(--border-strong)`; add `font-family:inherit;` (needed once it becomes a `<button>` in Task 4)
- Line 55 `.nav-btn:hover`: `background:#333` → `background:var(--border-strong)`
- Line 56 `.time-range`: `color:#777` → `color:var(--text-muted)`
- Line 61 `.epg-row`: `border-bottom:1px solid #1c1c1c` → `border-bottom:1px solid var(--border-subtle)`
- Lines 62-63 `.channel-col`: `border-right:1px solid #222;background:#111` → `border-right:1px solid var(--border);background:var(--surface-1)`
- **Line 68 `.time-label` (FONT BUMP):** `font-size:0.7rem;color:#555` → `font-size:0.75rem;color:var(--text-dim)`
- **Lines 71-74 `.program` (BLUE → NEUTRAL + FONT BUMP):** `font-size:0.78rem` → `font-size:0.82rem`; `background:#1a2235` → `background:var(--surface-2)`
- **Line 75 `.program:hover`:** `background:#22304a` → `background:#222` (neutral hover lift)
- **Lines 76-77 `.program.live` / `.program.live:hover`:** `background:#1a2a1a` → `background:var(--live-tint)`; `background:#203220` → `background:#203a20` (keep green semantic)
- Lines 78-79 `.live-badge`: `background:#c0001a` → `background:var(--live)`
- Line 83 `.now-line`: `background:#e94560` → `background:var(--accent)`
- Lines 100-108 status badges: `.health-ok{color:#4caf50}` → `color:var(--ok)`; `.health-down{color:#e94560}` → `color:var(--accent)`; `.health-unknown{color:#666}` → `color:var(--text-dim)`

- [ ] **Step 3: Move the inline header-nav style into a class**

Replace line 114:

```html
    <nav style="display:flex;gap:12px;font-size:0.85rem"><a href="/admin" style="color:#999;text-decoration:none">Admin</a></nav>
```

with:

```html
    <nav class="site-nav"><a href="/admin">Admin</a></nav>
```

And add to the `<style>` block (after `.site-header h1`, line 20):

```css
    .site-nav{display:flex;gap:12px;font-size:0.85rem}
    .site-nav a{color:var(--text-muted)}
```

- [ ] **Step 4: Add focus rings for guide-specific interactive elements**

In the `<style>` block, after the `.now-line` rule (line 84), add:

```css
    /* keyboard focus rings (#31) */
    .program:focus-visible,.nav-btn:focus-visible{outline:2px solid var(--accent);outline-offset:1px}
```

- [ ] **Step 5: Verify the guide still renders**

Run: `cargo test --test http test_guide_returns_200 test_guide_partial_returns_200`
Expected: PASS (both).

- [ ] **Step 6: Commit**

```bash
git add templates/base.html
git commit -m "refactor(ui): tokenize base.html, neutralize blue tints, bump fonts, add focus rings"
```

---

## Task 3: Tokenize `admin/base.html` + remove duplicated tab block

**Files:**
- Modify: `templates/admin/base.html:1-64`

- [ ] **Step 1: Link the stylesheet**

In `<head>`, after line 7 (`<link rel="manifest" ...>`), add:

```html
  <link rel="stylesheet" href="/app.css">
```

- [ ] **Step 2: Remove the duplicated tab block, keep a margin override**

Delete lines 60-63 (the `.tabs` and `.tab`/`.tab.active` rules — now in `app.css`). Replace with a single context override:

```css
    .tabs{margin-bottom:16px}
```

- [ ] **Step 3: Apply tokens to the shared palette values**

Edit `templates/admin/base.html`'s `<style>`:

- Line 13 `body`: `background:#0f0f0f;color:#e0e0e0` → `background:var(--bg);color:var(--text)`
- Line 14 `a`: `color:#e94560` → `color:var(--accent)`
- Line 17 `.site-header`: `background:#111` → `background:var(--surface-1)`; `border-bottom:1px solid #222` → `border-bottom:1px solid var(--border)`
- Line 18 `.site-header h1`: `color:#e94560` → `color:var(--accent)`
- Line 20 `.site-header nav a`: `color:#999` → `color:var(--text-muted)`
- Line 21 `.site-header nav a:hover`: `color:#e94560` → `color:var(--accent)`
- Line 29 `td`: `border-bottom:1px solid #1a1a1a` → `border-bottom:1px solid var(--border-subtle)`
- Line 30 `tr:hover td`: `background:#111` → `background:var(--surface-1)`
- Lines 32-37 buttons: `.btn` `background:#1a1a1a;color:#ccc;border:1px solid #333` → `background:var(--surface-2);color:#ccc;border:1px solid var(--border-strong)`; `.btn-primary{background:#e94560;...border-color:#e94560}` → `var(--accent)`; `.btn-primary:hover{background:#c73050}` → `var(--accent-dark)`
- Line 47 `input:focus,select:focus`: `border-color:#e94560` → `border-color:var(--accent)`
- Line 57 `.section`: `border-top:1px solid #1c1c1c` → `border-top:1px solid var(--border-subtle)`

(Admin-only colors not in the shared palette — `.badge-*`, `.btn-danger`, input `#2a2a2a` — are left as literals; the spec scopes tokenization to the shared palette, YAGNI on the rest.)

- [ ] **Step 4: Verify admin renders + tab styling intact**

Run: `cargo test --test http test_admin_channels_authed_returns_200`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add templates/admin/base.html
git commit -m "refactor(ui): tokenize admin/base.html, dedupe shared .tabs/.tab into app.css"
```

---

## Task 4: EPG accessibility — buttons, aria, keyboard

**Files:**
- Modify: `templates/partials/epg_content.html`, `templates/guide.html`
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/http.rs`:

```rust
#[tokio::test]
async fn test_guide_partial_tabs_are_buttons() {
    let response = app().await.oneshot(req("/guide/partial")).await.unwrap();
    let body = body_text(response).await;
    assert!(
        body.contains("<button class=\"tab"),
        "EPG category tabs must be <button> elements for accessibility"
    );
    assert!(
        !body.contains("<a class=\"tab"),
        "EPG tabs must no longer be hrefless <a> elements"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test http test_guide_partial_tabs_are_buttons`
Expected: FAIL — tabs are still `<a class="tab">`.

- [ ] **Step 3: Convert tabs and time-nav to `<button>` in `epg_content.html`**

Replace the `.tabs` block (lines 3-14) with:

```html
  <div class="tabs" role="tablist">
    <button type="button" class="tab{% if active_category.as_str() == "all" %} active{% endif %}"
       role="tab" aria-selected="{% if active_category.as_str() == "all" %}true{% else %}false{% endif %}"
       hx-get="/guide/partial?category=all&amp;offset={{ offset_hours }}"
       hx-target="#epg-content" hx-indicator="#epg-skeleton"
       hx-swap="innerHTML">All</button>
    {% for cat in categories %}
    <button type="button" class="tab{% if active_category.as_str() == cat.as_str() %} active{% endif %}"
       role="tab" aria-selected="{% if active_category.as_str() == cat.as_str() %}true{% else %}false{% endif %}"
       hx-get="/guide/partial?category={{ cat }}&amp;offset={{ offset_hours }}"
       hx-target="#epg-content" hx-indicator="#epg-skeleton"
       hx-swap="innerHTML">{{ cat }}</button>
    {% endfor %}
  </div>
```

Replace the `.time-nav` block (lines 15-25) with:

```html
  <div class="time-nav">
    <button type="button" class="nav-btn"
       hx-get="/guide/partial?category={{ active_category }}&amp;offset={{ offset_prev }}"
       hx-target="#epg-content" hx-indicator="#epg-skeleton"
       hx-swap="innerHTML">&#8592; Earlier</button>
    <span class="time-range">{{ window_label }}</span>
    <button type="button" class="nav-btn"
       hx-get="/guide/partial?category={{ active_category }}&amp;offset={{ offset_next }}"
       hx-target="#epg-content" hx-indicator="#epg-skeleton"
       hx-swap="innerHTML">Later &#8594;</button>
  </div>
```

- [ ] **Step 4: Add a keydown handler to program blocks**

In `epg_content.html`, the program block (lines 49-52) currently has `onclick="tune({{ prog.channel_id }})"`. Add a keyboard handler alongside it:

```html
        <div class="program{% if prog.is_live %} live{% endif %}"
             style="left: {{ prog.left_pct }}%; width: {{ prog.width_pct }}%"
             role="button" tabindex="0"
             onclick="tune({{ prog.channel_id }})"
             onkeydown="if(event.key==='Enter'||event.key===' '){event.preventDefault();tune({{ prog.channel_id }})}">
```

- [ ] **Step 5: Add aria-labels to the debug panel buttons in `guide.html`**

In `templates/guide.html`, line 33 (`debug-clear` button) add `aria-label="Clear debug log"`; line 34 (`debug-toggle` button) add `aria-label="Toggle debug log visibility"`. No other change to the panel.

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --test http test_guide_partial_tabs_are_buttons`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add templates/partials/epg_content.html templates/guide.html
git commit -m "feat(a11y): button tabs, aria-selected, program keydown, debug aria-labels"
```

---

## Task 5: EPG loading skeleton

**Files:**
- Modify: `templates/guide.html`, `templates/base.html`
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/http.rs`:

```rust
#[tokio::test]
async fn test_guide_has_epg_skeleton() {
    let response = app().await.oneshot(req("/guide")).await.unwrap();
    let body = body_text(response).await;
    assert!(
        body.contains("id=\"epg-skeleton\""),
        "guide must include the HTMX loading skeleton element"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test http test_guide_has_epg_skeleton`
Expected: FAIL — no skeleton element.

- [ ] **Step 3: Add the skeleton element in `guide.html`**

In `templates/guide.html`, wrap `#epg-content`. Replace:

```html
<div id="epg-content">
  {% include "partials/epg_content.html" %}
</div>
```

with:

```html
<div id="epg-area" style="position:relative">
  <div id="epg-skeleton" class="htmx-indicator" aria-hidden="true">
    <div class="skel-row"><div class="skel-chan"><span class="shim" style="width:60px;height:12px"></span></div><div class="skel-progs"><span class="shim" style="flex:2"></span><span class="shim" style="flex:1"></span><span class="shim" style="flex:3"></span></div></div>
    <div class="skel-row"><div class="skel-chan"><span class="shim" style="width:50px;height:12px"></span></div><div class="skel-progs"><span class="shim" style="flex:1"></span><span class="shim" style="flex:2"></span></div></div>
    <div class="skel-row"><div class="skel-chan"><span class="shim" style="width:70px;height:12px"></span></div><div class="skel-progs"><span class="shim" style="flex:3"></span><span class="shim" style="flex:1"></span><span class="shim" style="flex:1"></span></div></div>
    <div class="skel-row"><div class="skel-chan"><span class="shim" style="width:55px;height:12px"></span></div><div class="skel-progs"><span class="shim" style="flex:2"></span><span class="shim" style="flex:2"></span></div></div>
    <div class="skel-row"><div class="skel-chan"><span class="shim" style="width:65px;height:12px"></span></div><div class="skel-progs"><span class="shim" style="flex:1"></span><span class="shim" style="flex:3"></span><span class="shim" style="flex:1"></span></div></div>
    <div class="skel-row"><div class="skel-chan"><span class="shim" style="width:50px;height:12px"></span></div><div class="skel-progs"><span class="shim" style="flex:3"></span><span class="shim" style="flex:2"></span></div></div>
  </div>
  <div id="epg-content">
    {% include "partials/epg_content.html" %}
  </div>
</div>
```

(Six static placeholder rows — no Askama range loop, which keeps it compatible across template-engine versions.)

- [ ] **Step 4: Add skeleton CSS to `base.html`**

In `templates/base.html`'s `<style>` block, after the status-badge rules (line 108), add:

```css
    /* ── loading skeleton (#33) ───────────────────────────────── */
    #epg-skeleton{display:none}
    #epg-skeleton.htmx-request,.htmx-request #epg-skeleton{display:block;
      position:absolute;top:0;left:0;right:0;z-index:10;background:var(--bg)}
    .skel-row{display:flex;min-height:44px;border-bottom:1px solid var(--border-subtle)}
    .skel-chan{width:140px;flex-shrink:0;border-right:1px solid var(--border);
      background:var(--surface-1);display:flex;align-items:center;padding:0 10px}
    .skel-progs{flex:1;display:flex;gap:2px;padding:1px;align-items:center;height:44px}
    .skel-progs .shim{height:42px}
    .shim{background:var(--surface-2);border-radius:3px;
      background-image:linear-gradient(90deg,var(--surface-2) 0,#262626 150px,var(--surface-2) 300px);
      background-size:600px 100%;animation:shimmer 1.2s infinite linear}
    @keyframes shimmer{0%{background-position:-300px 0}100%{background-position:300px 0}}
    @media(max-width:600px){.skel-chan{width:80px}}
```

Note: HTMX adds the `htmx-request` class to **both** the element issuing the request and any element named by its `hx-indicator`. Since the nav buttons use `hx-indicator="#epg-skeleton"`, `#epg-skeleton` itself gets `htmx-request` during the fetch — hence the `#epg-skeleton.htmx-request` selector.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --test http test_guide_has_epg_skeleton`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add templates/guide.html templates/base.html
git commit -m "feat(ui): EPG loading skeleton via htmx-indicator"
```

---

## Task 6: Discovery search spinners

**Files:**
- Modify: `templates/admin/discover.html`
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/http.rs`:

```rust
#[tokio::test]
async fn test_discover_page_has_search_spinner() {
    let response = app().await.oneshot(authed("/admin/discover")).await.unwrap();
    let body = body_text(response).await;
    assert!(
        body.contains("class=\"htmx-indicator spinner\""),
        "discovery search forms must show an inline spinner during fetch"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test http test_discover_page_has_search_spinner`
Expected: FAIL — no spinner element.

- [ ] **Step 3: Add a spinner next to each search button**

In `templates/admin/discover.html`, after each of the four submit buttons (lines 27, 51, 70, 85 — the `<button class="btn btn-primary btn-sm" type="submit" ...>` elements), add a sibling spinner span:

```html
    <span class="htmx-indicator spinner" aria-hidden="true"></span>
```

Each `<form>` already has `hx-post`; HTMX adds `htmx-request` to the form during the fetch, so a descendant `.htmx-indicator` shows automatically (no `hx-indicator` attribute needed).

- [ ] **Step 4: Add spinner CSS**

Append to the `<style>` block in `templates/admin/base.html` (after the `.tabs` override from Task 3):

```css
    .spinner{display:none;width:14px;height:14px;border-radius:50%;
      border:2px solid var(--border-strong);border-top-color:var(--accent);
      vertical-align:middle;margin-left:6px}
    .htmx-request .spinner,.spinner.htmx-request{display:inline-block;
      animation:spin 0.7s linear infinite}
    @keyframes spin{to{transform:rotate(360deg)}}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --test http test_discover_page_has_search_spinner`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add templates/admin/discover.html templates/admin/base.html
git commit -m "feat(ui): inline spinner on discovery search forms"
```

---

## Task 7: Player overlay toolbar + buffering overlay

**Files:**
- Modify: `templates/guide.html`, `templates/base.html`
- Test: `tests/http.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/http.rs`:

```rust
#[tokio::test]
async fn test_guide_has_player_overlay_toolbar() {
    let response = app().await.oneshot(req("/guide")).await.unwrap();
    let body = body_text(response).await;
    assert!(
        body.contains("id=\"player-toolbar\""),
        "player must include the overlay toolbar"
    );
    assert!(
        body.contains("id=\"player-buffering\""),
        "player must include the buffering overlay"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test http test_guide_has_player_overlay_toolbar`
Expected: FAIL — neither element exists.

- [ ] **Step 3: Add the toolbar, help panel, and buffering overlay markup in `guide.html`**

Replace the `#player-panel` block (lines 7-12) with:

```html
<div id="player-panel">
  <div id="player-toolbar">
    <button type="button" class="ov-btn" id="ov-close" title="Close" aria-label="Close player">✕</button>
    <button type="button" class="ov-btn" id="ov-prev" title="Previous channel" aria-label="Previous channel">↑</button>
    <button type="button" class="ov-btn" id="ov-next" title="Next channel" aria-label="Next channel">↓</button>
    <span class="ov-spacer"></span>
    <button type="button" class="ov-btn" id="ov-help" title="Keyboard shortcuts" aria-label="Keyboard shortcuts">?</button>
  </div>
  <div id="player-help" hidden>
    <strong>Keyboard shortcuts</strong>
    <div>↑ / ↓ — change channel</div>
    <div>Space — play / pause</div>
    <div>← / → — seek 10s (VOD)</div>
    <div>F — fullscreen</div>
  </div>
  <div id="player-buffering"><span class="spinner-lg"></span> Loading…</div>
  <video id="video" controls></video>
  <div id="player-error">Channel unavailable</div>
  <div id="player-ended">Stream ended — switching to next channel…</div>
  <div id="player-waiting">Waiting for stream…</div>
</div>
```

- [ ] **Step 4: Add overlay CSS to `base.html`**

In `templates/base.html`'s `<style>`, after the `#player-panel video` rule (line 24), add:

```css
    #player-panel{position:relative}
    #player-toolbar{position:absolute;top:0;left:0;right:0;z-index:6;
      display:flex;align-items:center;gap:8px;padding:10px 12px;
      background:linear-gradient(#000,transparent);
      opacity:0;transition:opacity 0.2s;pointer-events:none}
    #player-panel.show-controls #player-toolbar{opacity:1;pointer-events:auto}
    .ov-btn{width:34px;height:34px;display:flex;align-items:center;justify-content:center;
      background:rgba(20,20,20,0.7);color:#eee;border:1px solid var(--border-strong);
      border-radius:4px;cursor:pointer;font-size:1rem;font-family:inherit}
    .ov-btn:hover{background:var(--accent);border-color:var(--accent);color:#fff}
    .ov-btn:focus-visible{outline:2px solid var(--accent);outline-offset:1px}
    .ov-spacer{flex:1}
    #player-help{position:absolute;top:52px;right:12px;z-index:7;
      background:rgba(10,10,10,0.92);border:1px solid var(--border-strong);
      border-radius:5px;padding:10px 14px;font-size:0.8rem;color:var(--text);line-height:1.7}
    #player-help strong{display:block;margin-bottom:4px;color:var(--accent)}
    #player-buffering{display:none;position:absolute;top:0;left:0;right:0;bottom:0;z-index:5;
      align-items:center;justify-content:center;gap:10px;
      background:#000;color:#fff;font-size:0.95rem}
    #player-buffering.show{display:flex}
    .spinner-lg{width:22px;height:22px;border-radius:50%;
      border:3px solid #444;border-top-color:var(--accent);
      animation:spin 0.7s linear infinite}
    @keyframes spin{to{transform:rotate(360deg)}}
```

- [ ] **Step 5: Wire the overlay behavior in `base.html` JS**

Inside the `DOMContentLoaded` handler in `templates/base.html`, after `window.tune = tune;` (line 462), add:

```javascript
      // ── player overlay toolbar (#32) ──────────────────────────
      var panel = document.getElementById('player-panel');
      var helpBox = document.getElementById('player-help');
      var hideControlsTimer = null;
      function showControls() {
        if (!panel) return;
        panel.classList.add('show-controls');
        if (hideControlsTimer) clearTimeout(hideControlsTimer);
        hideControlsTimer = setTimeout(function() {
          panel.classList.remove('show-controls');
          if (helpBox) helpBox.hidden = true;
        }, 3000);
      }
      if (panel) {
        panel.addEventListener('mousemove', showControls);
        panel.addEventListener('touchstart', showControls, {passive: true});
        panel.addEventListener('focusin', showControls);
      }
      var ovPrev = document.getElementById('ov-prev');
      var ovNext = document.getElementById('ov-next');
      var ovClose = document.getElementById('ov-close');
      var ovHelp = document.getElementById('ov-help');
      if (ovPrev) ovPrev.addEventListener('click', function() {
        var id = nextChannelId('up'); if (id) tune(id);
      });
      if (ovNext) ovNext.addEventListener('click', function() {
        var id = nextChannelId('down'); if (id) tune(id);
      });
      if (ovClose) ovClose.addEventListener('click', function() {
        stopPlayback();
        if (panel) panel.style.display = 'none';
        currentChannelId = null;
      });
      if (ovHelp) ovHelp.addEventListener('click', function() {
        if (helpBox) helpBox.hidden = !helpBox.hidden;
      });

      // ── buffering overlay (#33) ───────────────────────────────
      function showBuffering() {
        var el = document.getElementById('player-buffering');
        if (el) el.classList.add('show');
      }
      function hideBuffering() {
        var el = document.getElementById('player-buffering');
        if (el) el.classList.remove('show');
      }
      window.__showBuffering = showBuffering;
      window.__hideBuffering = hideBuffering;
      if (video) {
        video.addEventListener('playing', hideBuffering);
        video.addEventListener('error', hideBuffering);
      }
```

- [ ] **Step 6: Trigger buffering at load points**

In `templates/base.html`:
- In `_loadSource` (line 233), as the **first line** of the function body, add: `if (window.__showBuffering) window.__showBuffering();`
- In the HLS branch, inside the `MANIFEST_PARSED` callback (line 272-275), the `playing` event already hides it; no extra change needed.
- In `showPlayerError()` (line 338), add as the first line: `if (window.__hideBuffering) window.__hideBuffering();`
- In `enterWaitingState()` (line 386), add as the first line: `if (window.__hideBuffering) window.__hideBuffering();`
- In `advanceEndedChannel()` (line 366), add as the first line: `if (window.__hideBuffering) window.__hideBuffering();`

- [ ] **Step 7: Reset the panel display on tune**

In `tune()` (line 435, `document.getElementById('player-panel').style.display = 'block';`) — this already sets display to block, which correctly re-shows the panel after `ov-close` hid it. No change needed; verify it remains.

- [ ] **Step 8: Run the test to verify it passes**

Run: `cargo test --test http test_guide_has_player_overlay_toolbar`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add templates/guide.html templates/base.html
git commit -m "feat(ui): player overlay toolbar + buffering overlay"
```

---

## Task 8: Full regression + manual browser verification

**Files:** none (verification only)

- [ ] **Step 1: Format and lint**

Run:
```bash
cargo fmt
cargo clippy -- -D warnings
```
Expected: no diff, no warnings.

- [ ] **Step 2: Full test suite**

Run: `cargo test`
Expected: all tests pass (the four new assertions + the existing suite).

- [ ] **Step 3: Manual browser check**

Run `cargo run`, open `http://localhost:3000/guide`, and confirm:
- EPG nav bar and program blocks are neutral grey (no blue tint); pink-red accent retained on header, active tab, now-line.
- Time labels and program titles are slightly larger and legible.
- Switching category/time shows the shimmer skeleton briefly, then the grid (no blank flash).
- Tab through the page: focus rings appear on tabs, nav buttons, and program blocks; Enter/Space on a focused program tunes it.
- Tune a channel: buffering overlay (spinner + "Loading…") shows until playback starts.
- Hover the video: top toolbar fades in (✕ ↑ ↓ … ?), fades out after ~3s; prev/next change channel; ✕ stops and hides; ? toggles the shortcuts panel.
- Visit `/admin/discover` and run a search: inline spinner appears on the form.
- Confirm `/admin/channels` tab strip still looks correct (styling now from `app.css`).

- [ ] **Step 4: Final commit (if manual check required tweaks)**

```bash
git add -A
git commit -m "fix(ui): adjustments from manual verification"
```

(Skip if no changes were needed.)

---

## Self-Review Notes

- **Spec coverage:** #30 tokens → Tasks 1-3; #29 color normalization → Task 2; #29 font bumps → Task 2; #29 spinners → Task 6; #33 skeletons + buffering → Tasks 5, 7; #31 a11y (focus rings, keydown, buttons, aria) → Tasks 2, 4; #32 overlay → Task 7. Debug-panel gating intentionally excluded per spec.
- **Naming consistency:** `#epg-skeleton`, `#player-toolbar`, `#player-buffering`, `#player-help`, `.ov-btn`, `.spinner`, `.spinner-lg`, `.shim`, `--accent`/`--surface-*`/`--text-*` used consistently across tasks. `@keyframes spin` is defined in both `base.html` (Task 7) and `admin/base.html` (Task 6) — these are separate documents with no shared stylesheet for keyframes, so each needs its own; not a conflict.

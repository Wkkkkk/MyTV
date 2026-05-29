# Category Icons Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show an emoji category icon before each channel name in the EPG guide channel column.

**Architecture:** Add a `category_icon(&str) -> &'static str` helper to `routes/guide.rs`, add a `category_icon` field to `ChannelRow`, populate it in `build_guide_data`, and update the template to render it.

**Tech Stack:** Rust, Askama templates

---

### Task 1: Add `category_icon` helper and `ChannelRow` field

**Files:**
- Modify: `src/routes/guide.rs`

- [ ] **Step 1: Write the failing test**

Add this test inside the existing `#[cfg(test)] mod tests` block in `src/routes/guide.rs`:

```rust
#[test]
fn test_category_icon_known_categories() {
    assert_eq!(category_icon("News"), "📰");
    assert_eq!(category_icon("SPORTS"), "⚽");
    assert_eq!(category_icon("Movies"), "🎬");
    assert_eq!(category_icon("Films"), "🎬");
    assert_eq!(category_icon("cinema"), "🎬");
    assert_eq!(category_icon("Music"), "🎵");
    assert_eq!(category_icon("Kids"), "🧒");
    assert_eq!(category_icon("Children"), "🧒");
    assert_eq!(category_icon("Documentary"), "🎥");
    assert_eq!(category_icon("Docu"), "🎥");
    assert_eq!(category_icon("Entertainment"), "🎭");
    assert_eq!(category_icon("Cooking"), "🍳");
    assert_eq!(category_icon("Food"), "🍳");
    assert_eq!(category_icon("Travel"), "✈️");
    assert_eq!(category_icon("Science"), "🔬");
    assert_eq!(category_icon("Tech"), "🔬");
    assert_eq!(category_icon("Unknown"), "📺");
    assert_eq!(category_icon(""), "📺");
}
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cargo test test_category_icon_known_categories 2>&1
```

Expected: compile error — `category_icon` is not defined yet.

- [ ] **Step 3: Add the `category_icon` function and update `ChannelRow`**

In `src/routes/guide.rs`, replace the `ChannelRow` struct definition (lines 35–38):

```rust
pub struct ChannelRow {
    pub name: String,
    pub category_icon: &'static str,
    pub programs: Vec<ProgramSlot>,
}
```

Add this function directly below the `ChannelRow` struct (before the `// ── template structs` comment):

```rust
fn category_icon(category: &str) -> &'static str {
    let c = category.to_lowercase();
    if c.contains("news")                              { return "📰" }
    if c.contains("sport")                             { return "⚽" }
    if c.contains("movie") || c.contains("film") || c.contains("cinema") { return "🎬" }
    if c.contains("music")                             { return "🎵" }
    if c.contains("kid") || c.contains("child")        { return "🧒" }
    if c.contains("documentary") || c.contains("docu") { return "🎥" }
    if c.contains("entertainment")                     { return "🎭" }
    if c.contains("cooking") || c.contains("food")     { return "🍳" }
    if c.contains("travel")                            { return "✈️" }
    if c.contains("science") || c.contains("tech")     { return "🔬" }
    "📺"
}
```

- [ ] **Step 4: Update `build_guide_data` to populate the new field**

In `build_guide_data` (around line 198), update the `ChannelRow` construction:

```rust
rows.push(ChannelRow {
    name: ch.name.clone(),
    category_icon: category_icon(&ch.category),
    programs,
});
```

- [ ] **Step 5: Run tests to confirm all pass**

```bash
cargo test 2>&1
```

Expected: all 93 tests pass (92 existing + 1 new).

- [ ] **Step 6: Commit**

```bash
git add src/routes/guide.rs
git commit -m "feat: add category_icon helper and ChannelRow field"
```

---

### Task 2: Update the EPG template to display the icon

**Files:**
- Modify: `templates/partials/epg_content.html`

- [ ] **Step 1: Update the channel column in the template**

In `templates/partials/epg_content.html`, replace line 42:

```html
      <div class="channel-col">{{ row.name }}</div>
```

with:

```html
      <div class="channel-col">{{ row.category_icon }} {{ row.name }}</div>
```

- [ ] **Step 2: Build and verify it compiles**

```bash
cargo build 2>&1
```

Expected: `Finished` with no errors. Askama compiles templates at build time — any template variable mismatch will surface here.

- [ ] **Step 3: Run full test suite**

```bash
cargo test 2>&1
```

Expected: all 93 tests pass.

- [ ] **Step 4: Commit**

```bash
git add templates/partials/epg_content.html
git commit -m "feat: show category emoji icon in EPG guide channel column"
```

---

### Task 3: Deploy

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

- [ ] **Step 3: Verify in browser**

Open `https://kunstv.fly.dev/guide` and confirm emoji icons appear before channel names in the left column.

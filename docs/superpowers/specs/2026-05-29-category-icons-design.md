# Category Icons — Design Spec

**Date:** 2026-05-29
**Scope:** EPG guide only — show an emoji icon before each channel name in the channel column, derived from the channel's category string.

---

## What it does

Each channel row in the EPG guide gets a category-derived emoji prepended to the channel name (e.g. `⚽ BBC Sport`). Categories are free-text user strings; the mapping is keyword-based and case-insensitive. Unrecognised categories fall back to `📺`.

No per-channel logo field, no admin UI change, no external dependencies.

---

## Mapping

| Keyword(s) matched (case-insensitive) | Emoji |
|---|---|
| `news` | 📰 |
| `sport` | ⚽ |
| `movie`, `film`, `cinema` | 🎬 |
| `music` | 🎵 |
| `kid`, `child` | 🧒 |
| `documentary`, `docu` | 🎥 |
| `entertainment` | 🎭 |
| `cooking`, `food` | 🍳 |
| `travel` | ✈️ |
| `science`, `tech` | 🔬 |
| _(fallback)_ | 📺 |

---

## Implementation

### 1. Helper function — `routes/guide.rs`

```rust
fn category_icon(category: &str) -> &'static str {
    let c = category.to_lowercase();
    if c.contains("news")                          { return "📰" }
    if c.contains("sport")                         { return "⚽" }
    if c.contains("movie") || c.contains("film") || c.contains("cinema") { return "🎬" }
    if c.contains("music")                         { return "🎵" }
    if c.contains("kid") || c.contains("child")    { return "🧒" }
    if c.contains("documentary") || c.contains("docu") { return "🎥" }
    if c.contains("entertainment")                 { return "🎭" }
    if c.contains("cooking") || c.contains("food") { return "🍳" }
    if c.contains("travel")                        { return "✈️" }
    if c.contains("science") || c.contains("tech") { return "🔬" }
    "📺"
}
```

### 2. `ChannelRow` struct — `routes/guide.rs`

Add a `category_icon: &'static str` field to `ChannelRow`. Populated by calling `category_icon(&channel.category)` when constructing each row in `build_guide_data`.

### 3. Template — `templates/partials/epg_content.html`

In the channel column, prepend the icon to the channel name:

```html
{{ channel.category_icon }} {{ channel.name }}
```

---

## What is not in scope

- Per-channel logo URL field
- Admin UI changes
- Icon display anywhere other than the EPG guide channel column
- Any external icon library or CDN dependency

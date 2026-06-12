# Intake Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the duplicated channel/source/playlist validation rules into one deep `*Input::validate_*` per entity, co-located in `src/model/`, so form and JSON handlers share pure, unit-tested validators.

**Architecture:** Each entity gains an `Input` struct in its `src/model/*.rs` module holding transport-decoded fields. `validate_new`/`validate_update` consume `self` and return the existing `New*`/`Update*` types or a new `model::IntakeError`. All I/O (HTTP duration fetch, DB sort-order/existing lookups, transport string parsing) stays in the adapters. Form handlers map `IntakeError → 422`; JSON handlers map it via `From<IntakeError> for ApiError → ApiError::Validation`.

**Tech Stack:** Rust 1.96, Axum 0.7, SQLx 0.7, chrono. Tests via `cargo test`; pure validator unit tests live in each model file's existing `#[cfg(test)] mod tests`.

**Spec:** `docs/superpowers/specs/2026-06-12-intake-validation-design.md`

---

## File Structure

- `src/model/mod.rs` — **modify**: add `pub struct IntakeError(pub String)` + `Display`.
- `src/routes/api/mod.rs` — **modify**: add `impl From<IntakeError> for ApiError`.
- `src/model/channel.rs` — **modify**: add `ChannelInput` + `validate_new`/`validate_update`, private helpers (`validate_names`, `normalize_logo`, `parse_loop_anchor` moved here, `resolve_anchor`); unit tests.
- `src/model/source.rs` — **modify**: add `SourceInput` + validators; unit tests.
- `src/model/playlist_item.rs` — **modify**: add `PlaylistInput` + validators; unit tests.
- `src/routes/admin/channels.rs` — **modify**: `channel_create`/`channel_update` call `ChannelInput`; remove old `parse_loop_anchor`; add private `parse_sort_order`.
- `src/routes/api/channels.rs` — **modify**: `create`/`update` call `ChannelInput`; delete `parse_type`/`normalize_logo`/`resolve_anchor`/`validate_names` and the `parse_loop_anchor` import.
- `src/routes/admin/sources.rs` — **modify**: `source_create` calls `SourceInput`.
- `src/routes/api/sources.rs` — **modify**: `create`/`update` call `SourceInput`.
- `src/routes/admin/playlist.rs` — **modify**: `playlist_item_create` calls `PlaylistInput` after duration/sort_order resolution.
- `src/routes/api/playlist.rs` — **modify**: `create`/`update` call `PlaylistInput`.

---

## Task 1: `IntakeError` type and its `ApiError` mapping

**Files:**
- Modify: `src/model/mod.rs`
- Modify: `src/routes/api/mod.rs`

- [ ] **Step 1: Add `IntakeError` to `src/model/mod.rs`**

Insert after the existing `use` lines (after line 6), before `update_health_sql`:

```rust
/// A validation failure produced by the `*Input::validate_*` intake validators.
/// Carries a human-readable message; adapters decide the transport status code.
#[derive(Debug)]
pub struct IntakeError(pub String);

impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
```

- [ ] **Step 2: Add `From<IntakeError> for ApiError` in `src/routes/api/mod.rs`**

Insert after the `ApiError` `IntoResponse` impl (after line 45):

```rust
impl From<crate::model::IntakeError> for ApiError {
    fn from(e: crate::model::IntakeError) -> Self {
        ApiError::Validation(e.0)
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds clean (warnings about unused `IntakeError` are acceptable at this point).

- [ ] **Step 4: Commit**

```bash
git add src/model/mod.rs src/routes/api/mod.rs
git commit -m "feat(intake): add IntakeError + From<IntakeError> for ApiError"
```

---

## Task 2: `ChannelInput` validators (pure, unit-tested)

**Files:**
- Modify: `src/model/channel.rs` (add struct + impl after `UpdateChannel`/its `update` fn block, and tests in the existing `mod tests`)

- [ ] **Step 1: Write the failing unit tests**

Add to the bottom of the existing `#[cfg(test)] mod tests` in `src/model/channel.rs` (inside the closing brace), after the existing tests:

```rust
    #[test]
    fn validate_new_live_trims_and_drops_anchor() {
        let new = ChannelInput {
            name: "  CNN  ".into(),
            category: " news ".into(),
            channel_type: "live".into(),
            sort_order: 3,
            logo_url: Some("  ".into()),
            loop_anchor: Some("2021-01-01T00:00".into()),
        }
        .validate_new()
        .unwrap();
        assert_eq!(new.name, "CNN");
        assert_eq!(new.category, "news");
        assert_eq!(new.channel_type, ChannelType::Live);
        assert_eq!(new.sort_order, 3);
        assert_eq!(new.logo_url, None); // whitespace-only logo normalized away
        assert_eq!(new.loop_anchor, None); // live channel: no anchor even if supplied
    }

    #[test]
    fn validate_new_rejects_empty_name_and_bad_type() {
        let bad_name = ChannelInput {
            name: "   ".into(),
            category: "news".into(),
            channel_type: "live".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: None,
        }
        .validate_new();
        assert!(bad_name.is_err());

        let bad_type = ChannelInput {
            name: "CNN".into(),
            category: "news".into(),
            channel_type: "bogus".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: None,
        }
        .validate_new();
        assert!(bad_type.is_err());
    }

    #[test]
    fn validate_new_vod_parses_explicit_anchor() {
        let new = ChannelInput {
            name: "VOD".into(),
            category: "movies".into(),
            channel_type: "vod_loop".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: Some("2021-05-05T10:00".into()),
        }
        .validate_new()
        .unwrap();
        let anchor = new.loop_anchor.expect("vod_loop must have an anchor");
        assert_eq!(anchor.format("%Y-%m-%dT%H:%M").to_string(), "2021-05-05T10:00");
    }

    #[test]
    fn validate_new_vod_defaults_anchor_to_now_when_blank() {
        let new = ChannelInput {
            name: "VOD".into(),
            category: "movies".into(),
            channel_type: "vod_loop".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: Some("".into()),
        }
        .validate_new()
        .unwrap();
        assert!(new.loop_anchor.is_some()); // falls back to Utc::now()
    }

    #[test]
    fn validate_update_prefers_existing_anchor_when_blank() {
        let existing = DateTime::from_naive_utc_and_offset(
            NaiveDateTime::parse_from_str("2020-02-02T08:00", "%Y-%m-%dT%H:%M").unwrap(),
            Utc,
        );
        let upd = ChannelInput {
            name: "VOD".into(),
            category: "movies".into(),
            channel_type: "vod_loop".into(),
            sort_order: 0,
            logo_url: None,
            loop_anchor: None,
        }
        .validate_update(Some(existing))
        .unwrap();
        assert_eq!(upd.loop_anchor, Some(existing));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib channel::tests::validate_`
Expected: FAIL — `ChannelInput` / `validate_new` / `validate_update` not found.

- [ ] **Step 3: Add the `ChannelInput` struct, validators, and helpers**

First, update the chrono import at the top of `src/model/channel.rs` (line 2):

```rust
use chrono::{DateTime, NaiveDateTime, Utc};
```

Then add this block after the `update` fn (after the existing `UpdateChannel` `update` function, before the `#[cfg(test)]` module):

```rust
/// Raw, transport-decoded channel fields awaiting validation.
/// `sort_order` is already an `i64` (form adapter parses its string field first).
pub struct ChannelInput {
    pub name: String,
    pub category: String,
    pub channel_type: String,
    pub sort_order: i64,
    pub logo_url: Option<String>,
    pub loop_anchor: Option<String>,
}

fn parse_loop_anchor(s: &str) -> Option<DateTime<Utc>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc))
}

fn normalize_logo(logo: Option<String>) -> Option<String> {
    logo.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn validate_names(name: String, category: String) -> Result<(String, String), IntakeError> {
    let name = name.trim();
    let category = category.trim();
    if name.is_empty() || category.is_empty() {
        return Err(IntakeError("name and category are required".into()));
    }
    Ok((name.to_string(), category.to_string()))
}

fn resolve_anchor(
    channel_type: ChannelType,
    raw: Option<&str>,
    existing: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    if channel_type == ChannelType::VodLoop {
        raw.and_then(parse_loop_anchor)
            .or(existing)
            .or_else(|| Some(Utc::now()))
    } else {
        None
    }
}

impl ChannelInput {
    pub fn validate_new(self) -> Result<NewChannel, IntakeError> {
        let channel_type = self
            .channel_type
            .parse::<ChannelType>()
            .map_err(|_| IntakeError(format!("invalid channel type: {}", self.channel_type)))?;
        let (name, category) = validate_names(self.name, self.category)?;
        let loop_anchor = resolve_anchor(channel_type, self.loop_anchor.as_deref(), None);
        Ok(NewChannel {
            name,
            category,
            logo_url: normalize_logo(self.logo_url),
            channel_type,
            sort_order: self.sort_order,
            loop_anchor,
        })
    }

    pub fn validate_update(
        self,
        existing_anchor: Option<DateTime<Utc>>,
    ) -> Result<UpdateChannel, IntakeError> {
        let channel_type = self
            .channel_type
            .parse::<ChannelType>()
            .map_err(|_| IntakeError(format!("invalid channel type: {}", self.channel_type)))?;
        let (name, category) = validate_names(self.name, self.category)?;
        let loop_anchor = resolve_anchor(channel_type, self.loop_anchor.as_deref(), existing_anchor);
        Ok(UpdateChannel {
            name,
            category,
            logo_url: normalize_logo(self.logo_url),
            channel_type,
            sort_order: self.sort_order,
            loop_anchor,
        })
    }
}
```

Add the import for `IntakeError` near the top of `src/model/channel.rs` (after line 4's `use sqlx...`):

```rust
use super::IntakeError;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib channel::tests::validate_`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src/model/channel.rs
git commit -m "feat(intake): add ChannelInput validators with unit tests"
```

---

## Task 3: Rewire channel adapters to `ChannelInput`

**Files:**
- Modify: `src/routes/admin/channels.rs` (`channel_create`, `channel_update`; remove `parse_loop_anchor`; add `parse_sort_order`)
- Modify: `src/routes/api/channels.rs` (`create`, `update`; delete local helpers + import)

- [ ] **Step 1: Replace the form `parse_loop_anchor` with `parse_sort_order` in `src/routes/admin/channels.rs`**

Delete the `parse_loop_anchor` fn (lines 73–81) and replace the `// ── helpers ──` block with:

```rust
fn parse_sort_order(s: &str) -> Result<i64, StatusCode> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Ok(0)
    } else {
        trimmed
            .parse()
            .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)
    }
}
```

Update the top imports: remove `NaiveDateTime` from the chrono import (line 7) so it reads `use chrono::Utc;` (kept for `channel_detail`'s `Utc::now()`), and remove `DateTime` if no longer referenced. After edits, fix any unused-import warnings flagged by the compiler.

- [ ] **Step 2: Rewrite `channel_create`**

Replace the body of `channel_create` (lines 108–153) with:

```rust
pub async fn channel_create(
    State(state): State<AppState>,
    Form(form): Form<ChannelForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let sort_order = parse_sort_order(&form.sort_order)?;
    let new = channel::ChannelInput {
        name: form.name,
        category: form.category,
        channel_type: form.channel_type,
        sort_order,
        logo_url: Some(form.logo_url),
        loop_anchor: Some(form.loop_anchor),
    }
    .validate_new()
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    channel::create(&state.pool, new)
        .await
        .map_err(internal_error)?;

    Ok(Redirect::to("/admin/channels"))
}
```

- [ ] **Step 3: Rewrite `channel_update`**

Replace the body of `channel_update` (lines 179–233) with (existing-row lookup first so a valid update to a missing id still returns 404):

```rust
pub async fn channel_update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let existing = channel::get(&state.pool, id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let sort_order = parse_sort_order(&form.sort_order)?;
    let upd = channel::ChannelInput {
        name: form.name,
        category: form.category,
        channel_type: form.channel_type,
        sort_order,
        logo_url: Some(form.logo_url),
        loop_anchor: Some(form.loop_anchor),
    }
    .validate_update(existing.loop_anchor)
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    channel::update(&state.pool, id, upd)
        .await
        .map_err(internal_error)?;

    Ok(Redirect::to("/admin/channels"))
}
```

- [ ] **Step 4: Rewrite the JSON `create`/`update` and delete dead helpers in `src/routes/api/channels.rs`**

Delete the import `use crate::routes::admin::channels::parse_loop_anchor;` (line 10), and delete the four helper fns `parse_type`, `normalize_logo`, `resolve_anchor`, `validate_names` (lines 26–59). Remove the now-unused `use chrono::{DateTime, Utc};` (line 6). Replace `create` (lines 76–97) and `update` (lines 99–131) with:

```rust
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<ChannelRequest>,
) -> Result<(StatusCode, Json<channel::Channel>), ApiError> {
    let new = channel::ChannelInput {
        name: req.name,
        category: req.category,
        channel_type: req.channel_type,
        sort_order: req.sort_order,
        logo_url: req.logo_url,
        loop_anchor: req.loop_anchor,
    }
    .validate_new()?;
    let ch = channel::create(&state.pool, new).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(ch)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<ChannelRequest>,
) -> Result<Json<channel::Channel>, ApiError> {
    let existing = channel::get(&state.pool, id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    let upd = channel::ChannelInput {
        name: req.name,
        category: req.category,
        channel_type: req.channel_type,
        sort_order: req.sort_order,
        logo_url: req.logo_url,
        loop_anchor: req.loop_anchor,
    }
    .validate_update(existing.loop_anchor)?;
    let ch = channel::update(&state.pool, id, upd)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(ch))
}
```

- [ ] **Step 5: Run the channel integration tests**

Run: `cargo test --test http channel && cargo test --test api channel`
Expected: PASS — including `channel_create_rejects_invalid_type`, `channel_update_rejects_invalid_type`, `channel_update_returns_404_for_missing_channel`, `channel_create_empty_name_is_422`, `channel_update_preserves_existing_loop_anchor`.

- [ ] **Step 6: Build to confirm no dead-code/unused-import warnings**

Run: `cargo build 2>&1 | grep -E "warning|error" || echo CLEAN`
Expected: `CLEAN` (no warnings about unused `parse_loop_anchor`, `DateTime`, `NaiveDateTime`, or the deleted helpers).

- [ ] **Step 7: Commit**

```bash
git add src/routes/admin/channels.rs src/routes/api/channels.rs
git commit -m "refactor(intake): route channel form + JSON handlers through ChannelInput"
```

---

## Task 4: `SourceInput` validators (pure, unit-tested)

**Files:**
- Modify: `src/model/source.rs`

- [ ] **Step 1: Write the failing unit tests**

Add to the bottom of the existing `#[cfg(test)] mod tests` in `src/model/source.rs`:

```rust
    #[test]
    fn validate_new_parses_explicit_kind() {
        let new = SourceInput {
            kind: Some("dash".into()),
            url: "  https://x.example/s.mpd  ".into(),
            priority: 5,
        }
        .validate_new(7)
        .unwrap();
        assert_eq!(new.channel_id, 7);
        assert_eq!(new.kind, SourceKind::Dash);
        assert_eq!(new.url, "https://x.example/s.mpd");
        assert_eq!(new.priority, 5);
    }

    #[test]
    fn validate_new_detects_kind_when_absent_or_blank() {
        let detected = SourceInput {
            kind: None,
            url: "https://x.example/s.m3u8".into(),
            priority: 1,
        }
        .validate_new(1)
        .unwrap();
        assert_eq!(detected.kind, SourceKind::Hls);

        let blank = SourceInput {
            kind: Some("   ".into()),
            url: "https://youtu.be/abc".into(),
            priority: 1,
        }
        .validate_new(1)
        .unwrap();
        assert_eq!(blank.kind, SourceKind::YoutubeLive);
    }

    #[test]
    fn validate_new_rejects_empty_url_and_bad_kind() {
        assert!(SourceInput {
            kind: Some("hls".into()),
            url: "   ".into(),
            priority: 1,
        }
        .validate_new(1)
        .is_err());

        assert!(SourceInput {
            kind: Some("rtmp".into()),
            url: "https://x.example/s".into(),
            priority: 1,
        }
        .validate_new(1)
        .is_err());
    }

    #[test]
    fn validate_update_trims_url_ignores_kind() {
        let upd = SourceInput {
            kind: Some("rtmp".into()), // ignored on update
            url: "  https://x.example/s.m3u8  ".into(),
            priority: 9,
        }
        .validate_update()
        .unwrap();
        assert_eq!(upd.url, "https://x.example/s.m3u8");
        assert_eq!(upd.priority, 9);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib source::tests::validate_`
Expected: FAIL — `SourceInput` not found.

- [ ] **Step 3: Add the `SourceInput` struct and validators**

Add the `IntakeError` import near the top of `src/model/source.rs` (after line 3):

```rust
use super::IntakeError;
use std::str::FromStr;
```

Add this block after the `UpdateSource` `update` fn (before `set_active` or before the `#[cfg(test)]` module — anywhere among the item fns is fine):

```rust
/// Raw, transport-decoded source fields awaiting validation.
/// `kind` is `None`/blank when the caller wants it auto-detected from the URL.
pub struct SourceInput {
    pub kind: Option<String>,
    pub url: String,
    pub priority: i64,
}

impl SourceInput {
    pub fn validate_new(self, channel_id: i64) -> Result<NewSource, IntakeError> {
        let url = self.url.trim();
        if url.is_empty() {
            return Err(IntakeError("url is required".into()));
        }
        let url = url.to_string();
        let kind = match self.kind.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(k) => SourceKind::from_str(k).map_err(|e| IntakeError(e.to_string()))?,
            None => SourceKind::detect(&url),
        };
        Ok(NewSource {
            channel_id,
            kind,
            url,
            priority: self.priority,
        })
    }

    pub fn validate_update(self) -> Result<UpdateSource, IntakeError> {
        let url = self.url.trim();
        if url.is_empty() {
            return Err(IntakeError("url is required".into()));
        }
        Ok(UpdateSource {
            url: url.to_string(),
            priority: self.priority,
        })
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib source::tests::validate_`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/model/source.rs
git commit -m "feat(intake): add SourceInput validators with unit tests"
```

---

## Task 5: Rewire source adapters to `SourceInput`

**Files:**
- Modify: `src/routes/admin/sources.rs` (`source_create`)
- Modify: `src/routes/api/sources.rs` (`create`, `update`)

- [ ] **Step 1: Rewrite the form `source_create` in `src/routes/admin/sources.rs`**

Replace the body of `source_create` (lines 30–55) with (priority string parse stays here as a transport step):

```rust
pub async fn source_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    axum::extract::Form(form): axum::extract::Form<SourceForm>,
) -> Result<impl IntoResponse, StatusCode> {
    let priority: i64 = form.priority.trim().parse().unwrap_or(1);
    let new = source::SourceInput {
        kind: Some(form.kind),
        url: form.url,
        priority,
    }
    .validate_new(channel_id)
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    source::create(&state.pool, new)
        .await
        .map_err(internal_error)?;
    Ok(Redirect::to(&format!("/admin/channels/{channel_id}")))
}
```

- [ ] **Step 2: Rewrite the JSON `create`/`update` in `src/routes/api/sources.rs`**

Remove the now-unused `use std::str::FromStr;` (line 7). Replace `create` (lines 46–73) and `update` (lines 75–96) with:

```rust
pub async fn create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Json(req): Json<CreateSourceRequest>,
) -> Result<(StatusCode, Json<source::Source>), ApiError> {
    let new = source::SourceInput {
        kind: req.kind,
        url: req.url,
        priority: req.priority.unwrap_or(1),
    }
    .validate_new(channel_id)?;
    let src = source::create(&state.pool, new).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(src)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSourceRequest>,
) -> Result<Json<source::Source>, ApiError> {
    let upd = source::SourceInput {
        kind: None,
        url: req.url,
        priority: req.priority,
    }
    .validate_update()?;
    let src = source::update(&state.pool, id, upd)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(src))
}
```

- [ ] **Step 3: Run the source integration tests**

Run: `cargo test --test http source && cargo test --test api source`
Expected: PASS — including `source_create_rejects_invalid_kind`, `source_create_rejects_empty_url`, `source_create_empty_url_is_422`, `source_update_unknown_is_404`.

- [ ] **Step 4: Build to confirm no warnings**

Run: `cargo build 2>&1 | grep -E "warning|error" || echo CLEAN`
Expected: `CLEAN`.

- [ ] **Step 5: Commit**

```bash
git add src/routes/admin/sources.rs src/routes/api/sources.rs
git commit -m "refactor(intake): route source form + JSON handlers through SourceInput"
```

---

## Task 6: `PlaylistInput` validators (pure, unit-tested)

**Files:**
- Modify: `src/model/playlist_item.rs`

- [ ] **Step 1: Write the failing unit tests**

Add to the bottom of the existing `#[cfg(test)] mod tests` in `src/model/playlist_item.rs`:

```rust
    #[test]
    fn validate_new_trims_and_keeps_fields() {
        let new = PlaylistInput {
            title: "  Ep 1  ".into(),
            url: "  https://x.example/e1.mp4  ".into(),
            duration_secs: 1800,
            sort_order: 4,
        }
        .validate_new(7)
        .unwrap();
        assert_eq!(new.channel_id, 7);
        assert_eq!(new.title, "Ep 1");
        assert_eq!(new.url, "https://x.example/e1.mp4");
        assert_eq!(new.duration_secs, 1800);
        assert_eq!(new.sort_order, 4);
    }

    #[test]
    fn validate_new_rejects_empty_title_url_and_nonpositive_duration() {
        assert!(PlaylistInput {
            title: "   ".into(),
            url: "https://x.example/e.mp4".into(),
            duration_secs: 10,
            sort_order: 0,
        }
        .validate_new(1)
        .is_err());

        assert!(PlaylistInput {
            title: "Ep".into(),
            url: "  ".into(),
            duration_secs: 10,
            sort_order: 0,
        }
        .validate_new(1)
        .is_err());

        assert!(PlaylistInput {
            title: "Ep".into(),
            url: "https://x.example/e.mp4".into(),
            duration_secs: 0,
            sort_order: 0,
        }
        .validate_new(1)
        .is_err());
    }

    #[test]
    fn validate_update_trims_and_keeps_fields() {
        let upd = PlaylistInput {
            title: " Ep 2 ".into(),
            url: " https://x.example/e2.mp4 ".into(),
            duration_secs: 600,
            sort_order: 2,
        }
        .validate_update()
        .unwrap();
        assert_eq!(upd.title, "Ep 2");
        assert_eq!(upd.url, "https://x.example/e2.mp4");
        assert_eq!(upd.duration_secs, 600);
        assert_eq!(upd.sort_order, 2);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib playlist_item::tests::validate_`
Expected: FAIL — `PlaylistInput` not found.

- [ ] **Step 3: Add the `PlaylistInput` struct, validators, and helper**

Add the `IntakeError` import near the top of `src/model/playlist_item.rs` (after the existing `use` lines):

```rust
use super::IntakeError;
```

Add this block after the `UpdatePlaylistItem` `update` fn (before the `#[cfg(test)]` module):

```rust
/// Raw, transport-decoded playlist-item fields awaiting validation.
/// `duration_secs` and `sort_order` are already resolved by the adapter
/// (the form auto-fetches duration and derives sort_order from the DB max).
pub struct PlaylistInput {
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}

fn validate_title_url(title: String, url: String) -> Result<(String, String), IntakeError> {
    let title = title.trim();
    let url = url.trim();
    if title.is_empty() || url.is_empty() {
        return Err(IntakeError("title and url are required".into()));
    }
    Ok((title.to_string(), url.to_string()))
}

impl PlaylistInput {
    pub fn validate_new(self, channel_id: i64) -> Result<NewPlaylistItem, IntakeError> {
        let (title, url) = validate_title_url(self.title, self.url)?;
        if self.duration_secs <= 0 {
            return Err(IntakeError("duration_secs must be > 0".into()));
        }
        Ok(NewPlaylistItem {
            channel_id,
            title,
            url,
            duration_secs: self.duration_secs,
            sort_order: self.sort_order,
        })
    }

    pub fn validate_update(self) -> Result<UpdatePlaylistItem, IntakeError> {
        let (title, url) = validate_title_url(self.title, self.url)?;
        if self.duration_secs <= 0 {
            return Err(IntakeError("duration_secs must be > 0".into()));
        }
        Ok(UpdatePlaylistItem {
            title,
            url,
            duration_secs: self.duration_secs,
            sort_order: self.sort_order,
        })
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib playlist_item::tests::validate_`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/model/playlist_item.rs
git commit -m "feat(intake): add PlaylistInput validators with unit tests"
```

---

## Task 7: Rewire playlist adapters to `PlaylistInput`

**Files:**
- Modify: `src/routes/admin/playlist.rs` (`playlist_item_create`)
- Modify: `src/routes/api/playlist.rs` (`create`, `update`)

- [ ] **Step 1: Rewrite the form `playlist_item_create` in `src/routes/admin/playlist.rs`**

Keep the duration auto-fetch and `sort_order = max + 1` derivation (I/O, stays in the adapter), then build a `PlaylistInput`. Replace the body of `playlist_item_create` (lines 34–81) with:

```rust
pub async fn playlist_item_create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    axum::extract::Form(form): axum::extract::Form<PlaylistItemForm>,
) -> impl IntoResponse {
    let url = form.url.trim().to_string();
    let mut duration_secs: i64 = form.duration_secs.trim().parse().unwrap_or(0);
    if duration_secs <= 0 {
        match media::fetch_duration(&state.http_client, &url).await {
            Ok(d) => duration_secs = d,
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "failed to auto-fetch duration");
                return Html(format!(
                    r#"<p style="color:#e94560;padding:16px">Could not determine duration — enter it manually. <a href="/admin/channels/{channel_id}">← Go back</a></p>"#
                ))
                .into_response();
            }
        }
    }

    let existing = match playlist_item::list_for_channel(&state.pool, channel_id).await {
        Ok(items) => items,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let sort_order = existing
        .iter()
        .map(|i| i.sort_order)
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);

    let new = match (playlist_item::PlaylistInput {
        title: form.title,
        url,
        duration_secs,
        sort_order,
    })
    .validate_new(channel_id)
    {
        Ok(new) => new,
        Err(_) => return StatusCode::UNPROCESSABLE_ENTITY.into_response(),
    };

    if playlist_item::create(&state.pool, new).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Redirect::to(&format!("/admin/channels/{channel_id}")).into_response()
}
```

Note: the import `playlist_item::NewPlaylistItem` (line 13) is no longer referenced directly here — change line 13 to import only what remains used, i.e. drop `playlist_item::NewPlaylistItem` from the `use` if the compiler flags it unused.

- [ ] **Step 2: Rewrite the JSON `create`/`update` in `src/routes/api/playlist.rs`**

Replace `create` (lines 48–74) and `update` (lines 76–103) with:

```rust
pub async fn create(
    State(state): State<AppState>,
    Path(channel_id): Path<i64>,
    Json(req): Json<CreatePlaylistItemRequest>,
) -> Result<(StatusCode, Json<playlist_item::PlaylistItem>), ApiError> {
    let new = playlist_item::PlaylistInput {
        title: req.title,
        url: req.url,
        duration_secs: req.duration_secs,
        sort_order: req.sort_order.unwrap_or(0),
    }
    .validate_new(channel_id)?;
    let item = playlist_item::create(&state.pool, new)
        .await
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(item)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdatePlaylistItemRequest>,
) -> Result<Json<playlist_item::PlaylistItem>, ApiError> {
    let upd = playlist_item::PlaylistInput {
        title: req.title,
        url: req.url,
        duration_secs: req.duration_secs,
        sort_order: req.sort_order,
    }
    .validate_update()?;
    let item = playlist_item::update(&state.pool, id, upd)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(item))
}
```

- [ ] **Step 3: Run the playlist integration tests**

Run: `cargo test --test http playlist && cargo test --test api playlist`
Expected: PASS — including `playlist_item_create_sort_order_skips_gap_after_delete`, `playlist_create_zero_duration_is_422`.

- [ ] **Step 4: Build to confirm no warnings**

Run: `cargo build 2>&1 | grep -E "warning|error" || echo CLEAN`
Expected: `CLEAN`.

- [ ] **Step 5: Commit**

```bash
git add src/routes/admin/playlist.rs src/routes/api/playlist.rs
git commit -m "refactor(intake): route playlist form + JSON handlers through PlaylistInput"
```

---

## Task 8: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Format**

Run: `cargo fmt`
Then: `cargo fmt --check`
Expected: no diff.

- [ ] **Step 2: Clippy with CI's flags**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings, no errors.

- [ ] **Step 3: Full test suite**

Run: `cargo test`
Expected: all tests pass (the 382 existing + the 12 new validator unit tests; ignored tests stay ignored).

- [ ] **Step 4: Commit any fmt fixes**

```bash
git add -A
git commit -m "style(intake): cargo fmt" || echo "nothing to commit"
```

---

## Self-Review

**Spec coverage:**
- Interface (`ChannelInput`/`SourceInput`/`PlaylistInput` + `validate_new`/`validate_update`) → Tasks 2, 4, 6. ✅
- Rules each validator owns → Tasks 2 (channel), 4 (source), 6 (playlist). ✅
- `IntakeError` + `From<IntakeError> for ApiError` → Task 1. ✅
- Form maps to 422; JSON maps via `?` → Tasks 3, 5, 7. ✅
- I/O stays in adapters (duration fetch, sort_order-from-max, form string parse, existing lookup) → Tasks 3 (sort_order parse + existing lookup), 5 (priority parse), 7 (duration fetch + sort_order max). ✅
- Source kind provided→parse/absent→detect → Task 4 + test `validate_new_detects_kind_when_absent_or_blank`. ✅
- Behavior reconciliations (empty source kind → detect; empty playlist title → 422) → covered by Task 4/6 validators; no existing test pins the old lax behavior (verified: `source_create_rejects_invalid_kind` uses non-empty `rtmp`; no form test creates an empty title). ✅
- Pure-validator unit tests → Tasks 2, 4, 6. ✅
- Existing integration tests preserved → Tasks 3, 5, 7 run them; Task 8 runs the full suite. ✅
- Purity note (`Utc::now()` inline fallback) → Task 2 `resolve_anchor` + test `validate_new_vod_defaults_anchor_to_now_when_blank`. ✅

**Placeholder scan:** none — every code step shows complete code.

**Type consistency:** `IntakeError(pub String)` used uniformly; `ChannelInput`/`SourceInput`/`PlaylistInput` field names and `validate_new(...)`/`validate_update(...)` signatures match between definition tasks (2/4/6) and adapter tasks (3/5/7).

# Spec — Deepen channel/source/playlist intake validation

_2026-06-12 · MyTV (Rust/Axum IPTV) · Candidate #1 of the architecture review
(`docs/architecture/changes-20260612.md`), the only **Strong** pick._

## Problem

The same parse/trim/normalize/anchor validation rules are duplicated between the
**form** admin handlers and the **JSON** API handlers for all three intake entities:

- `src/routes/admin/channels.rs` (`channel_create:108`, `channel_update:179`) vs `src/routes/api/channels.rs`
- `src/routes/admin/sources.rs` (`source_create:30`) vs `src/routes/api/sources.rs`
- `src/routes/admin/playlist.rs` (`playlist_item_create:34`) vs `src/routes/api/playlist.rs`

The seam is already half-built: the JSON side extracted `parse_type` / `normalize_logo` /
`resolve_anchor` / `validate_names` and even imports `parse_loop_anchor` from the form module.
The two copies have begun to drift (`StatusCode` vs `ApiError`; inline vs extracted; and the
behavioral divergences in §"Behavior reconciliations" below). Validation bugs must be fixed
in two places, and the rules can only be tested through a full HTTP transport.

## Goal

One deep intake module per entity: a small interface (`validate_new` / `validate_update`)
behind which all the validation rules live. Adapters (form + JSON) shrink to
"decode transport → build Input → validate → map error → write". Rules become **pure,
directly unit-testable** functions — tested once, not through two transports.

## Decisions (from brainstorming)

1. **Seam scope — pure sync validators.** The intake validators own only pure rules
   (parse / trim / normalize / anchor / non-empty / range). All I/O (HTTP duration fetch,
   DB `sort_order` derivation, the existing-channel lookup, transport string parsing) stays
   in the adapters as explicit pre-steps. Validators take no `&pool` / `&http_client` and are
   not `async`.
2. **Module home — co-locate in `src/model/`.** `ChannelInput` lives in `model::channel`
   beside the `NewChannel`/`UpdateChannel` it produces; likewise `SourceInput` in
   `model::source` and `PlaylistInput` in `model::playlist_item`.
3. **Source `kind` — provided→parse, absent→detect.** Unify on the JSON side's richer rule:
   if `kind` is `Some(non-empty)`, parse it (error on invalid); otherwise
   `SourceKind::detect(&url)`.

## Interface

All three `Input` structs hold transport-decoded fields and consume `self`.

```rust
// model::channel
pub struct ChannelInput {
    pub name: String,
    pub category: String,
    pub channel_type: String,        // parsed inside
    pub sort_order: i64,             // adapter pre-parses (form String→i64, JSON passes i64)
    pub logo_url: Option<String>,    // normalized inside
    pub loop_anchor: Option<String>, // parsed/resolved inside
}
impl ChannelInput {
    pub fn validate_new(self) -> Result<NewChannel, IntakeError>;
    pub fn validate_update(self, existing_anchor: Option<DateTime<Utc>>)
        -> Result<UpdateChannel, IntakeError>;
}

// model::source — UpdateSource carries no kind/channel_id, so validate_update ignores kind
pub struct SourceInput {
    pub kind: Option<String>,
    pub url: String,
    pub priority: i64,
}
impl SourceInput {
    pub fn validate_new(self, channel_id: i64) -> Result<NewSource, IntakeError>;
    pub fn validate_update(self) -> Result<UpdateSource, IntakeError>;
}

// model::playlist_item — duration & sort_order already resolved by the adapter
pub struct PlaylistInput {
    pub title: String,
    pub url: String,
    pub duration_secs: i64,
    pub sort_order: i64,
}
impl PlaylistInput {
    pub fn validate_new(self, channel_id: i64) -> Result<NewPlaylistItem, IntakeError>;
    pub fn validate_update(self) -> Result<UpdatePlaylistItem, IntakeError>;
}
```

## Rules each validator owns

Lifted verbatim from today's handlers (no rule changes beyond the reconciliations below):

- **Channel:** trim + require `name` and `category`; parse `channel_type` via `FromStr`
  (error on invalid); normalize `logo_url` (trim, empty → `None`); resolve `loop_anchor` —
  for `vod_loop`: parsed-`loop_anchor` → else `existing_anchor` (update only) → else
  `Utc::now()`; for `live`: `None`.
- **Source:** trim + require `url`; resolve `kind` — `Some(non-empty)` → `SourceKind::from_str`
  (error on invalid), else `SourceKind::detect(&url)`. (`validate_update` validates `url` only;
  `UpdateSource` has no `kind`/`channel_id`.)
- **Playlist:** trim + require `title`; trim + require `url`; require `duration_secs > 0`.

## Error type & mapping

New `model::IntakeError` — a validation failure carrying a human-readable message:

```rust
pub struct IntakeError(pub String);   // + Display
```

Adapters map it:

- **Form handlers:** `.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)` — message discarded,
  matching today's form behavior.
- **JSON handlers:** one `impl From<IntakeError> for ApiError` (in `src/routes/api/mod.rs`,
  so `model` does not depend on `routes`) → `ApiError::Validation(msg)`, letting handlers
  use `?`.

`IntakeError` is exported via `model` (`src/model/mod.rs`) and made reachable per the
library-crate convention (`pub` in `lib.rs` where needed).

## What stays in the adapters (per decision #1)

- Form `sort_order` / `priority` **string** parsing with defaults (`empty → 0`,
  `parse().unwrap_or(1)`) — transport-specific; JSON passes typed values.
- Playlist **duration auto-fetch** (`media::fetch_duration` when `duration_secs <= 0`) and
  **`sort_order = existing.max + 1`** computation — both require I/O (HTTP / DB).
- Update handlers' DB lookup of the existing channel, to pass `existing_anchor` into
  `ChannelInput::validate_update`.

## Behavior reconciliations (intentional)

Unifying the seam makes the **form** adopt the JSON side's stricter rules in two spots.
Both are arguably bug fixes; TDD must surface any existing test that depends on the old lax
behavior, and such a test should be updated to assert the new (stricter) behavior.

1. **Empty source `kind`** → now auto-detects instead of returning 422. The form sends `kind`
   via a `<select>`, so this only changes behavior on malformed input.
2. **Empty playlist `title`** → now rejected with 422. Today the form silently creates an item
   with an empty title; the JSON API already rejects it.

## Testing

- **New:** pure unit tests on each `validate_new` / `validate_update` — no router, no DB, no
  HTTP. Cover every rule once: name/category required, `channel_type` parse + invalid,
  logo normalization, anchor resolution (live → `None`; vod_loop parsed / existing / now),
  source `kind` provided-vs-detected + invalid, url required, title required, `duration > 0`.
- **Preserved:** existing `tests/http.rs` and `tests/api.rs` integration tests must still pass
  unchanged, except the two cases in "Behavior reconciliations".

### Purity note

`validate_*` calls `Utc::now()` only as the `vod_loop` anchor *fallback* (matching today's
code). This stays inline rather than injecting a clock; tests that care pass an explicit
`loop_anchor`, consistent with the existing `tune_vod` tests' `url.contains(...)` philosophy.

## Out of scope

- Candidates #2–#5 of the architecture review (separate specs).
- Any change to the `model::*` DB-write functions or the `New*`/`Update*` struct shapes.
- The budget-badge row dance (candidate #5), even though it sits in the same handlers.

## Wins (from the review's deletion test)

Remove the module → validation reappears, copied, in every form and JSON handler. It earns
its keep: locality (validation bugs concentrate in one module), leverage (one interface, two
adapters, N fields), shrunken handlers, and rules tested once rather than through two
transports.

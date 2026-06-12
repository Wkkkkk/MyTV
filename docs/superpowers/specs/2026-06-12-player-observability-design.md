# Player Observability — Design (Spec 1 of 3)

**Date:** 2026-06-12
**Status:** Approved design, pending implementation plan
**Part of:** "Agent + E2E testing capability" (3-spec effort)

## Context

This is the first of three specs in a larger effort to make MyTV scriptable and
end-to-end testable against the live `kunstv.fly.dev` instance:

1. **Spec 1 — Player observability (this doc):** surface *which source/item* is
   playing, and add a shareable per-channel deep-link.
2. **Spec 2 — Admin automation:** a JSON `/api/admin` API + a `mytvctl` CLI client.
3. **Spec 3 — E2E suite:** prod-driving end-to-end tests (uses Spec 1's `source_id`
   for its strongest assertions).

Spec 1 is independent of Specs 2 and 3 — it shares only the model layer — and
ships on its own.

### Problem

When a viewer starts playback on `/guide`:

- **Channel** is known to the server (`/channel/:id/tune` carries the id) but is
  *not* reflected in the browser URL — there is no bookmarkable per-channel address.
- **Source** is known to *no one*: `next_live` picks the first resolvable source
  from `list_tunable_for_channel` and returns `TuneResponse { url, name, ... }`,
  which never names the chosen source. Neither the client, an agent, nor an E2E
  test can tell which source is playing or whether a fallback engaged.

### Goals

- Make "what am I watching" fully answerable: the tune payload names the source
  (live) or playlist item (VOD); the browser URL names the channel.
- Keep the change additive — the existing `/guide` flow, tune fields, and browser
  behavior are untouched unless a client opts into the new URL.

### Non-goals

- No JSON admin API and no CLI (Spec 2).
- No E2E tests (Spec 3).
- No change to source-selection logic, fallback order, or VOD positioning.

---

## D — Source/item observability in `TuneResponse`

### Change

Add three optional fields to `TuneResponse` (`src/routes/player.rs:21`):

```rust
pub source_id: Option<i64>,        // live: the source that resolved; None otherwise
pub source_url: Option<String>,    // live: that source's stored URL; None otherwise
pub playlist_item_id: Option<i64>, // vod_loop: the item being played; None otherwise
```

All three are `Option` because ended / waiting / failed responses have no active
source or item. Existing fields are unchanged; this is purely additive, so current
clients keep working and the JSON simply gains keys.

### Where each is set

The shared constructor `tune_response` (`player.rs:75`) gains parameters for the
new fields; the three response helpers set them as follows:

| Helper / path | `source_id` / `source_url` | `playlist_item_id` |
|---|---|---|
| `next_live` → `LiveOutcome::Play` (`player.rs:176`) | `Some(src.id)` / `Some(src.url.clone())` | `None` |
| `tune_vod_at` (`player.rs:319`), `next_vod_at` (`player.rs:341`) | `None` | `Some(item.id)` |
| `tune_response_ended` (`player.rs:94`) | `None` | `None` |
| `tune_response_waiting` (`player.rs:108`) | `None` | `None` |

In the `Play` branch `src` is already in scope; in the VOD paths `item` is already
in scope — no new queries, no new data flow.

### Client

`base.html`'s `tune()` already stores the whole payload in `currentChannel`
(`base.html:442`). No client change is required for D; the new fields are simply
available. (Optionally the info bar could show the source, but that is out of scope.)

### Tests

Unit/integration tests in `tests/http.rs`:

- Channel 1 (Live OK) tune → asserts `source_id`/`source_url` are present and match
  the seeded active source; `playlist_item_id` is null.
- Channel 4 (VOD Has Items) tune → asserts `playlist_item_id` is present and matches
  the active item; `source_id`/`source_url` are null.
- Channel 2 (All Down, 503) and a waiting case → fields are null (where a body exists).

Assertions parse the JSON body and check the new keys; existing assertions are
unaffected because no field was renamed or removed.

---

## E — `GET /watch/:channel_id` deep-link

### Behavior

A new **public** route (no auth — same visibility as `/guide`) that serves the
**same guide page**, pre-targeted to one channel:

- On load, the client auto-tunes the target channel (reusing the existing global
  `tune(id)` from `base.html:462`).
- The browser address bar shows `/watch/:id` via `history.replaceState` (no extra
  history entry, no reload).
- An unknown/invalid id **falls back to the normal guide** — auto-tune is skipped,
  no hard error. (The id is validated against `channel::get`; on `None`, render the
  plain guide.)
- The existing `/guide` route is untouched.

### Mechanism (avoids shared-template breakage)

`base.html` is extended by admin templates too, so the parent template cannot
reference a guide-only field. The id is therefore passed through a JS global emitted
*inside* `guide.html`'s content block, and read by `base.html`'s init:

1. `GuidePageTemplate` gains `auto_tune_channel_id: Option<i64>`.
   - `guide_page` sets it to `None`.
   - the new `watch_page` handler sets it to `Some(id)` only when `channel::get`
     confirms the id exists, else `None` (fallback).
2. In `guide.html`'s `{% block content %}`, an Askama `{% match auto_tune_channel_id %}`
   emits `<script>window.__autoTuneChannelId = {{ id }};</script>` only in the
   `Some` arm. Admin templates never reference this field, so they are unaffected.
3. At the end of `base.html`'s `DOMContentLoaded` init (after `window.tune = tune`,
   `base.html:462`), add:
   ```js
   if (window.__autoTuneChannelId) {
     var cid = window.__autoTuneChannelId;
     history.replaceState(null, '', '/watch/' + cid);
     tune(cid);
   }
   ```

### Route registration

In `src/lib.rs` (public router, alongside `/guide` at `lib.rs:122`):

```rust
.route("/watch/:id", get(routes::guide::watch_page))
```

`watch_page` lives in `src/routes/guide/mod.rs` next to `guide_page`: it builds the
same guide data, sets `auto_tune_channel_id` from a validated id, and renders
`GuidePageTemplate`.

### Tests

Integration tests in `tests/http.rs`:

- `GET /watch/1` → 200, body contains `window.__autoTuneChannelId = 1`.
- `GET /watch/999999` (unknown) → 200, body does **not** contain
  `window.__autoTuneChannelId` (clean fallback to guide).
- `GET /guide` → 200, still contains no `__autoTuneChannelId` (unchanged).

---

## Architecture & isolation

- **D** touches only `src/routes/player.rs` (struct + helper signatures + four
  call sites). No model, schema, or migration changes.
- **E** touches `src/lib.rs` (one route), `src/routes/guide/mod.rs` (one handler +
  one template field), `templates/guide.html` (one `{% match %}` block), and
  `templates/base.html` (one init block). No model or schema changes.
- D and E are independent of each other and can be implemented/committed separately.

## Error handling

- `/watch/:id` never 404s on a bad id — it degrades to the standard guide.
- Tune error/ended/waiting responses keep their existing status codes; the new
  fields are simply `null`.

## Out of scope (fast-follows)

- Showing the source/item in the info-bar UI.
- Persisting "currently watching" server-side.
- Per-source deep-links (`/watch/:channel/:source`).

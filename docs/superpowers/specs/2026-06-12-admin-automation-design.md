# Admin Automation — JSON API + `mytvctl` CLI — Design (Spec 2 of 4)

**Date:** 2026-06-12
**Status:** Approved design, pending implementation plan
**Part of:** "Agent + E2E testing capability" effort

## Context

Second of four specs making MyTV scriptable and end-to-end testable:

1. **Spec 1 — Player observability** ✅ (merged): `source_id`/`playlist_item_id` in `TuneResponse`, `/watch/:id` deep-link.
2. **Spec 2 — Admin automation (this doc):** a JSON `/api/admin` API + a `mytvctl` CLI client, covering full CRUD (incl. edit + test) for channels, sources, playlist items.
3. **Spec 3 — Discovery API + CLI:** YouTube/M3U search, URL resolve, add — builds on Spec 2's `/api/admin` router, `ApiError`, and `mytvctl` scaffolding.
4. **Spec 4 — E2E suite:** drives the live prod instance; exercises the CLI's real HTTP path.

### Problem

Today's admin endpoints take form-encoded input and respond with redirects/HTML — built for the browser, unusable by agents or scripts. The only JSON endpoint is `/admin/metrics`. We want a clean, machine-facing surface and a CLI so the user (and agents) can manage channels/sources/playlist items from a terminal, and so Spec 4's E2E suite has something to drive.

### Goals

- A JSON `/api/admin` API mirroring the admin CRUD, reusing the model layer so DB behavior never diverges from the browser flow.
- A `mytvctl` CLI binary that talks to a remote instance over HTTP, configured by env vars, emitting JSON.
- Add the two missing model capabilities (`source::update`, `playlist_item::update`) and expose a `test` (re-probe) operation.

### Non-goals

- Discovery (search/resolve/add) — Spec 3.
- E2E tests against prod — Spec 4.
- No change to the existing form routes, templates, or browser behavior.
- No new auth mechanism — reuse the existing `basic_auth`.

---

## Architecture

**New module `src/routes/api/`:**
- `mod.rs` — builds the `/api/admin` router; defines `ApiError` (the shared JSON error type); declares request DTOs or re-exports them from the submodules.
- `channels.rs`, `sources.rs`, `playlist.rs` — handlers per entity, thin wrappers over `model::*`.

**Router wiring** (`src/lib.rs`): an `api_router: Router<AppState>` nested with `.nest("/api/admin", api_router)`, carrying the **same** `basic_auth` `route_layer` the form admin router uses. The form `admin_router` is untouched. Both are added to the top-level router; `track_metrics` and `redirect_trailing_slash` layers already wrap everything.

**Responses reuse model structs.** `Channel`, `Source`, `PlaylistItem` already derive `Serialize` (with `FromRow`), so handlers return `Json<Channel>` / `Json<Vec<Source>>` etc. directly. No response DTOs.

**Requests use small DTOs.** The model `New*`/`Update*` types can't `Deserialize` (their `ChannelType`/`SourceKind` enum fields don't derive it). Each request DTO takes JSON-friendly types and converts to the model type, reusing existing parsing:
- `CreateChannelRequest` / `UpdateChannelRequest`: `{ name, category, logo_url?, type: "live"|"vod_loop", sort_order, loop_anchor? }`. Maps `type` → `ChannelType` (same string match the form handler uses), parses `loop_anchor` via the existing `parse_loop_anchor` helper (lifted from `routes/admin/channels.rs` to a shared spot, or duplicated minimally — see Open question below).
- `CreateSourceRequest`: `{ url (required), priority? (default 1), kind? }`. When `kind` is omitted, derive it with `Source::detect(&url)`. Explicit `kind` ("hls"|"youtube"|… per `SourceKind`) overrides.
- `UpdateSourceRequest`: `{ url?, priority? }` — only the editable fields (not `channel_id`, not health columns).
- `CreatePlaylistItemRequest`: `{ title, url, duration_secs, sort_order? }`.
- `UpdatePlaylistItemRequest`: `{ title?, url?, duration_secs?, sort_order? }`.
- `ToggleRequest`: `{ active: bool }`.

**Error model.** One `ApiError` enum implementing `IntoResponse`, rendering `{ "error": "<message>" }` with status:
- `NotFound` → 404 (unknown id on get/update/delete/toggle/test).
- `Validation(msg)` → 422 (empty URL, unparseable `type`, bad `loop_anchor`).
- `Internal` → 500 (DB/model error; logged via `tracing`).
Auth `401` is produced by the existing `basic_auth` middleware before handlers run.

**New model functions** (mirroring `channel::update`, returning `Result<Option<…>>`, `None` ⇒ 404):
- `source::update(pool, id, UpdateSource { url, priority }) -> Result<Option<Source>>`.
- `playlist_item::update(pool, id, UpdatePlaylistItem { title, url, duration_secs, sort_order }) -> Result<Option<PlaylistItem>>`.

(`UpdateSource` / `UpdatePlaylistItem` are model structs carrying the resolved values; the request DTOs above convert into them. For partial updates, the handler reads the current row, applies provided fields, and writes the full struct — simplest given SQLite and these small rows.)

## Endpoint surface (all under `/api/admin`, JSON in/out)

| Method | Path | Body → Returns |
|--------|------|----------------|
| GET | `/channels` | → `200 [Channel]` |
| POST | `/channels` | `CreateChannelRequest` → `201 Channel` |
| GET | `/channels/:id` | → `200 Channel` / `404` |
| PATCH | `/channels/:id` | `UpdateChannelRequest` → `200 Channel` / `404` |
| DELETE | `/channels/:id` | → `204` / `404` |
| GET | `/channels/:id/sources` | → `200 [Source]` |
| POST | `/channels/:id/sources` | `CreateSourceRequest` → `201 Source` |
| GET | `/sources/:id` | → `200 Source` / `404` |
| PATCH | `/sources/:id` | `UpdateSourceRequest` → `200 Source` / `404` |
| DELETE | `/sources/:id` | → `204` / `404` |
| POST | `/sources/:id/toggle` | `ToggleRequest` → `200 Source` / `404` |
| POST | `/sources/:id/test` | → `200 Source` (refreshed health) / `404` |
| GET | `/channels/:id/playlist` | → `200 [PlaylistItem]` |
| POST | `/channels/:id/playlist` | `CreatePlaylistItemRequest` → `201 PlaylistItem` |
| GET | `/playlist/:id` | → `200 PlaylistItem` / `404` |
| PATCH | `/playlist/:id` | `UpdatePlaylistItemRequest` → `200 PlaylistItem` / `404` |
| DELETE | `/playlist/:id` | → `204` / `404` |
| POST | `/playlist/:id/toggle` | `ToggleRequest` → `200 PlaylistItem` / `404` |
| POST | `/playlist/:id/test` | → `200 PlaylistItem` (refreshed health) / `404` |

**`test` behavior:** runs `health::probe_source` (source) / the playlist-item probe — which write `last_checked_at`/`last_status`/`consecutive_failures`/`failure_reason` to the row — then re-fetches and returns the entity. yt-dlp resolution goes through the existing capped `resolver::run_under_cap`, so `test` is naturally load-shed; no new concurrency handling.

## CLI — `mytvctl`

**Binary:** `src/bin/mytvctl.rs` (Cargo auto-detects; `cargo build` yields both `mytv` and `mytvctl`). Depends on the library crate for response structs. New dependency: `clap` (derive feature). `reqwest` (already present, `json` feature) is the HTTP client; `tokio` already present.

**Config** (resolved once at startup, in a small pure function so it's unit-testable):
- Base URL: `--base-url` flag, else `MYTV_BASE_URL` env, else `http://localhost:3000`.
- Password: `MYTV_ADMIN_PASSWORD` env **only** (never a flag — keeps secrets out of argv/history). Missing → print `set MYTV_ADMIN_PASSWORD` to stderr and exit `2`.
- Auth: HTTP Basic `user:<password>` (matches the server's check), over the base URL.

**Commands** (noun → verb, 1:1 with the API):
```
channel  list | get <id> | create --name --category --type <live|vod_loop> [--logo-url --sort-order --loop-anchor]
                          | update <id> [--name --category --type --logo-url --sort-order --loop-anchor]
                          | delete <id>
source   list --channel <id> | get <id>
                          | create --channel <id> --url [--priority --kind]
                          | update <id> [--url --priority] | delete <id>
                          | toggle <id> --active <true|false> | test <id>
playlist list --channel <id> | get <id>
                          | create --channel <id> --title --url --duration-secs [--sort-order]
                          | update <id> [--title --url --duration-secs --sort-order] | delete <id>
                          | toggle <id> --active <true|false> | test <id>
```

**Output:** always the API's JSON response body to stdout (success or `{"error":…}`). No second rendering path; humans pipe to `jq`.

**Exit codes:** `0` = 2xx; `1` = non-2xx HTTP (body still printed); `2` = usage/config error (clap handles most; plus missing password).

**Structure:** thin `main` (parse args → build `Client { base_url, password }` → dispatch) with HTTP calls in a focused `client` module, so `main` stays readable. Each subcommand builds request JSON from flags and prints the response. The flag→JSON mapping, config resolution, and status→exit-code mapping are pure functions for unit testing.

## Testing

**API — `tests/api.rs`** (new file; same harness as `http.rs`: `app()`, in-memory seeded DB, `tower::oneshot`). New JSON+auth request helpers (`authed_json_post`/`_patch`, etc.). Coverage:
- CRUD round-trip per entity: create → 201 + body has expected fields + new id; get → 200; list → contains it; update (PATCH) → 200 + changed fields; delete → 204; get-after-delete → 404.
- Auth: 401 without credentials (representative coverage: at least one GET, one POST, one DELETE).
- Validation: empty source URL → 422; bad channel `type` string → 422; unknown id on get/update/delete/toggle/test → 404.
- Source `kind` auto-detect: create source with a YouTube URL and no `kind` → returned `Source.kind` equals `Source::detect`'s result.
- Toggle: flips `is_active`; returned body reflects it.
- Test: `POST /sources/1/test` → 200, returned `Source.last_checked_at` is populated (seed URLs unreachable, so it records a probe result without real network success — mirrors existing `/admin/sources/:id/test` tests).

**Model unit tests** for `source::update` / `playlist_item::update` in their own modules (insert → update → re-fetch → assert; unknown id → `None`).

**CLI unit tests** (no network): flag→request-JSON mapping per subcommand; config resolution (missing password → exit 2; `--base-url` overrides env); status→exit-code mapping. The CLI's real HTTP path is covered in Spec 4 (E2E), avoiding a duplicate server spin-up inconsistent with the project's `oneshot` style.

## Open questions (resolved)

- **`parse_loop_anchor` reuse:** it currently lives private in `routes/admin/channels.rs`. The plan will make it `pub(crate)` and share it (rather than duplicate), since both the form handler and the API need identical anchor parsing.

## Out of scope (later specs/fast-follows)

- Discovery endpoints (Spec 3).
- Human-readable CLI table output (`--json` is the only mode; tables are a fast-follow if wanted).
- Bulk/batch operations.

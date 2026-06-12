# Discovery API + CLI — Design (Spec 3 of 4)

**Date:** 2026-06-12
**Status:** Approved design, pending implementation plan
**Part of:** "Agent + E2E testing capability" effort

## Context

Third of four specs making MyTV scriptable and end-to-end testable:

1. **Spec 1 — Player observability** ✅ (merged).
2. **Spec 2 — Admin automation** ✅ (merged): JSON `/api/admin` CRUD + `mytvctl` CLI.
3. **Spec 3 — Discovery API + CLI (this doc):** expose the discover subsystem (M3U/YouTube search, URL resolve, add) as JSON under `/api/admin/discover`, plus a `mytvctl discover` noun. Builds on Spec 2's `api_router`, `ApiError`, `ToggleRequest`-style DTO patterns, and `mytvctl` scaffolding.
4. **Spec 4 — E2E suite:** drives prod; exercises the CLI's real HTTP path.

### Problem

The discover subsystem (`src/routes/admin/discover/`, ~1,234 lines) is reachable only through HTML form endpoints returning partials — unusable by agents or scripts. It is the one admin capability Spec 2 deliberately deferred because it is a distinct, external/network-bound, search→candidate→add subsystem rather than CRUD.

### Goals

- Expose all four discovery producers as JSON: M3U search, YouTube search, manual URL resolve, YouTube channel URL resolve.
- Expose a dedicated JSON `discover/add` that reuses the existing `do_discover_add` orchestration (create-or-reuse channel + attach as source/playlist item, auto-fetching VOD duration).
- A `mytvctl discover` noun covering all five operations.
- Share logic with the existing HTML handlers (extract, don't duplicate).

### Non-goals

- No change to the HTML discover page/flow's behavior (only an internal refactor to share logic).
- No E2E tests against prod (Spec 4).
- No new auth mechanism — reuse the existing `basic_auth`.

---

## Architecture

**Refactor (pure extraction, no behavior change):** Lift the orchestration currently inlined in the HTML handlers into reusable functions in the `discover` module, so both the HTML handlers and the new JSON handlers call the same code:
- `discover::m3u::search(client, country: Option<&str>, group: &str, limit: usize) -> anyhow::Result<Vec<M3uResultRow>>` — lifts the body of `discover_m3u_search` (fetch → `parse_m3u` → `filter_m3u` → concurrent `url_is_reachable` → rows), now with a `limit` cap.
- `discover::resolve_manual(client, url: &str) -> Result<ResolvedMeta, StatusCode>` — lifts `discover_manual_resolve` (http(s) validation; for YouTube URLs, `resolver::fetch_duration_secs` + `resolver::fetch_title` with the existing 5s timeouts; else title=url, duration=0; `is_live = duration_secs == 0`; `source_kind = SourceKind::detect`).
- `discover::resolve_channel(url: &str) -> Result<ResolvedMeta, StatusCode>` — lifts `discover_channel_resolve` (`normalize_channel_url` → `channel_title_from_url`; `source_kind = youtube_live`, `is_live = true`).

`ResolvedMeta { url, title, duration_secs, is_live, source_kind }` is a plain struct in the discover module; the HTML handlers map it into their existing template structs, and the JSON layer maps it into `ResolvedCandidate`. `fetch_youtube_results`/`fetch_youtube_channels` (already reusable) are used as-is.

**New module `src/routes/api/discover.rs`:** JSON handlers wired into the Spec 2 `api_router()` under `/api/admin/discover/**`, behind the same `basic_auth` route-layer. Reuses `ApiError`. Defines clean candidate DTOs (no UI `form_id`). The `add` handler is a thin wrapper over `do_discover_add(DiscoverAddParams)`.

**`mod.rs` route wiring:** add the five discover routes to the existing `api_router()`.

---

## Endpoint surface (under `/api/admin/discover`, JSON out, behind basic_auth)

| Method | Path | Input → Returns |
|--------|------|-----------------|
| GET | `/discover/m3u?country=&group=&limit=` | → `200 [M3uCandidate]` |
| GET | `/discover/youtube?keyword=&type=` | → `200 [YoutubeCandidate]`; `503` if no API key |
| POST | `/discover/resolve` | `{ url }` → `200 ResolvedCandidate`; `422` bad URL |
| POST | `/discover/channel` | `{ url }` → `200 ResolvedCandidate`; `422` not a YT channel URL |
| POST | `/discover/add` | `AddRequest` → `201 AddResponse` |

**Query params:** `m3u` — `country` (name or 2-letter code, optional → global index), `group` (optional substring filter), `limit` (optional, default 50, hard cap 200). `youtube` — `keyword` (required), `type` (`video` default, or `channel`).

**Candidate DTOs** (serde `Serialize`):
- `M3uCandidate { name, group, country, url, source_kind }`
- `YoutubeCandidate { title, channel_title, is_live, is_upcoming, duration_secs, scheduled_start, thumbnail_url, url, source_kind }`
- `ResolvedCandidate { url, title, duration_secs, is_live, source_kind }`

**`AddRequest`** (serde `Deserialize`) — JSON-friendly mirror of `DiscoverAddParams`, with the channel target as a tagged enum so "existing" vs "new" is explicit and validated:
```jsonc
{
  "url": "https://...",
  "title": "Some title",
  "source_kind": "hls",            // hls|youtube_live|youtube_vod|iptv|dash
  "duration_secs": 0,               // optional; <=0 → auto-fetch for a vod_loop target
  "channel": { "existing_id": 4 }   // OR: { "new": { "name": "...", "category": "...", "type": "live" } }
}
```
Modeled as an **externally-tagged** enum (serde default) with `#[serde(rename_all = "snake_case")]` — `enum ChannelTarget { ExistingId(i64), New(NewChannelSpec) }` — which serializes/deserializes exactly as `{"existing_id": 4}` or `{"new": {...}}` (variant name as the key). `NewChannelSpec { name, category, #[serde(rename="type")] channel_type }`. The handler translates it to `DiscoverAddParams` (`channel_choice = "<id>"` or `"new"` + `new_name`/`new_category`/`new_channel_type`) and calls `do_discover_add`.

**`AddResponse`** `{ channel_id: i64, channel: Channel }` — after `do_discover_add` returns the id, re-fetch the channel and return it (so the caller sees the result without a second request). Status `201`.

**Errors:** all via `ApiError` → `{"error": "..."}`. `404`/`422`/`500` as in Spec 2; `503` specifically for "YOUTUBE_API_KEY not configured" (server-config, not client error — add an `ApiError` variant or map an internal one to 503). Validation: empty/non-http URL → 422; unknown existing channel id on add → 404 (from `do_discover_add`'s `NOT_FOUND`); bad `source_kind`/`type` → 422.

---

## CLI — `mytvctl discover`

A new `discover` subcommand noun, consistent with the existing channel/source/playlist nouns and slotting into the pure `request_for` mapping:

```
mytvctl discover m3u      --country <c> --group <g> [--limit <n>]
mytvctl discover youtube  --keyword <k> [--type <video|channel>]
mytvctl discover resolve  --url <url>
mytvctl discover channel  --url <url>
mytvctl discover add      --url <url> --title <t> --source-kind <k> [--duration-secs <n>]
                          ( --channel <id> | --new-name <n> --new-category <c> --new-type <live|vod_loop> )
```

- `m3u`/`youtube` → `GET` with a query string built from the flags (skip absent optionals).
- `resolve`/`channel` → `POST {"url": ...}`.
- `add` → `POST` an `AddRequest` body; the mutually-exclusive `--channel` vs `--new-*` flags map to the `channel` enum. clap enforces the group exclusivity (`--channel` conflicts with `--new-name`); the mapping in `request_for` builds `{"existing_id": id}` or `{"new": {...}}`.
- Output stays always-JSON; exit codes 0/1/2 unchanged.

---

## Testing

**Deterministic `tests/api.rs` (no network):**
- `discover/add` with `{"channel":{"existing_id":1}}` → 201, attaches a source to channel 1 (live), `AddResponse.channel_id == 1`; verify via `GET /channels/1/sources` that the source exists.
- `discover/add` with `{"channel":{"new":{...,"type":"live"}}}` → 201, a new channel is created and returned, with the source attached.
- `discover/youtube` with test config `youtube_api_key: None` → `503` + `{"error":"YOUTUBE_API_KEY not configured"}`.
- `discover/resolve` validation: non-http URL → 422; a plain non-YouTube `https://x/y.m3u8` → 200 with `{title==url, duration_secs==0, is_live==true, source_kind=="hls"}` (the yt-dlp branch only fires for YouTube URLs, so this is network-free and deterministic).
- `discover/channel`: a non-YouTube URL → 422; a valid `https://www.youtube.com/@handle` → 200 with `source_kind=="youtube_live"`, `is_live==true` (normalization is pure, no network).
- `discover/add` unknown existing channel id → 404.

**Network-gated (`#[ignore]`, matching the existing 7 network tests):** live M3U search (`discover/m3u` — fetches iptv-org + reachability HEADs), live YouTube search (needs a real key), and yt-dlp manual-resolve of a real YouTube URL. Marked `#[ignore = "requires network access — run manually"]`.

**CLI unit tests** (`src/bin/mytvctl.rs`): `request_for` mappings for the new `discover` subcommands — `m3u`/`youtube` produce the right GET path + query string; `resolve`/`channel` produce `POST {"url":...}`; `add` produces the right body with the `channel` enum for both existing-id and new-channel forms.

**Refactor safety:** the extraction of `m3u::search`/`resolve_manual`/`resolve_channel` is behavior-preserving; the existing HTML discover tests in `tests/http.rs` (e.g. the discover-page/add tests) must still pass unchanged.

---

## Open questions (resolved)

- **`form_id` field:** dropped from the JSON candidate DTOs (it is an HTML-form index, irrelevant to API clients).
- **M3U result size:** bounded by an explicit `limit` (default 50, cap 200) since the iptv-org index is large and each match triggers a reachability probe.
- **Search verbs:** GET for the read-only searches; POST for resolve/add (carry URLs/structured bodies).

## Out of scope (later/fast-follows)

- Spec 4 E2E.
- Caching discovery results server-side.
- Pagination beyond the simple `limit` cap on M3U search.

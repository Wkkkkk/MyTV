# Design: CORS Probe Descent + Manual Test-Button Trigger

**Date:** 2026-06-02
**Status:** Draft
**Idea:** `docs/IDEAS.md` #11 (A+B). VOD budget (#12 / C) is explicitly out of scope.

---

## Problem

The automatic CORS detection feature (see `2026-06-01-stream-proxy-cors-detection-design.md`) is effectively dead for the common case, so HTTPS+CORS channels never show the blue ⚡ "direct" badge in the guide — they stay blank (Unknown). Two distinct defects:

1. **The probe never populates the cache for master playlists.** The background health cycle calls `probe_cors_for_source` (`health.rs`), which fetches the source URL and calls `find_first_segment_url` to pick a segment to HEAD-probe. `find_first_segment_url` returns `None` for a *master* playlist (a manifest containing only `.m3u8` variant lines) — which is what most real HLS streams are. On `None`, the probe bails without writing `cors_cache`. So those channels stay Unknown forever unless tuned.

2. **The player writes the cache under the wrong key.** `resolve_direct_segments` in `player.rs` keys by the *variant-playlist* host (`base_url`), while the guide reads by the *source-URL* host. When a master returns no direct segment, the player returns `false` and writes nothing; the later variant proxy call writes under the variant host, which the guide never reads. So even tuning doesn't light up the guide badge.

Separately, the admin source table's **Health** dot is populated *only* by the 15-minute background checker. The **Test** button (`source_test`) does its own one-off HEAD and persists nothing, so clicking Test never moves the Health dot. There is no Budget indicator in the admin UI at all.

---

## Goal

- Make the CORS probe descend one level (master → first variant → first segment) so the background cycle, the player, and a new manual trigger all populate `cors_cache` for real-world master-playlist streams.
- Key the cache consistently by the **source-URL host** everywhere, so the guide budget badge reflects probes from any path.
- Turn the admin **Test** button into a manual trigger of the same per-source check the background cycle runs: it persists Health to the DB and probes CORS into the cache.
- Add a **Budget** column to the admin source table, rendered from `cors_cache`, using the same ⚡ / ☁ / blank icons as the guide.

Out of scope: VOD budget badges (VOD channels have no `sources` rows and no per-source Test button — idea #12).

---

## Decisions

| Decision | Choice |
|---|---|
| Cache key | Source-URL host everywhere (probe the descended segment on its CDN host, store/read under the top-level source-URL host) |
| Descent depth | Exactly one level (master → first variant → first segment). A source URL that is already a variant resolves at depth 0. Deeper nesting is not followed. |
| Test result persistence | Persist — Test reuses the background per-source check (writes `last_status` / `failure_reason` / `consecutive_failures`, applies auto-disable/re-enable, and probes CORS) |
| Row refresh mechanism | Whole-row swap via a shared `source_row.html` partial (`hx-swap="outerHTML"`) |

---

## Components

### 1. `probe_source_cors` — shared probe with 1-level descent

New function in `src/media/hls.rs`, built on the existing `find_first_segment_url`, `probe_cors`, and `has_cors_wildcard`:

```rust
/// Determines whether segments for `source_url` can be fetched directly by the browser.
/// `Some(true)` = direct (HTTPS segment with `Access-Control-Allow-Origin: *`).
/// `Some(false)` = must proxy (HTTP segment, or HTTPS segment without CORS).
/// `None` = could not determine (network error, or no segment found after one descent) — leave cache untouched.
pub async fn probe_source_cors(client: &reqwest::Client, source_url: &str) -> Option<bool>
```

Algorithm:
1. GET `source_url` (use `HTTP_TIMEOUT`). On error → `None`.
2. `find_first_segment_url(&body, source_url)`:
   - `Some(seg)` → the source URL was already a variant; use `seg`.
   - `None` → it's a master. Find the first non-comment line whose path ends in `.m3u8`/`.m3u` (the first variant), resolve it to an absolute URL, GET it (one descent), then `find_first_segment_url` on that body. Still `None` → return `None`.
3. With a segment URL:
   - `http://` → `Some(false)` (mixed content, always proxied; no HEAD needed).
   - `https://` → `probe_cors(client, seg).await` → `Some(bool)`.

Finding the first variant line reuses the same resolution rules as `find_first_segment_url` (absolute / root-relative / dir-relative). Factor a small private helper `resolve_manifest_line(line, base_url) -> String` shared by both if it reduces duplication; otherwise inline.

### 2. `extract_manifest_host` consolidation

`extract_manifest_host` is currently duplicated verbatim in `health.rs` and `player.rs`. Move one copy to `src/media/hls.rs` as `pub fn extract_manifest_host(url: &str) -> String` and have both callers use it. This is the canonical cache-key derivation.

### 3. `health::check_source` — unified per-source check

Promote the per-source work into one public function:

```rust
pub async fn check_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    src: &Source,
)
```

It performs the existing `check_one` work (HTTP check + `source::update_health` with failure counting and auto-disable/re-enable) **and**, when `src.url` starts with `https://`, calls `probe_source_cors` and writes the result to `cors_cache` keyed by `extract_manifest_host(&src.url)` (only on `Some`).

- The background loop's `check_all` calls `check_source` per source (replacing the current `check_one` + `probe_cors_for_source` pair).
- `source_test` calls `check_source`.

The now-redundant `probe_cors_for_source` is removed; its descent-less body is superseded by `probe_source_cors`.

### 4. Player cache-key fix

`resolve_direct_segments` in `src/routes/player.rs`:
- Today: if `find_first_segment_url(content, base_url)` is `None`, return `false` (no cache write).
- New: when it's `None` (the proxied body is a master), descend one level — find the first variant line in `content`, fetch it, and `find_first_segment_url` on that body. If a segment is found, probe and proceed.
- Cache key stays `extract_manifest_host(base_url)`. On the first tune the proxied URL is the master/source URL, so this writes the **source-host** key the guide reads.

To avoid re-fetching the manifest that `stream_proxy` already has in hand, `resolve_direct_segments` keeps operating on the already-fetched `content`; only the one-level variant fetch is a new request. The existing read-cache-first short-circuit (`if let Some(&cached) = cache.get(&host_key)`) is preserved.

### 5. `BudgetStatus` shared module

Move `BudgetStatus`, `derive_budget_status`, and `budget_badge` out of `routes/guide.rs` into a small shared module (e.g. `src/budget.rs`, exported from `lib.rs`), so both the guide and the admin source table render identical icons (`⚡` direct / `☁` proxied / blank unknown). `routes/guide.rs` imports them from there; its existing behaviour and unit tests are unchanged (tests move with the functions or import from the new path).

### 6. Admin source row: Budget column + row partial

**`AdminSourceRow`** (`routes/admin/mod.rs`) gains a `budget_status: BudgetStatus` field. The `From<Source>` impl cannot compute it (no cache access), so it sets `budget_status: BudgetStatus::Unknown`; the handler (`channel_detail` and `source_test`) then overwrites it with the cache-derived value in a follow-up step. `BudgetStatus` derives `Clone`/`Copy` (it already does in `guide.rs`).

**`channel_detail` handler** (`routes/admin/channels.rs`) reads a `cors_cache` snapshot (`state.cors_cache.read().await.clone()`) and, for each source row, sets `budget_status` via `derive_budget_status` keyed by `extract_manifest_host(&src.url)` (HTTP source URL → Proxied without a cache lookup, mirroring the guide's rule).

**Row partial** `templates/admin/partials/source_row.html` renders one full `<tr id="src-row-{{ src.id }}">` with cells: Kind, URL, Priority, Active, Health dot, **Budget badge**, actions (Toggle / Delete / Test). It references a single variable `src` (an `AdminSourceRow`).

**`channel_detail.html`**:
- Table header gains a `<th>Budget</th>` between `Health` and the empty actions header.
- The `{% for src in sources %}` loop body becomes `{% include "admin/partials/source_row.html" %}` (the loop variable `src` is visible to the include).
- The Test form moves to `hx-target="#src-row-{{ src.id }}"`, `hx-swap="outerHTML"`. The orphan `<span id="src-test-{{ src.id }}">` and the `OK` / `Failed` HTML strings are removed.

**`source_test` handler** (`routes/admin/sources.rs`): change signature to take `State<AppState>` (already present) and `Path<i64>`. Body:
1. `source::get` the source (404 if missing).
2. `health::check_source(&state.pool, &state.http_client, &state.cors_cache, &src)`.
3. Re-fetch the updated source (`source::get` again) to pick up the persisted `last_status` etc.
4. Build an `AdminSourceRow`, derive its `budget_status` from a fresh `cors_cache` snapshot.
5. Render a standalone `SourceRowTemplate { src }` using the same `source_row.html` partial, and return it as `Html<String>`.

The previous `resolver::needs_resolution` / `resolve_url` branch in `source_test` is dropped; `check_source`'s HTTP check already handles `youtube_live` sources (it returns healthy on a successful GET without reading the body, per `do_http_check`). yt-dlp/YouTube sources get Health from the check and a blank Budget (no static segment to probe → Unknown).

---

## Data Flow

**Background cycle (every 15 min):** `check_all` → for each source `check_source` → persists Health to DB + writes `cors_cache[source_host] = bool` for HTTPS sources.

**Guide render:** `build_guide_data` reads `cors_cache` snapshot, derives `BudgetStatus` per channel from the first active source URL's host — now populated by the descent probe.

**First tune:** `stream_proxy` proxies the master (= source URL) → `resolve_direct_segments` descends, probes, writes `cors_cache[source_host]`.

**Admin Test click:** `source_test` → `check_source` → DB Health updated + `cors_cache` updated → returns refreshed `<tr>` with new Health dot and Budget badge.

**Admin page load:** `channel_detail` reads `cors_cache` snapshot, derives Budget per source row.

---

## What Does Not Change

- `cors_cache` type stays `Arc<RwLock<HashMap<String, bool>>>`; absence = Unknown, so `None` probes write nothing.
- Database schema — no migration. Budget remains a runtime signal.
- Guide badge markup and `HealthStatus`/health-dot logic.
- HTTP sources: always Proxied, no probe, no cache lookup (both guide and admin).
- Descent never exceeds one level; deeper-nested or looping manifests resolve to `None` (Unknown), never an infinite loop.

---

## Testing

**Unit (`src/media/hls.rs`):**
- `probe_source_cors`: variant source URL with HTTPS+CORS segment → `Some(true)`; HTTP segment → `Some(false)`; master that descends one level to a segment → probes the segment; master whose variant still has no segment → `None`; manifest fetch error → `None`. (Mock the `reqwest::Client` / use a local test server as existing HLS tests do.)
- `extract_manifest_host`: scheme+host extraction, trailing-path stripping, no-path URL.

**Unit (shared budget module):**
- `derive_budget_status`: HTTP source URL → Proxied; HTTPS host cache-hit `true` → Direct; `false` → Proxied; cache-miss → Unknown. (Moved from `guide.rs`, behaviour unchanged.)

**Integration (`tests/http.rs`):**
- `POST /admin/sources/:id/test` persists `last_status` (assert via a follow-up read or guide/admin render) and returns a `<tr id="src-row-…">` containing a Health dot; assert the response no longer contains the literal `OK` badge that the old handler returned.
- Guide route renders the correct budget badge for a channel whose source host is seeded into `cors_cache`.

---

## Files Changed

| File | Change |
|---|---|
| `src/media/hls.rs` | Add `probe_source_cors` (1-level descent); add `extract_manifest_host`; optional `resolve_manifest_line` helper |
| `src/health.rs` | Add `pub check_source` (check + CORS probe); `check_all` calls it; remove `probe_cors_for_source` and the local `extract_manifest_host` |
| `src/routes/player.rs` | `resolve_direct_segments` descends one level on master; use shared `extract_manifest_host` |
| `src/budget.rs` (new) + `src/lib.rs` | Move `BudgetStatus` / `derive_budget_status` / `budget_badge` here; export from `lib.rs` |
| `src/routes/guide.rs` | Import budget items from the shared module (remove local defs) |
| `src/routes/admin/mod.rs` | `AdminSourceRow` gains `budget_status` |
| `src/routes/admin/channels.rs` | `channel_detail` reads `cors_cache`, derives per-source budget |
| `src/routes/admin/sources.rs` | `source_test` calls `check_source`, returns the row partial |
| `templates/admin/partials/source_row.html` (new) | One `<tr>` with Health + Budget cells and actions |
| `templates/admin/channel_detail.html` | `Budget` header; loop includes the partial; Test form targets the row; remove OK/Failed span |
| `tests/http.rs` | Integration tests for Test-button persistence + row render, and guide budget badge |

# Changelog

Completed ideas, migrated from `docs/ideas.md`. Each entry keeps its original
idea number and its completion note. Deeper rationale lives in the matching spec
under `docs/superpowers/specs/` and in git history; the active backlog now lives
in `docs/ideas.md`. New completions are appended here.

## Foundational work

Early infrastructure and tooling, completed before the numbered backlog settled.

- **Source health monitoring** — done: background checker, health badges in admin, guide warnings
- **yt-dlp auto-update** — done: `.github/workflows/ytdlp-update.yml` weekly PR workflow
- **Player UX + mobile/TV UI** — done: channel info bar (logo, name, category, position), keyboard shortcuts (space/arrows/F), fullscreen, responsive CSS for phone and TV, channel logos
- **Markdown docs with Mermaid charts** — done: architecture docs in `docs/architecture/` covering request flow, health checker, tune flow, yt-dlp resolution, database ER diagram
- **SQL indexes on foreign keys** — done: compound indexes `(channel_id, priority)` on `sources` and `(channel_id, sort_order)` on `playlist_items`
- **Multi-agent PR bug review** — done: `pr-bug-review` skill with parallel correctness/security/architecture agents + synthesis pass
- **Code quality review checklist** — done: `self-review` skill with 13 universal checks (KISS/DRY, HTML a11y, SQL indexes, N+1, dead code, test gaps)
- **Source auto-re-enable after cooldown** — done: disabled sources are automatically re-enabled on the first passing health check (`HealthAction::Reenable`, no schema change needed)

## Feature backlog (#9–#42)

### #9 — Favicon and PWA manifest

done: TV+Play SVG favicon at `/favicon.svg`, PWA manifest at `/manifest.json`, both linked from all pages via `<head>` tags.

### #10 — Stream proxy: automatic CORS detection

done: HLS segments load directly from origin CDN when the CDN sends `Access-Control-Allow-Origin: *`; manifests always proxy. CORS probed on first tune and on each 15-min health check cycle; cached per `scheme://host`. EPG guide shows health (green/red/grey ●) and budget (blue ⚡ / amber ☁) badges per channel.

### #11 — CORS probe: descend into master playlists + manual Test-button trigger

done: the CORS probe now descends one level (master → first variant `.m3u8` → first segment → HEAD-probe) in `media::hls::probe_source_cors`, the cache is keyed by source-URL host everywhere (`extract_manifest_host`), `health::check_source` unifies the background health check + CORS probe, and the admin **Test** button runs it and swaps the whole source row showing updated Health + a new Budget column (⚡/☁/blank). Spec: `docs/superpowers/specs/2026-06-02-cors-probe-descent-test-trigger-design.md`.

### #12 — CORS budget badge for VOD channels

done: `build_guide_data` derives a VOD channel's budget badge from its currently-playing playlist item (`vod_budget_url` + `playlist_item::current_position`); the CORS cache for item hosts is warmed by a per-item admin **Test** button (`POST /admin/playlist/:id/test`) and a new background-checker sweep (`probe_all_playlist_cors`, deduped by host). The shared `health::probe_and_cache_cors` helper (skips non-HTTPS and youtube/twitch URLs) backs sources, the Test button, and the sweep. Spec: `docs/superpowers/specs/2026-06-02-vod-budget-badge-design.md`.

### #13 — Stream-proxy SSRF / open-proxy hardening

done: `src/ssrf.rs` with `is_safe_url` (DNS resolve + IP range blocking for loopback/private/link-local/IPv6-ULA); `proxy_client` in `AppState` with `redirect::Policy::none()`; `stream_proxy` handler uses a manual redirect loop (max 5 hops) with per-hop SSRF check + 20 MB body cap. 13 tests (10 unit + 3 integration). — `/stream-proxy` (`routes/player.rs`) is on the public, unauthenticated router and fetches any `?url=` after only checking the `http(s)://` prefix, returning the body with `Access-Control-Allow-Origin: *`. Anyone on the internet can use the Fly instance as an open proxy and probe the Fly internal network or cloud metadata (`169.254.169.254`, `127.0.0.1`, `[::1]`). Fix: resolve the target host and reject private/loopback/link-local IP ranges (and re-check on redirects), plus cap the buffered response body size to avoid OOM on large upstreams.

### #14 — Code-health refactors for `discover.rs` and `guide.rs`

done: `discover.rs` split into `discover/{mod,add,youtube,m3u}.rs` (handlers/templates vs. DB-add core vs. YouTube API vs. iptv-list+country-map), `guide.rs` split into `guide/{mod,layout,badges,data}.rs` (handlers/templates vs. EPG geometry vs. health/budget derivation vs. DB aggregation, plus deduped query-parse + GuideData→template field copy). Also collapsed the four `tune_*`/`next_*` `TuneResponse` builders in `player.rs` to one `tune_response` + shared live/vod helpers, and extracted a single `resolve_uri`/`origin_of` in `media/hls.rs` (3 copies → 1). No behavior change; all 117 tests still pass. Spec: `docs/superpowers/specs/2026-06-02-code-health-refactors-design.md`.

### #15 — Streaming proxy: cut first-byte latency and peak memory

done: `Body::from_stream(upstream.bytes_stream())` for segments (pipeline CDN→browser); `SsrfCache` (`Arc<RwLock<HashMap<String, Instant>>>`) skips DNS re-check for 60 s; `proxy_client` split into connect (5 s) + read (30 s) timeouts. Spec: `docs/superpowers/specs/2026-06-03-streaming-proxy-design.md`.

### #16 — Proxy response fidelity

done: upstream HTTP status passed through; all upstream response headers forwarded (except `access-control-allow-origin` which we own); `content-length` removed on playlist path (body length changes after URL rewriting); browser `Range` header forwarded to CDN on every redirect hop. Spec: `docs/superpowers/specs/2026-06-03-proxy-response-fidelity-design.md`.

### #17 — Onboarding dry-run

done: simulated new-contributor flow; found 5 friction points (stale Rust version requirement, buried git-hooks step, `--release` in quick start, missing `cargo fmt` callout, no test-suite context); all fixed in `docs/SETUP.md` and `README.md`.

### #18 — DASH stream support

the current player uses a native `<video>` element which only plays HLS natively in Safari; DASH (`.mpd`) streams require a JS player library (e.g. dash.js or Shaka Player). Needed to tune DASH sources without transcoding.

### #19 — Fix DASH stream addition

We have some problems adding a DASH stream to a channel. When we hit add, we see a blank page returned. Mark this as to fix.

### #20 — Unify Source and Playlist

currently live channels have `sources` with a `url`, while VOD channels have `playlist_items` with a `vod_url`. We could unify these into a single `MediaItem` table with a `type` (live/vod) and optional fields, simplifying the data model and code paths. And both channels should display the same duration/health/budget badges. It should be possible to mix HLS and DASH items in the same channel, so the player needs to support both per-item as well.

### #21 — DASH stream health checking

done: basic health checking (up/down, auto-disable/re-enable) was already format-agnostic; added `find_mpd_probe_url` + `probe_mpd_cors` in `media/mpd.rs` to extract the effective CDN origin from an MPD (`<BaseURL>` → `<SegmentURL media>` → `<SegmentTemplate initialization>` → MPD directory) and HEAD-probe it for CORS; `probe_and_cache_cors` in `health.rs` now routes `.mpd` URLs through the new DASH prober so DASH sources and VOD items get budget badges (⚡/☁) like HLS.

### #22 — DASH stream proxying

already done as part of idea 18: `stream_proxy` in `routes/player.rs` detects DASH by `Content-Type: application/dash+xml` or `.mpd` URL, buffers the body, calls `mpd::rewrite_mpd_urls` to rewrite `<BaseURL>`, `<SegmentURL media>`, and `<SegmentTemplate media/initialization>` through `/stream-proxy`, and sets the correct `Content-Type`. Covered by integration test `test_stream_proxy_rewrites_dash_bbb_manifest`.

### #23 — Health checker refactor

two intertwined code-quality issues surfaced by PR review: (1) `check_source`/`probe_source` and `check_playlist_item`/`probe_playlist_item` are four near-identical functions; the shared lifecycle (do_http_check → update_health → log action → probe CORS) should be extracted into a generic helper parameterised by whether `is_active` may change — four coordinated edits are required today for any health-check behavior change. (2) Host deduplication was removed when playlist-item health checking was added — every item in a VOD channel now generates an individual HTTP check + CORS HEAD probe per 15-minute cycle; a channel with N episodes on one CDN sends N requests instead of 1; the old `probed_hosts` dedup set should be restored for the CORS sweep step.

### #24 — Proxy `#EXT-X-KEY` URLs in HLS manifests

`rewrite_tag_uri` in `media/hls.rs` only rewrites `URI=` attributes on `#EXT-X-MEDIA`, `#EXT-X-MAP`, `#EXT-X-I-FRAME-STREAM-INF`, and `#EXT-X-SESSION-DATA` tags; `#EXT-X-KEY:URI="…"` (AES-128 encryption key URL) is not in the whitelist, so the key request would go direct to the CDN rather than through `/stream-proxy`, causing decryption failures in the player for encrypted HLS streams.

### #25 — Fix DASH CORS cache key mismatch

`probe_and_cache_cors` caches the CORS result under `hls::extract_manifest_host(mpd_url)` (the MPD manifest origin), but `probe_mpd_cors` probes the segment CDN discovered inside the MPD (via `<BaseURL>` / `<SegmentURL>` etc.), which may be a different host; `budget::status_for_url` then looks up by the media URL's host and returns Unknown for streams where manifest host ≠ segment CDN host. Fix: cache under the probed URL's host instead of the MPD URL's host.

### #26 — Cap MPD fetch body size in `probe_mpd_cors`

`probe_mpd_cors` in `media/mpd.rs` buffers the entire MPD response with no size limit (`.text().await`), while the stream proxy caps bodies at 20 MB; a misbehaving server could return an arbitrarily large response and exhaust memory. Fix: add a body-size guard consistent with the proxy cap, or switch to streaming XML parsing.

### #27 — Small DRY cleanups in `media/`

two minor duplications flagged by PR review: (1) `pct_encode_template` in `mpd.rs` reimplements the same byte-mapping loop as `hls::pct_encode` with a single added `b'$'` branch — it should call `pct_encode` as a base or both should live in `media/mod.rs`; (2) `update_health` in `model/source.rs` and `model/playlist_item.rs` are identically structured with two SQL branches differing only in table name — a shared helper or macro would eliminate the duplication.

### #28 — Playlist admin correctness gaps

two related gaps: (1) `playlist_item_create` computes `sort_order = list_for_channel(...).len()` (all items including inactive), but VOD position is computed over active items only; inactive items create gaps in sort order that cause items to appear out-of-sequence once re-enabled. (2) `playlist_item_toggle` has no direct integration test — no 404 coverage for unknown IDs, no assertion on the redirect destination.

### #29 — UI quick-wins

done (2026-06-13): (2) EPG nav/program backgrounds neutralized to `var(--surface-nav)` / `var(--surface-2)` (blue tint removed, pink-red accent kept), (3) time labels `0.7→0.75rem` + program titles `0.78→0.82rem`, (4) `htmx-indicator` spinners on EPG nav (skeleton) + discovery search forms. Sub-item (1) debug-panel gating was deliberately dropped — the panel stays as-is by request. Spec: `docs/superpowers/specs/2026-06-13-ui-polish-cluster-design.md`.

### #30 — CSS design tokens

done (2026-06-13): new `static/app.css` (served at `/app.css` via `static_files::app_css`) holds the `:root` token ramp (`--bg --surface-1/-2/-nav --border/-strong/-subtle --text/-muted/-dim --accent --accent-dark --live --live-tint --ok`); the four muted greys collapse to `--text-muted`/`--text-dim`; both base templates link app.css and use `var(--…)`; the copy-pasted `.tabs`/`.tab` block now lives once in app.css; the header-nav inline style moved to a `.site-nav` class.

### #31 — Keyboard and accessibility polish

done (2026-06-13): `:focus-visible` rings on `.program`/`.tab`/`.nav-btn`; program blocks gained an Enter/Space `keydown` handler; EPG category tabs and time-nav `<a hx-get>` converted to `<button>` (with `role="tablist"`/`role="tab"`/dynamic `aria-selected`); `aria-label`s added to the debug panel Clear/Hide buttons.

### #32 — Player overlay controls

done (2026-06-13): a fading top toolbar (`#player-toolbar`) over the video with close / prev (↑) / next (↓) / help (?) buttons; the native `<video controls>` bottom bar is untouched. Toolbar fades in on mousemove/touch/focus and out after 3s idle; `?` toggles a `#player-help` panel listing the existing shortcuts (↑↓ channel, Space, ←→ VOD seek, F fullscreen). prev/next reuse `nextChannelId`+`tune`; close runs `stopPlayback`.

### #33 — HTMX loading skeletons

done (2026-06-13): a shimmer-animated `#epg-skeleton` (six placeholder rows matching the grid) shows during EPG partial fetches via the `htmx-indicator` pattern (nav buttons carry `hx-indicator="#epg-skeleton"`); a `#player-buffering` overlay (spinner + "Loading…") shows while a stream connects and clears on `playing`/`canplay`/`pause`/`error` (the `canplay`/`pause` listeners cover the autoplay-blocked case).

### #34 — Skip stream-proxy for resolved YouTube VOD URLs

done: `TuneResponse` carries a `skip_proxy: bool` (set from `resolver::needs_resolution` at each tune/next call site); the player's `_loadSource` uses the original unproxied `currentUrl` for `video.src` in the direct-MP4 branch when `skip_proxy` is true, so YouTube VOD streams go googlevideo CDN → browser directly with no Fly egress. The playlist-item **Test** button also marks such items' host as Direct (⚡) in the CORS cache so the budget badge reflects the bypass. Spec: `docs/superpowers/specs/2026-06-09-skip-proxy-youtube-vod-design.md`.

### #35 — YouTube Discover improvements

done: gaps (1) `source_kind` VOD detection and (2) `type=channel` search + channel-URL resolve were already fixed earlier; this change adds (3) thumbnails (`snippet.thumbnails.default`, lazy-loaded 80 px column for video and channel results) and (4) upcoming-stream handling (`liveBroadcastContent="upcoming"` → amber UPCOMING badge + scheduled start from `liveStreamingDetails.scheduledStartTime`, addable as a `youtube_live` source that activates when the stream goes live). Also extends the yt-dlp live-status probe to the full `live_status` state model (`LiveStatus`: Live / Upcoming(ts) / PostLive / WasLive / NotLive / Offline / Unknown) with distinct admin badges — the state foundation for ideas #38/#39. Spec: `docs/superpowers/specs/2026-06-10-youtube-discover-improvements-design.md`.

### #36 — Auto-convert ended YouTube live streams to VOD

done: at tune time `next_live` detects `force_finished/1` in the resolved manifest (`resolver::is_finished_live`) and, instead of black-screening, returns `TuneResponse { ended: true, url: "" }`. The frontend shows a brief "Stream ended — switching…" overlay and auto-advances to the next channel in the lineup (loop-guarded, cancellable on manual tune). A `tokio::spawn`ed background task converts the dead channel to a `vod_loop`: builds the canonical `watch?v=<id>` URL (`resolver::live_url_to_watch_url`, falling back to `resolver::fetch_video_id` for handle/channel `/live` forms), fetches the duration, creates a playlist_item, flips `channel.type`, and deactivates the sources (`convert_channel_to_vod_loop`, idempotent). No `source::update_url` (the watch URL lives on the playlist_item); no migration. Spec: `docs/superpowers/specs/2026-06-09-ended-live-to-vod-design.md`.

### #37 — Budget badge for YouTube Live streams

done: `health::probe_and_cache_resolved_cors` resolves a live YouTube/Twitch source via yt-dlp on admin **Test**, probes the resolved manifest's segment-CDN CORS (`hls::probe_source_cors`), and caches the result under **both** the resolved CDN host and the original source host — the latter is required because the guide and admin source-row budget lookups key off the DB source URL host (`youtube.com`), never the ephemeral resolved googlevideo URL. The `source_test` handler calls it behind a `needs_resolution` guard, mirroring `playlist_item_test`. Background sweep unchanged (`probe_and_cache_cors` still skips `needs_resolution` URLs). Spec: `docs/superpowers/specs/2026-06-09-live-budget-badge-design.md`.

### #38 — Auto-resume offline live channels

done: `resolve_url_with_status` now returns an empty URL + `Offline`/`Upcoming` status on resolve failure (`resolver::recoverable_status`) instead of erroring; `next_live` classifies each source (`classify_live_outcome` → Play/Ended/Waiting) and returns `TuneResponse { waiting: true, url: "" }` when no source is playable but ≥1 is offline/upcoming, else 503. A successful tune feeds `health::record_source_liveness(ok=true)` (resets failures / re-enables), but the tune path never disables — keeping the source active so the backoff poll can resume it mid-window. Persisting "offline" to source health (and auto-disable) is owned by the now liveness-aware 15-min background checker for `youtube_live` (`live_status_health` via `cached_live_status`), so the offline health badge reflects real liveness and re-enables only when the stream truly returns. The player shows a "Waiting for stream…" overlay and re-polls `/tune` on a 15→30→60→120s backoff (`waitingGen`-guarded, cancelled on manual tune), settling into "Channel offline". No migration. Spec: `docs/superpowers/specs/2026-06-11-auto-resume-offline-live-design.md`.

### #39 — Reopen idea36

done: `next_live` resolves via `resolve_url_with_status` (`--print live_status --print urls`, still one yt-dlp call) and converts on `was_live`/`post_live` in addition to the `force_finished/1` manifest fallback, so fully-processed recordings flip to `vod_loop` on first tune. `Upcoming`/`Offline` still fail resolution and fall through to failover/503 — that error branch is the seam for idea #38. Spec: `docs/superpowers/specs/2026-06-11-ended-live-conversion-via-live-status-design.md`.

### #41 — Stop the old stream when tuning to an unavailable channel

done: a single `stopPlayback()` teardown helper in `templates/base.html` (`video.pause()` → null `video.onerror` → `hls.stopLoad()`+`hls.detachMedia()` → `dash.reset()` → `removeAttribute('src')`+`video.load()`) is now called from `tune()` (replacing the partial `hls.stopLoad()`/`dash.reset()` block), `showPlayerError()`, and `enterWaitingState()`, so the previous stream is fully torn down — audio included — before the new tune resolves or the error/waiting overlay shows. `_loadSource`'s HLS branch re-`attachMedia`s and the native/direct branches reset `video.src`/`onerror`, so re-tuning is unaffected.

### #42 — Unify source status indicators (Active/Health/Live → Status)

done: new `src/status.rs` (`SourceStatus` + `compute`/`status_badge`/`most_optimistic`) collapses Active+Health+Live into one Status badge; Budget stays separate. `is_active` is now pure manual intent — the health checker records health but never disables/re-enables (`process_failures`); the tune path skips observed-Down regular sources via `source::list_tunable_for_channel` while `youtube_live` stays in rotation for #38. Admin source + playlist rows and the guide render the unified Status (guide = most-optimistic across sources). No migration. Spec: `docs/superpowers/specs/2026-06-11-unified-source-status-design.md`; arch: `docs/architecture/source-status.md`.

### #43 — Bilibili VOD playback

closed (2026-06-17, won't implement): tuning Bilibili videos as VOD sources. A 2026-06-14 spike (yt-dlp 2026.03.17 + Chrome cookies) resolved the open unknown — Bilibili is **DASH-only with no combined progressive format**: every format is `audio only`/`video only`, default best resolves to two signed `.m4s` URLs on `*.akamaized.net` with `upsig`/`deadline` tokens that expire in hours. Neither viable path is worth the cost: **(A) synthesized DASH** (build an `.mpd` over both `.m4s`, play via dash.js, proxy segments with a `Referer: bilibili.com` header) is the most code + MPD/byte-range risk + a new proxy header path; **(B) download+mux at add-time** (yt-dlp+ffmpeg stream-copy into one MP4, persist as ordinary VOD) is most reliable but needs ffmpeg + storage and brushes the project's "no transcoding" principle. The cost driver is the public Fly instance — it forces a `cookies.txt` secret (the headless VM has no browser for `--cookies-from-browser`) plus, for B, a Fly volume. Decision: shelved after a brainstorming session; revisit only if Bilibili playback becomes worth the cost. Details: `docs/superpowers/plans/2026-06-09-bilibili-support.md`, test plan `docs/superpowers/specs/2026-06-09-bilibili-support-test-plan-design.md`.

### #44 — Direct-play heuristic for self-hosted MP4 VOD

done: pure `resolver::is_direct_media_file(url)` (true for `.mp4/.webm/.m4v/.mov`, query/fragment stripped, case-insensitive) + `resolver::should_skip_proxy(url) = needs_resolution(url) || is_direct_media_file(url)`; both VOD call sites in `src/routes/player.rs` (`tune_vod_at`, `next_vod_at`) now pass `should_skip_proxy` instead of `needs_resolution`, so a `vod_loop` item with a plain media-file URL (e.g. a bucket MP4 from the [publish-video plugin](https://github.com/Wkkkkk/publish-video-plugin)) plays via `<video src>` without the double-egress `/stream-proxy` hop. Manifests (`.m3u8`/`.mpd`) still proxy. No migration, no admin UI. This was Tasks 1–2 of the MP4-VOD plan (Tasks 3–5 shipped earlier as the publish-video plugin). Spec: `docs/superpowers/specs/2026-06-14-self-hosted-mp4-vod-object-storage-design.md`; plan: `docs/superpowers/plans/2026-06-14-self-hosted-mp4-vod-object-storage.md`.

follow-up (duration auto-fill): adding a direct media file via the admin playlist form bounced with "Could not determine duration — enter it manually", because `media::fetch_duration` (`src/media/mod.rs`) only handles yt-dlp sources, `.mpd`, and otherwise assumes HLS — a plain `.mp4` is none of those, and yt-dlp returns `NA` for direct files (prod has no ffprobe anyway, per the no-transcoding principle). Fix: the add-item form (`templates/admin/channel_detail.html`) now reads duration from the browser's native `<video preload="metadata">` and prefills the field client-side on URL change (with a submit guard), falling back to the server path when the URL isn't a direct media file or metadata won't load. Zero new deps, nothing shipped, no server memory cost — the work happens in the viewer's browser. The programmatic `/api/admin` path is unaffected: the publish-video plugin already sends `duration_secs` (ffprobe at publish time).

follow-up (budget badge): a direct-media VOD item (e.g. an R2 `.mp4`) showed a misleading ☁ "proxied" badge. The budget badge is derived from a CORS probe of the item host (`budget::status_for_url`), and R2's `pub-*.r2.dev` domain sends no `Access-Control-Allow-Origin` header → the probe recorded "not direct" → ☁. But a direct media file plays via `<video src>` and skips `/stream-proxy` entirely, and a media element loads cross-origin **without** CORS, so the missing header is irrelevant — the real path is browser↔origin, i.e. ⚡. Fix: `budget::status_for_url` now returns `Direct` for `resolver::is_direct_media_file` URLs before the CORS-cache lookup (after the `http://` mixed-content guard, which still wins). Centralized in `status_for_url`, so the guide badge, admin source row, and admin playlist-item row all agree. `http://` media stays `Proxied`.

### #46 — Move player overlay toolbar to the bottom

done: CSS-only change in `templates/base.html` — the overlay toolbar (close ✕ / prev ↑ / next ↓ / help ?) moved from `top:0` to `bottom:44px`, stacked just above the native `<video controls>` bar instead of covering the top of the picture in windowed playback; the scrim gradient flipped to darken downward, and the `#player-help` popup moved from `top:52px` to `bottom:100px` so it opens upward. Markup and JS (the `show-controls` toggle, 3 s auto-hide, button handlers) are untouched, so both toolbars appear/hide together and never truly overlap. Fullscreen was out of scope (it targets the `<video>` element, so the overlay never renders there). Spec: `docs/superpowers/specs/2026-06-15-bottom-player-toolbar-design.md`; plan: `docs/superpowers/plans/2026-06-15-bottom-player-toolbar.md`.

### #45 — On-demand VOD channel type

done: a third channel type `vod_on_demand` (alongside `live` and `vod_loop`) for a viewer-controlled playlist — items play sequentially, the native `<video controls>` timeline handles seeking, the viewer clicks any item to jump/replay, and playback **stops silently** after the last item (no loop). Unlike `vod_loop`'s wall-clock broadcast simulation, an on-demand channel has no `loop_anchor`, so there's no "snap back to the clock" tension — the playback cursor (current item + offset) lives in the browser (`localStorage["mytv:ondemand:<id>"]`), so no server-side position state. Backend: `ChannelType::VodOnDemand` threaded through the model + guide/health matches; two public player endpoints — `GET /channel/:id/playlist` (`[{id,title,duration_secs}]` in `sort_order`) and `GET /channel/:id/item/:item_id` (resolves one item → `TuneResponse`, 404/422/503); `channels_json` now carries `type` so the client branches before tuning. Client (`templates/base.html`): an `od*` module fetches the playlist, resumes the saved cursor, renders a clickable list in the bottom overlay (CJK-safe rows — ellipsis title, `tabular-nums` duration), advances item-by-item on `ended`, and persists the cursor (debounced `timeupdate` + `pause`/`beforeunload`). Admin gets an "On-demand playlist" option in the channel-type dropdown. Migration `007_channel_vod_on_demand.sql` extends the `channels.type` CHECK constraint (the original spec assumed no migration — the CHECK constraint required one). Spec: `docs/superpowers/specs/2026-06-15-vod-on-demand-channel-design.md`; plan: `docs/superpowers/plans/2026-06-15-vod-on-demand-channel.md`.

### #52 — Per-item EPG blocks for VOD-on-demand channels

done: in the guide, a `vod_on_demand` channel now renders one clickable block per active playlist item (showing the item title) instead of a single `{name} — On demand` block. On-demand items have no schedule, so they can't be positioned proportionally on the timeline — they are laid out as fixed-width blocks (`ON_DEMAND_ITEM_WIDTH_PCT = 25.0`, ≈4 visible), left-to-right, clipped at the window edge; off-edge items remain reachable via the player's `☰` panel (which opens by default on tune). Clicking a block tunes the channel *and* jumps straight to that item, reusing the existing `GET /channel/:id/item/:item_id` path: `ProgramSlot` gained an `item_id: Option<i64>`, the template's click handlers append `, {{ id }}` when present (`tune(channel, item)`), and the player's `tune`/`odTune` JS accept an optional start-item id (backward-compatible — single-arg callers fall through to the saved cursor). Positioning lives in a new `layout::on_demand_slots` (pure presentation; an empty playlist yields one full-width fallback block so the row stays tunable), retiring `epg::on_demand_entry`. The title-width cap "same for vod_loop too" was already satisfied by the existing `.program-title` ellipsis CSS, so no CSS change was needed. Scope: `src/routes/guide/layout.rs`, `src/routes/guide/data.rs`, `src/epg.rs`, `templates/partials/epg_content.html`, `templates/base.html`. No migration. Spec: `docs/superpowers/specs/2026-06-16-vod-on-demand-epg-per-item-design.md`; plan: `docs/superpowers/plans/2026-06-16-vod-on-demand-epg-per-item.md`.

### #53 — VOD-on-demand dead-item handling

done (2026-06-16): dead on-demand playlist items (e.g. an R2 object deleted out from under the channel) are now handled end to end: `playlist_item::apply_health_result` is the single owner of the auto-disable rule (reusing the shared `source::FAILURE_THRESHOLD` via the pure `playlist_item::is_dead` predicate), called by both the background health loop and the interactive tune handler, so an item that fails `FAILURE_THRESHOLD` consecutive checks is disabled and drops out of `list_active_for_channel` (and thus the player playlist). The admin item list surfaces the disable reason on inactive rows ("auto-disabled — …"), and the player toasts and auto-skips a dead on-demand item to the next playable one instead of stalling. No migration (reuses existing columns); the admin "Test" button stays a non-disabling manual diagnostic.

### #54 — Reap stale disabled playlist items after 3 days

done (2026-06-16): a time-based garbage collector for `playlist_items` — any item that has stayed `is_active = 0` for more than 3 days is hard-deleted, regardless of *how* it was disabled (auto via #53 or a manual admin toggle). It only reaps already-disabled rows; it never disables anything, so it composes cleanly with #53 (failure → disable → 3 days → delete) without depending on it. Scope is `playlist_items` only — *sources* keep their own auto-disable + cooldown + re-enable lifecycle, untouched. Migration `008_playlist_item_disabled_at.sql` adds a nullable `disabled_at INTEGER`, backfilled to `strftime('%s','now')` for rows already disabled so the clock starts at deploy and the first pass never mass-deletes pre-existing disabled rows. Clock invariant — *disabled ⇒ `disabled_at` set; active ⇒ `disabled_at` NULL* — is held by `playlist_item::set_active` and the `is_active = Some(_)` branch of `update_health`, both stamping `disabled_at = COALESCE(disabled_at, now)` on disable (so a repeated disable never resets the clock — otherwise a continuously-failing item would never elapse) and clearing it to NULL on re-enable. The reaper itself is `playlist_item::reap_stale_disabled(pool, older_than_secs)` (one atomic `DELETE … RETURNING id, title`), wired into the existing `health::check_all` 15-min tick after the checks via `health::reap_stale_disabled_items`; the 3-day threshold is a `const STALE_DISABLED_TTL_SECS` (not env-configurable, matching `FAILURE_THRESHOLD` style), and a reaper DB error logs and continues rather than crashing the loop. Each pass logs the count + reaped `(id, title)` pairs at `info`. Out of scope: deleting the underlying R2 object (the row is just a pointer; R2 lifecycle is owned by the publish-video plugin). Standalone idea #54, brainstormed + approved 2026-06-16.

### #51 — Redesign the player overlay (VOD-on-demand fit + mobile/touch)

done (2026-06-16): the keyboard-only player overlay was replaced with a gesture-forward, channel-type-aware design (Direction C) where touch and keyboard are co-equal first-class inputs. The native `<video controls>` bar was dropped in favour of a custom transport (play/pause, scrubber, time, ⏮/⏭, ☰ playlist, ⛶), with fullscreen delegated to the native API on `#player-panel` so the custom chrome stays visible. Idle = video fills the panel with no chrome; a single tap / `mousemove` reveals it (3s auto-hide). The core behaviour is **unified vertical navigation**: `↑`/`↓` and swipe up/down treat the whole channel list as one continuous feed — on-demand channels expand into their items, live/`vod_loop` are single entries, and stepping past a playlist edge overflows into the adjacent channel (landing on its last item when crossing up into another on-demand channel); the feed wraps at the ends, preserving `nextChannelId`'s existing behaviour. Touch gestures: single tap toggles chrome, double-tap near a left/right edge seeks ∓10s (seekable types only), vertical swipe navigates; multi-touch/pinch is ignored. Seek (`←`/`→`, double-tap, scrubber) is now enabled for `vod_on_demand` too (previously `vod_loop`-only). Keyboard map finalized: `Space` play/pause, `↑`/`↓` navigate, `←`/`→` seek, `F` fullscreen, `Esc` close (deferring to native fullscreen-exit first), `?` help cheat-sheet. Discoverability is a `?` help panel only (no coachmarks). Frontend-only — no backend/API/DB/migration changes; the tune/next/playlist endpoints were untouched. Implemented across `templates/guide.html` (markup) and `templates/base.html` (CSS + JS) in 6 reviewed tasks. Spec: `docs/superpowers/specs/2026-06-16-player-overlay-redesign-design.md`; plan: `docs/superpowers/plans/2026-06-16-player-overlay-redesign.md`.

### #55 — Publish weekly progress cards as a GitHub Pages site

done (2026-06-17): the weekly editorial progress cards (`docs/progress/<date>-week-card.html`, generated by the local `mytv-weekly-card` launchd job) are published as a static, self-regenerating GitHub Pages microsite at `https://wkkkkk.github.io/MyTV/`. A Python-stdlib build (`scripts/progress_site/build.py`) discovers the cards, extracts each week's headline / date-range / commit-count from the card HTML's stable classes (`.title`, `.kicker`, `.statblock .big`, with a markdown-heading fallback), and renders a designed editorial index plus one reflow detail page per week from `template.html` / `detail-template.html`. The landing page (decided via brainstorming + browser mockups): branded masthead, an About block, a **poster** hero (latest week, scaled 4:3) with the previous two weeks below, a full **archive timeline** of every week, and two standing reference sections — Architecture (links the interactive `architecture-diagram.html` + the `docs/architecture/*` deep-dives) and Incidents (the `docs/bug-logs/*` writeups). Recent/archive entries open a **reflow** detail page (card embedded in a <1000px column so its own responsive layout makes it readable). A GitHub Action (`.github/workflows/pages.yml`) tests the build, runs it, and deploys `site/` via `actions/deploy-pages` on push under `docs/progress/**`, the diagram, or the builder — so the site regenerates whenever a new card lands; the card-generation job is untouched. Built fully static (no backend, no markdown rendering — standing sections link to GitHub-rendered docs). 17 unit tests cover discover/render/assemble. Spec: `docs/superpowers/specs/2026-06-17-progress-pages-site-design.md`; plan: `docs/superpowers/plans/2026-06-17-progress-pages-site.md`; builder docs: `scripts/progress_site/README.md`.

## Architecture deepening

### #47 — Typed live resolution (kill the empty-URL sentinel)

done: `resolver::resolve_url_with_status -> Result<(String, LiveStatus)>` (where an *empty* string secretly meant "not playable, status Offline/Upcoming") is replaced by `resolver::resolve_live -> Result<LiveResolution>`, a closed enum `LiveResolution::{ Playable { url }, Ended, Waiting }` so an unplayable state is unrepresentable as a URL. The ended/waiting *decision* moved from the player into the resolver: a new pure, truth-table-tested `classify_resolved(status, url)` maps `WasLive`/`PostLive` or a `force_finished/1` manifest → `Ended`, else `Playable`; a recoverable resolve failure (`recoverable_status` → Offline/Upcoming) → `Waiting`. The HLS/IPTV passthrough still runs through `classify_resolved(Unknown, url)` so a manifest already carrying `force_finished` is `Ended`, not `Playable`. `next_live` becomes a flat `match` on the three variants, and the two shallow player helpers (`is_ended_live`, `classify_live_outcome`) plus the `LiveOutcome` enum and their tests are deleted (decision no longer smeared across two files). `resolve_url` (the VOD/discovery wrapper) becomes a `match` that maps `Ended`/`Waiting` to `bail!`. Scope: `src/media/resolver.rs` + `src/routes/player.rs`; no migration, no behavior change (verified by the unchanged `test_tune_finished_live_returns_ended_and_no_url` integration test). Idea #47 from the 2026-06-16 architecture-deepening round.

### #48 — Source-availability classification + threshold unification

done: the failure threshold `3` — previously a `#[cfg(test)]`-only `FAILURE_THRESHOLD` in `health.rs`, a hand-typed `consecutive_failures >= 3` literal in the tune SQL, and a `FAILURE_DOWN_THRESHOLD` in source tests — is unified into one `pub const FAILURE_THRESHOLD: i64 = 3` in `src/model/source.rs`. A new pure, truth-table-tested `is_observed_down(kind, last_status, consecutive_failures) -> bool` captures the tune-time "Down" rule (non-`youtube_live` + `last_status='error'` + failures ≥ threshold), and `list_tunable_for_channel` now reuses `list_active_for_channel` and filters in Rust through that predicate — deleting the bespoke SQL `AND NOT (...)` clause so there is exactly one source of truth and the threshold cannot skew between code paths. **Premise correction:** idea #48 originally read `docs/architecture/health-checker.md` as the authority and proposed re-adding a `HealthAction { Disable, Reenable, None }` enum to flip `is_active`; investigation showed the code had deliberately moved to "manual intent is the source of truth" (every `update_health` call from `health.rs` passes `is_active = None`; "Down" is computed at tune time, not stored). So the *doc* was the stale artifact, not the code — it was rewritten to describe the real model (the once-shipped auto-disable/`HealthAction::Reenable` behavior recorded under Foundational work was removed in a later refactor). No migration, behavior preserved (the unchanged `test_list_tunable_skips_down_regular_keeps_youtube_and_disabled_excluded` integration test is the guard). Scope: `src/model/source.rs`, `src/health.rs`, `docs/architecture/health-checker.md`. Spec: `docs/superpowers/specs/2026-06-16-health-action-classification-design.md`. Idea #48 from the 2026-06-16 architecture-deepening round.

### #49 — One coercion source of truth for intake DTOs

done: the numeric-coercion logic for intake fields — the `0`/`1` default literals and the string→`i64` parsing — was duplicated across the HTML-form admin handlers and the JSON-API handlers, leaving the two front doors free to drift. It is collapsed into a single model-layer seam: `coerce_i64` plus the `DEFAULT_PRIORITY`/`DEFAULT_SORT_ORDER` constants in `src/model/mod.rs`. The form handlers now call `coerce_i64` (strict: blank → default, garbage → 422); the JSON handlers keep their typed serde fields but reference the same constants, so the threshold values can no longer skew between code paths. Side effect: a form `source.priority="abc"` now returns 422 instead of silently becoming `1`, making the form path internally consistent with `sort_order` and the JSON serde path. The JSON contract is unchanged, and auto-fetch-duration stays in the playlist handler. Idea #49 from the 2026-06-16 architecture-deepening round.

### #50 — Collapse the model-layer CRUD triplication

done (2026-06-17): scoped to the low-risk, high-value step; the broad generic-CRUD collapse is intentionally **not** pursued. Shipped: the string-parameterized `model::update_health_sql` — which `format!`'d a table name into SQL (the *only* dynamic-table-name query in the codebase, shared across `sources` and `playlist_items`) — is retired; `source::update_health` and `playlist_item::update_health` now each inline a literal-table-name query, killing the silent field-skew hazard. Investigation of the broader collapse changed the decision: the original framing assumed SQLx compile-time-checked `query_as!`, but the codebase uses runtime `sqlx::query(...)`/`query_as::<_, T>(...)` with literal SQL everywhere (no `.sqlx` offline cache, no build-time `DATABASE_URL`), so a generic `CrudModel<T>` trait would give up no checking that exists today — but it *would* violate the real constraint (each query stays a self-contained literal string, the convention this step restored) by building `format!` strings. A per-table *macro* expanding literal queries remains the most faithful path **if** the ~2,000-line `source.rs`/`playlist_item.rs`/`channel.rs` boilerplate collapse is ever pursued; left as a deliberate non-goal pending a spike. Idea #50 from the 2026-06-16 architecture-deepening round.

# Architecture Docs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create five Markdown files in `docs/architecture/` with Mermaid diagrams and prose documenting the request/route map, health checker, tune flow, yt-dlp resolution chain, and database schema.

**Architecture:** Each file is a self-contained Markdown document with a Mermaid code block followed by prose explaining non-obvious behavior. No build tooling required — GitHub renders Mermaid natively.

**Tech Stack:** Markdown, Mermaid diagram syntax (flowchart, stateDiagram-v2, erDiagram)

---

## File Structure

- Create: `docs/architecture/request-route-map.md`
- Create: `docs/architecture/health-checker.md`
- Create: `docs/architecture/tune-flow.md`
- Create: `docs/architecture/ytdlp-resolution.md`
- Create: `docs/architecture/database-er.md`

No existing files are modified.

---

### Task 1: Request & Route Map

**Files:**
- Create: `docs/architecture/request-route-map.md`

- [ ] **Step 1: Verify route list against source**

Run: `grep -n 'route\|nest\|layer' src/lib.rs`

Expected: see all `.route(...)`, `.nest("/admin", ...)`, `.layer(...)`, `.route_layer(...)` calls matching the list below.

- [ ] **Step 2: Create the file**

Create `docs/architecture/request-route-map.md` with this exact content:

```markdown
# Request & Route Map

Every HTTP request passes through one middleware layer before reaching a handler.

~~~mermaid
flowchart LR
    req([HTTP Request]) --> rts["redirect_trailing_slash\n(outermost .layer)"]
    rts -->|"path ends with /"| red(["308 Permanent Redirect"])
    rts --> router{Router}

    router --> pub[Public Routes]
    router --> adm["/admin/**\nbasic_auth (.route_layer)"]

    pub --> r1["GET /  →  redirect /guide"]
    pub --> r2["GET /health"]
    pub --> r3["GET /guide"]
    pub --> r4["GET /guide/partial  (HTMX)"]
    pub --> r5["GET /channel/:id/tune"]
    pub --> r6["GET /channel/:id/next"]
    pub --> r7["GET /stream-proxy"]

    adm --> a1["GET+POST /admin/channels\nGET /admin/channels/new\nGET+POST /admin/channels/:id\nGET /admin/channels/:id/edit\nPOST /admin/channels/:id/delete"]
    adm --> a2["POST /admin/channels/:id/sources\nPOST /admin/sources/:id/delete\nPOST /admin/sources/:id/toggle\nPOST /admin/sources/:id/test"]
    adm --> a3["POST /admin/channels/:id/playlist\nPOST /admin/playlist/:id/delete"]
    adm --> a4["GET /admin/discover\nPOST /admin/discover/add-form\nPOST /admin/discover/add\nPOST /admin/discover/m3u/search\nPOST /admin/discover/youtube/search\nPOST /admin/discover/manual/resolve"]
~~~

## Notes

**Middleware order matters.** `redirect_trailing_slash` is registered with `.layer()` (outermost), so it fires before route matching *and* before auth. A request to `GET /admin/` gets a 308 redirect without ever hitting the `basic_auth` middleware. Use `/admin` (no trailing slash) to test authentication.

**Auth scope.** `basic_auth` is registered with `.route_layer()` scoped to the admin sub-router only. Public routes (`/guide`, `/channel/:id/tune`, etc.) require no credentials.

**Player routes return JSON.** `/channel/:id/tune` and `/channel/:id/next` return `Json<TuneResponse>` with HTTP 200 on success or HTTP 503 on failure — they do not redirect or return HTML.
```

- [ ] **Step 3: Commit**

```bash
git add docs/architecture/request-route-map.md
git commit -m "docs: add request and route map diagram"
```

---

### Task 2: Health Checker & Source State Machine

**Files:**
- Create: `docs/architecture/health-checker.md`

- [ ] **Step 1: Verify key constants against source**

Run: `grep -n 'FAILURE_THRESHOLD\|CHECK_INTERVAL\|HTTP_TIMEOUT\|MissedTickBehavior\|set_inactive\|is_active' src/health.rs`

Expected: `FAILURE_THRESHOLD = 3`, `CHECK_INTERVAL = 15 * 60`, `HTTP_TIMEOUT = 5`, `MissedTickBehavior::Skip`, `set_inactive` flag, `is_active` guard.

- [ ] **Step 2: Create the file**

Create `docs/architecture/health-checker.md` with this exact content:

```markdown
# Health Checker & Source State Machine

A background Tokio task checks every source URL every 15 minutes and auto-disables sources that fail repeatedly.

## Tick Loop

~~~mermaid
flowchart TD
    start(["health::start(pool, client)"]) --> spawn[Spawn detached Tokio task]
    spawn --> t1[Consume first tick\nno check at startup]
    t1 --> wait["Wait 15 min\nMissedTickBehavior::Skip"]
    wait --> fetch["list_all sources"]
    fetch --> loop{For each source}
    loop --> check["HTTP GET  (5s timeout)"]
    check -->|youtube_live| yt["HTTP 200/3xx sufficient\n(yt-dlp too slow for background)"]
    check -->|hls / iptv| chunk["Read one chunk\nverify bytes delivered"]
    yt --> result[process_result]
    chunk --> result
    result -->|ok| reset["failures = 0"]
    result -->|"fail, failures < 3"| inc["failures++"]
    result -->|"fail, failures ≥ 3\nand is_active = true"| disable["set is_active = 0"]
    reset --> db[update_health in DB]
    inc --> db
    disable --> db
    db --> loop
    loop -->|all done| wait
~~~

## Source State Machine

~~~mermaid
stateDiagram-v2
    [*] --> Active : source created
    Active --> Active : check ok — failures reset to 0
    Active --> Active : check fails — failures < 3
    Active --> Disabled : check fails — failures reach 3
    Disabled --> Disabled : check runs — no state change
    Disabled --> Active : admin manually toggles on
~~~

## Notes

**No auto-re-enable.** Once a source is disabled the health checker does not bring it back. The `process_result` guard (`src.is_active`) prevents the disable flag from firing again on an already-inactive source, but a successful check only resets `consecutive_failures` — it does not set `is_active = 1`. Re-enabling requires manual action via the admin toggle button.

**Why `MissedTickBehavior::Skip`?** If a full check round (many sources all timing out at 5s each) takes longer than 15 minutes, any missed ticks are dropped rather than queued. This prevents a backlog of back-to-back check rounds after a slow cycle.

**First tick consumed.** The task calls `interval.tick().await` once immediately after spawning before entering the loop, which discards the initial zero-delay tick. Sources are not checked at startup.

**youtube_live shortcut.** Checking a YouTube live stream via yt-dlp takes several seconds and is too slow for a background health check. Instead, only the HTTP response status is checked (200 or 3xx is sufficient). The source is considered healthy if the page loads.
```

- [ ] **Step 3: Commit**

```bash
git add docs/architecture/health-checker.md
git commit -m "docs: add health checker and source state machine diagram"
```

---

### Task 3: Tune Flow

**Files:**
- Create: `docs/architecture/tune-flow.md`

- [ ] **Step 1: Verify key logic against source**

Run: `grep -n 'tune_live\|tune_vod_at\|next_live\|next_vod\|failed_url\|loop_anchor\|rem_euclid\|start_offset' src/routes/player.rs src/model/playlist_item.rs`

Expected: `tune_live`, `tune_vod_at`, `next_live`, `next_vod_at` functions; `failed_url` filter; `loop_anchor.timestamp()`; `rem_euclid(total)` in `current_position`; `start_offset_secs` in `TuneResponse`.

- [ ] **Step 2: Create the file**

Create `docs/architecture/tune-flow.md` with this exact content:

```markdown
# Tune Flow

How `/channel/:id/tune` and `/channel/:id/next` select a stream URL and return it to the player.

## `/tune` — Initial Tune

~~~mermaid
flowchart TD
    req(["GET /channel/:id/tune"]) --> get["channel::get(id)"]
    get -->|not found| n404(["404 Not Found"])
    get --> branch{"channel_type"}

    branch -->|live| live["tune_live"]
    branch -->|vod_loop| vod["tune_vod_at"]

    live --> src["list_active_for_channel\nordered by priority ASC"]
    src --> iter{"for each source"}
    iter --> res1["resolver::resolve_url(src.url)"]
    res1 -->|ok| ok1(["200 { url, start_offset_secs: 0 }"])
    res1 -->|err| iter
    iter -->|all fail or none| s503a(["503 Service Unavailable"])

    vod --> anc{"loop_anchor set?"}
    anc -->|no| s500(["500 Internal Server Error"])
    anc -->|yes| items["list_for_channel\nordered by sort_order ASC"]
    items -->|empty| s503b(["503 Service Unavailable"])
    items --> pos["current_position(items, now_secs, anchor_secs)"]
    pos --> res2["resolver::resolve_url(item.url)"]
    res2 -->|ok| ok2(["200 { url, start_offset_secs: offset }"])
    res2 -->|err| s503c(["503 Service Unavailable"])
~~~

## VOD Position Calculation

`current_position` determines which playlist item is currently airing and how far into it.

```
total      = sum of all item duration_secs
elapsed    = (now_secs − anchor_secs) rem_euclid total
walk items accumulating durations until elapsed < accumulated
offset     = elapsed − (accumulated − item.duration_secs)
```

`rem_euclid` is used instead of `%` so the result is always non-negative even if `anchor_secs` is in the future relative to `now_secs`.

## `/next` — Fallback to Next Source

~~~mermaid
flowchart TD
    req(["GET /channel/:id/next?failed_url=..."]) --> get["channel::get(id)"]
    get --> branch{"channel_type"}

    branch -->|live| nlive["next_live(failed_url)"]
    branch -->|vod_loop| nvod["next_vod_at"]

    nlive --> src["list_active_for_channel\nfilter out failed_url"]
    src --> iter{"for each remaining source"}
    iter --> res1["resolver::resolve_url"]
    res1 -->|ok| ok1(["200 { url, start_offset_secs: 0 }"])
    res1 -->|err| iter
    iter -->|none left| s503(["503 Service Unavailable"])

    nvod --> items["list_for_channel"]
    items --> idx["next_idx = (current_idx + 1) % len"]
    idx --> res2["resolver::resolve_url(items[next_idx].url)"]
    res2 -->|ok| ok2(["200 { url, start_offset_secs: 0 }"])
    res2 -->|err| s503b(["503 Service Unavailable"])
~~~

## Notes

**`failed_url` is the raw source URL**, not the resolved playable URL. The player passes the original URL it was given so the server can match it against `src.url` directly.

**Fallback is one level deep.** `/next` skips exactly one named URL. If all other active sources also fail to resolve, 503 is returned immediately — there is no retry loop beyond the source list.

**VOD `start_offset_secs`.** The player uses this value to seek mid-episode, making the channel behave like a broadcast schedule where every viewer sees the same content at the same time.

**VOD `/next` ignores `failed_url`.** The VOD next handler advances to the next playlist item by index and ignores the `failed_url` parameter entirely — VOD items don't have fallback sources.
```

- [ ] **Step 3: Commit**

```bash
git add docs/architecture/tune-flow.md
git commit -m "docs: add tune flow diagram with VOD position subsection"
```

---

### Task 4: yt-dlp Resolution Chain

**Files:**
- Create: `docs/architecture/ytdlp-resolution.md`

- [ ] **Step 1: Verify logic against source**

Run: `grep -n 'needs_resolution\|resolve_url\|fetch_duration\|yt-dlp\|timeout\|first_line' src/media/resolver.rs`

Expected: `needs_resolution` checks `youtube.com`, `youtu.be`, `twitch.tv`; `resolve_url` with 30s timeout and `yt-dlp -g --no-playlist`; `fetch_duration_secs` with `--print duration`; `first_line` from stdout.

- [ ] **Step 2: Create the file**

Create `docs/architecture/ytdlp-resolution.md` with this exact content:

```markdown
# yt-dlp Resolution Chain

How a raw source URL becomes a directly playable stream URL.

## `resolve_url` — Called at Tune Time

~~~mermaid
flowchart TD
    url(["Input URL"]) --> scheme{"starts with\nhttp:// or https://?"}
    scheme -->|no| err1(["Error: invalid scheme"])
    scheme -->|yes| need{"needs_resolution?\nyoutube.com / youtu.be / twitch.tv"}
    need -->|no| pass(["Return URL unchanged"])
    need -->|yes| spawn["spawn yt-dlp -g --no-playlist -- url\n(30s timeout)"]
    spawn -->|timeout| err2(["Error: timed out after 30s"])
    spawn -->|non-zero exit| err3(["Error: stderr message"])
    spawn -->|empty stdout| err4(["Error: empty output"])
    spawn -->|ok| line["Take first line of stdout"]
    line --> resolved(["Return playable HLS URL"])
~~~

## `fetch_duration_secs` — Called at Admin Time

~~~mermaid
flowchart TD
    url(["VOD item URL"]) --> scheme{"starts with\nhttp:// or https://?"}
    scheme -->|no| err1(["Error: invalid scheme"])
    scheme -->|yes| spawn["spawn yt-dlp --print duration --no-playlist -- url\n(30s timeout)"]
    spawn -->|timeout| err2(["Error: timed out"])
    spawn -->|non-zero exit| err3(["Error: stderr message"])
    spawn -->|ok| parse["parse stdout as f64\nvalidate finite and positive"]
    parse -->|invalid| err4(["Error: could not parse duration"])
    parse -->|ok| secs(["Return duration as i64 seconds"])
~~~

`fetch_duration_secs` is called once when an admin adds a VOD playlist item. The result is stored in `playlist_items.duration_secs` so the tune-time position calculation does not need to call yt-dlp again.

## Notes

**yt-dlp is optional.** If the binary is not installed, `resolve_url` returns an error for YouTube/Twitch URLs. The caller (tune flow) treats this as a source failure and moves on to the next source, or returns 503 if no sources remain.

**`needs_resolution` is pattern-based.** The check is a substring match on the URL — no HTTP probe is made. Vimeo and other platforms are not in the list; their URLs pass through unchanged and will typically fail to play in HLS-only players.

**Sequential resolution.** For a live channel with multiple YouTube sources, resolution attempts run one at a time. Each can take up to 30 seconds before timing out. A channel with three YouTube sources could wait up to 90 seconds before returning 503.

**First line only.** yt-dlp can return multiple URLs (e.g. different quality levels). Only the first line of stdout is used.
```

- [ ] **Step 3: Commit**

```bash
git add docs/architecture/ytdlp-resolution.md
git commit -m "docs: add yt-dlp resolution chain diagram"
```

---

### Task 5: Database ER Diagram

**Files:**
- Create: `docs/architecture/database-er.md`

- [ ] **Step 1: Verify schema against migrations**

Run: `cat migrations/001_initial.sql migrations/002_source_health.sql`

Expected: `channels` table with `id, name, category, logo_url, type, sort_order, loop_anchor`; `sources` with FK to channels + health columns from migration 002; `playlist_items` with FK to channels.

- [ ] **Step 2: Create the file**

Create `docs/architecture/database-er.md` with this exact content:

```markdown
# Database Schema

Three tables. Channels own sources and playlist items; both child tables cascade on delete.

~~~mermaid
erDiagram
    channels {
        integer id PK
        text name
        text category
        text logo_url "nullable"
        text type "live | vod_loop"
        integer sort_order
        datetime loop_anchor "nullable, required for vod_loop"
    }

    sources {
        integer id PK
        integer channel_id FK
        text kind "youtube_live | hls | iptv"
        text url
        integer priority
        integer is_active
        integer last_checked_at "nullable, unix timestamp"
        text last_status "nullable: ok | error"
        integer consecutive_failures
        text failure_reason "nullable"
    }

    playlist_items {
        integer id PK
        integer channel_id FK
        text title
        text url
        integer duration_secs
        integer sort_order
    }

    channels ||--o{ sources : "has"
    channels ||--o{ playlist_items : "has"
~~~

## Notes

**`loop_anchor`** is a fixed UTC timestamp set when a `vod_loop` channel is created. It serves as the epoch for the VOD position calculation — the playlist cycles continuously from this point forward. It is never updated.

**`ON DELETE CASCADE`** is set on both `sources.channel_id` and `playlist_items.channel_id`. Deleting a channel removes all its sources and playlist items in one operation.

**Health columns** (`last_checked_at`, `last_status`, `consecutive_failures`, `failure_reason`) were added in `migrations/002_source_health.sql`. They are written only by the background health checker — never by CRUD routes.

**`sources.priority`** determines the order in which sources are tried during tuning (`ORDER BY priority ASC`). Lower number = tried first.
```

- [ ] **Step 3: Commit**

```bash
git add docs/architecture/database-er.md
git commit -m "docs: add database ER diagram"
```

# Health Checker & Source State Machine

A background Tokio task checks every source URL every 15 minutes and auto-disables sources that fail repeatedly. Sources that recover are automatically re-enabled on the next successful check.

## Tick Loop

```mermaid
flowchart TD
    start(["health::start(HealthClients)"]) --> spawn[Spawn detached Tokio task]
    spawn --> t1[Consume first tick\nno check at startup]
    t1 --> wait["Wait 15 min\nMissedTickBehavior::Skip"]
    wait --> fetch["list_all sources"]
    fetch --> loop{For each source}
    loop --> check["HTTP GET  (5s timeout)"]
    check -->|youtube_live| yt["HTTP 200/3xx sufficient\n(yt-dlp too slow for background)"]
    check -->|hls / iptv| chunk["Read one chunk\nverify bytes delivered"]
    yt --> result[process_result]
    chunk --> result
    result -->|"ok, is_active = true"| reset["failures = 0"]
    result -->|"ok, is_active = false"| reenable["failures = 0\nset is_active = 1"]
    result -->|"fail, failures < 3"| inc["failures++"]
    result -->|"fail, failures ≥ 3\nand is_active = true"| disable["set is_active = 0"]
    reset --> db[update_health in DB]
    reenable --> db
    inc --> db
    disable --> db
    db -->|ok| cors["probe_and_cache_cors\n(skip if not ok)"]
    cors --> loop
    loop -->|all done| playlist["probe_all_playlist_cors\ndedupe by CDN host"]
    playlist --> wait
```

## Source State Machine

```mermaid
stateDiagram-v2
    [*] --> Active : source created
    Active --> Active : check ok — failures reset to 0
    Active --> Active : check fails — failures < 3
    Active --> Disabled : check fails — failures reach 3
    Disabled --> Active : check ok — auto re-enabled
    Disabled --> Active : admin manually toggles on
```

## Notes

**`health::start` takes `HealthClients`.** The struct bundles `pool`, `http_client`, and `cors_cache` so the checker can update both source health and the CORS badge cache in one pass.

**Auto-re-enable.** When a disabled source passes a health check, `process_result` returns `HealthAction::Reenable` and `update_health` sets `is_active = 1`. The source returns to active rotation immediately on the next check cycle — no cooldown period. The `HealthAction` enum (private to `health.rs`) makes the three outcomes — `Disable`, `Reenable`, `None` — mutually exclusive.

**CORS probing.** After a source HTTP check succeeds, `probe_and_cache_cors` sends a CORS preflight to the CDN and caches the result (keyed by host) in the shared `CorsCache`. After all sources are checked, `probe_all_playlist_cors` does the same for VOD playlist item URLs, deduping by CDN host so each CDN is probed at most once per cycle. Non-HTTPS and resolution-needed (YouTube/Twitch) URLs are skipped.

**`probe_source` vs `check_source`.** The admin Test button calls `probe_source`, which runs the same HTTP check and updates `last_status`/`consecutive_failures` but passes `is_active = None` to `update_health` — it never changes whether a source is enabled. `check_source` (used by the background loop) is the only path that can auto-disable or auto-re-enable sources.

**Why `MissedTickBehavior::Skip`?** If a full check round (many sources all timing out at 5s each) takes longer than 15 minutes, any missed ticks are dropped rather than queued. This prevents a backlog of back-to-back check rounds after a slow cycle.

**First tick consumed.** The task calls `interval.tick().await` once immediately after spawning before entering the loop, which discards the initial zero-delay tick. Sources are not checked at startup.

**youtube_live shortcut.** Checking a YouTube live stream via yt-dlp takes several seconds and is too slow for a background health check. Instead, only the HTTP response status is checked (200 or 3xx is sufficient). The source is considered healthy if the page loads.

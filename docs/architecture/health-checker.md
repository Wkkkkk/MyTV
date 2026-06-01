# Health Checker & Source State Machine

A background Tokio task checks every source URL every 15 minutes and auto-disables sources that fail repeatedly.

## Tick Loop

```mermaid
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
```

## Source State Machine

```mermaid
stateDiagram-v2
    [*] --> Active : source created
    Active --> Active : check ok — failures reset to 0
    Active --> Active : check fails — failures < 3
    Active --> Disabled : check fails — failures reach 3
    Disabled --> Disabled : check runs — no state change
    Disabled --> Active : admin manually toggles on
```

## Notes

**No auto-re-enable.** Once a source is disabled the health checker does not bring it back. The `process_result` guard (`src.is_active`) prevents the disable flag from firing again on an already-inactive source, but a successful check only resets `consecutive_failures` — it does not set `is_active = 1`. Re-enabling requires manual action via the admin toggle button.

**Why `MissedTickBehavior::Skip`?** If a full check round (many sources all timing out at 5s each) takes longer than 15 minutes, any missed ticks are dropped rather than queued. This prevents a backlog of back-to-back check rounds after a slow cycle.

**First tick consumed.** The task calls `interval.tick().await` once immediately after spawning before entering the loop, which discards the initial zero-delay tick. Sources are not checked at startup.

**youtube_live shortcut.** Checking a YouTube live stream via yt-dlp takes several seconds and is too slow for a background health check. Instead, only the HTTP response status is checked (200 or 3xx is sufficient). The source is considered healthy if the page loads.

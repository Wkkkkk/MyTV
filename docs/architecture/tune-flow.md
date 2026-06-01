# Tune Flow

How `/channel/:id/tune` and `/channel/:id/next` select a stream URL and return it to the player.

## `/tune` — Initial Tune

```mermaid
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
```

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

```mermaid
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
```

## Notes

**`failed_url` is the raw source URL**, not the resolved playable URL. The player passes the original URL it was given so the server can match it against `src.url` directly.

**Fallback is one level deep.** `/next` skips exactly one named URL. If all other active sources also fail to resolve, 503 is returned immediately — there is no retry loop beyond the source list.

**VOD `start_offset_secs`.** The player uses this value to seek mid-episode, making the channel behave like a broadcast schedule where every viewer sees the same content at the same time.

**VOD `/next` ignores `failed_url`.** The VOD next handler advances to the next playlist item by index and ignores the `failed_url` parameter entirely — VOD items don't have fallback sources.

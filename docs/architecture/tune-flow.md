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
    iter --> res1["resolver::resolve_url_with_status(src.url)"]
    res1 -->|ok| fin{"is_ended_live?"}
    fin -->|no| ok1(["200 { …, skip_proxy, ended: false }"])
    fin -->|yes| conv["spawn live→VOD conversion"]
    conv --> okE(["200 { url: '', …, ended: true }"])
    res1 -->|err| iter
    iter -->|all fail or none| s503a(["503 Service Unavailable"])

    vod --> anc{"loop_anchor set?"}
    anc -->|no| s500(["500 Internal Server Error"])
    anc -->|yes| items["list_for_channel\nordered by sort_order ASC"]
    items -->|empty| s503b(["503 Service Unavailable"])
    items --> pos["current_position(items, now_secs, anchor_secs)"]
    pos --> res2["resolver::resolve_url(item.url)"]
    res2 -->|ok| ok2(["200 { …, skip_proxy, ended: false }"])
    res2 -->|err| s503c(["503 Service Unavailable"])
```

Every success response carries two extra booleans beyond the metadata fields: `skip_proxy` (the player points `<video>` straight at the resolved CDN URL when `true` — see `ytdlp-resolution.md`) and `ended` (signals a broadcast that has finished — see below). The full payload is `{ url, start_offset_secs, name, logo_url, category, channel_type, skip_proxy, ended }`.

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
    iter --> res1["resolver::resolve_url_with_status"]
    res1 -->|ok| fin{"is_ended_live?"}
    fin -->|no| ok1(["200 { …, ended: false }"])
    fin -->|yes| conv["spawn live→VOD conversion"]
    conv --> okE(["200 { url: '', …, ended: true }"])
    res1 -->|err| iter
    iter -->|none left| s503(["503 Service Unavailable"])

    nvod --> items["list_for_channel"]
    items --> idx["next_idx = (current_idx + 1) % len"]
    idx --> res2["resolver::resolve_url(items[next_idx].url)"]
    res2 -->|ok| ok2(["200 { …, ended: false }"])
    res2 -->|err| s503b(["503 Service Unavailable"])
```

## Ended Live → VOD Conversion

Resolution returns the URL plus yt-dlp's `live_status`; the handler treats the broadcast as ended when the status is `was_live` (recording processed) or `post_live` (just ended), or — as a fallback for extractors without `live_status` — when `resolver::is_finished_live` detects a `force_finished/1` manifest. In any of these cases, the handler does **not** return the dead manifest. Instead it:

1. Fires a detached `tokio::spawn` task (`spawn_live_to_vod_conversion`) and returns `TuneResponse { ended: true, url: "" }` immediately.
2. The frontend shows a brief "Stream ended — switching…" overlay and auto-advances to the next channel in the lineup (loop-guarded, cancelled on a manual tune).

The background task (`live_to_vod_conversion` → `convert_channel_to_vod_loop`):

```
watch_url = live_url_to_watch_url(source_url)            # youtube.com/live/<id> → watch?v=<id>
          ?? "watch?v=" + fetch_video_id(source_url)     # fallback for handle/channel /live forms
duration  = fetch_duration_secs(watch_url)
create playlist_item { url: watch_url, duration, sort_order: 0 }
channel::set_type_and_anchor(VodLoop, anchor = now)
source::deactivate_all_for_channel(...)
```

The conversion is **idempotent** — a channel already in `vod_loop` is left untouched, so concurrent tunes that both observe the ended manifest don't create duplicate items. No schema migration was needed: the recording URL lives on the new `playlist_item`, not the source.

## Notes

**`failed_url` is the raw source URL**, not the resolved playable URL. The player passes the original URL it was given so the server can match it against `src.url` directly.

**Fallback is one level deep.** `/next` skips exactly one named URL. If all other active sources also fail to resolve, 503 is returned immediately — there is no retry loop beyond the source list.

**VOD `start_offset_secs`.** The player uses this value to seek mid-episode, making the channel behave like a broadcast schedule where every viewer sees the same content at the same time.

**VOD `/next` ignores `failed_url`.** The VOD next handler advances to the next playlist item by index and ignores the `failed_url` parameter entirely — VOD items don't have fallback sources.

**Channel metadata in response.** `/tune` and `/next` both include `name`, `logo_url`, `category`, and `channel_type` so the client can render the info bar without a separate fetch.

**`skip_proxy` and `ended` flags.** `skip_proxy` tells the player to use the unproxied resolved URL for `<video src>` (set for resolved YouTube VOD direct MP4s). `ended` signals the live broadcast has finished and conversion has been triggered — the client treats it as a cue to auto-advance, not an error.

# Story: Why YouTube Forces the Proxy — 2026-06-09

## A simple request

It started as a chore: "find me some YouTube live streams for testing." The user already had one — `https://www.youtube.com/@NASA/live` — and wanted more. The honest answer needed two halves, because MyTV does two things with a stream: it tunes it, and it decides whether the browser can play it *directly* or must route it through `/stream-proxy`. That second decision is the budget badge — ⚡ for direct, ☁ for proxied.

So the real question wasn't "which streams are live," it was "which streams earn the ⚡."

## Replaying the budget probe by hand

To answer honestly, we replayed MyTV's own logic in the shell instead of guessing. `budget.rs` was clear about the rule but delegated the actual probe to `health.rs` → `hls::probe_source_cors`: fetch the manifest, descend one level to a real media segment, `HEAD` it, and look for exactly `Access-Control-Allow-Origin: *`. Present means direct. Absent — or anything other than the bare wildcard — means proxy. Proxy is the safe default; a failed probe never grants ⚡.

Seven `@handle/live` URLs resolved cleanly through `yt-dlp`. All seven were live. All seven came back ☁ Proxied.

## The trap we'd already documented

The first attempt to confirm CORS broke in an instructive way. Grepping the media playlist for `.m3u8` to find the variant grabbed a *segment* instead — because YouTube's live segment URLs embed `.m3u8` mid-path: `.../playlist/index.m3u8/sq/222792/.../file/seg.ts`. The shell `curl` happily downloaded a binary `.ts` chunk and printed garbage.

That was the exact gotcha already burned into memory: *use `ends_with`, not `contains`.* MyTV's `find_first_segment_url` gets it right — it splits off the query string and tests `path.ends_with(".m3u8")`, so it skips those decoy URLs and lands on the true segment. The shell, written carelessly, did not. The codebase had already learned this lesson; the human reproducing it by hand had to relearn it for thirty seconds.

## The finding

Once the segment was correctly identified, the `HEAD` was unambiguous. The `googlevideo.com` CDN returned `HTTP 200`, `Content-Type: application/octet-stream`, and **no `Access-Control-Allow-Origin` header at all** — not with a plain request, not even when we sent an explicit `Origin`. The absence is the answer. For cross-origin JavaScript, a missing header is default-deny.

By contrast, when we went hunting for ⚡ streams, Mux, Apple, and Unified Streaming demo assets all sent `Access-Control-Allow-Origin: *` on real 200 segments — they exist to be embedded by arbitrary players. And one Akamai live stream (`cph-p2p-msl`) sent a *reflected* origin — `Access-Control-Allow-Origin: https://kunstv.fly.dev` — rather than `*`. MyTV's strict `== "*"` check correctly treats that as ☁: permissive in practice, but not the bare wildcard the code trusts.

## Reflecting on the choice

Then came the real question, the one worth writing down: *why* does googlevideo omit the header?

It isn't an oversight. It's the mechanism. CORS gates one specific thing — JavaScript reading a cross-origin response — and that is precisely the access YouTube has no reason to grant and every reason to deny:

- **It costs YouTube nothing for real playback.** The IFrame player fetches from Google's own surfaces; a native `<video>` element doesn't need CORS; server-side fetches aren't subject to it. Only third-party *browser* players — hls.js, Shaka, Video.js appending to an MSE `SourceBuffer` — must `fetch()` segments cross-origin. Those are exactly the clients the missing header turns away.
- **The URLs are session-bound.** `ip/…`, `expire/…`, `sig/…`, `spc/…` — each segment URL is minted for the client that negotiated it. Cross-origin JS fetch was never part of that contract.
- **It's a business boundary.** Letting arbitrary origins read raw segments would make it trivial to bypass ads, view counting, and region gating. Withholding CORS keeps playback inside surfaces Google controls.

So the absent header isn't a wall YouTube forgot to open — it's a wall built on purpose, and MyTV's proxy is the sanctioned door. The proxy fetches server-side, where CORS doesn't apply, and re-serves the bytes under MyTV's own origin with its own headers. The ☁ badge isn't a warning that something is wrong; it's an accurate label saying "this stream can only reach the browser through us, and that's by design."

## The payoff

The user's testing list ended up with a clean split that mirrors the architecture:

- **☁ Proxied** — every YouTube `/live` (NASA, Lofi Girl, Sky News, DW, Al Jazeera, NBC, ABC), plus the reflected-origin Akamai live stream. These exercise the proxy path end to end — the realistic case.
- **⚡ Direct** — Mux Apple BipBop, Mux Big Buck Bunny, Apple BipBop advanced, Unified Tears of Steel. These exercise the direct path and prove the badge actually flips.

A request for "some test streams" turned into a confirmation that the budget badge tells the truth, and an explanation of *why* YouTube will always wear the cloud.

## A follow-up: does VOD cost the same?

The natural next question — sharpened by the in-flight ended-live→VOD feature — was whether a *recorded* YouTube video pays the same toll as a live one. We resolved Blender's evergreen "Big Buck Bunny" (`aqz-KE-bpKQ`) and found the first structural surprise: a VOD exposes **no HLS at all**. Where live gives a single combined `.m3u8`, the VOD gave only `https` progressive MP4 and separate DASH streams — no manifest to descend into.

But the media still lived on `googlevideo.com`, and a ranged `GET` (following the obligatory `302`) returned `206 Partial Content`, `video/mp4`, `Accept-Ranges: bytes` — and, once again, **no `Access-Control-Allow-Origin` header**. The CORS posture is a property of the CDN's policy, not of the content type. Live or recorded, same wall.

| Dimension | YouTube **Live** | YouTube **VOD** |
|---|---|---|
| Resolved format | single **HLS** manifest (`.m3u8`) | **no HLS** — progressive MP4 / separate DASH |
| Media host | `*.googlevideo.com` | `*.googlevideo.com` (same CDN) |
| `Access-Control-Allow-Origin` | none | none |
| **Budget badge** | **☁ Proxied** | **☁ Proxied** |
| Delivery | continuous short segments (live/DVR) | one file, `Range`/`206` seekable |

So the **CORS cost is identical** — converting an ended live into a VOD changes nothing about the badge; both wear the cloud for the same reason. What diverges is the **proxy workload behind that identical badge**:

- **Live ☁** — the proxy sits in the path for the whole session, continuously re-fetching advancing segments. Bandwidth ≈ watch duration, nothing caches, the proxy never "finishes."
- **VOD ☁** — one immutable MP4 with range support. Bytes are bounded by file size, seeking is cheap, the content is cache-friendly. A far gentler ☁.

Same badge, very different economics: live is the expensive cloud, VOD is the cheap, bounded, seekable one.

The comparison raised a probe subtlety, and chasing it down corrected a wrong guess. The first instinct was: a progressive-MP4 VOD has no manifest to descend, so `find_segment_with_descent` finds no segment line, returns `None`, and the badge falls to a blank `Unknown`. We tested it instead of trusting it — and the mechanism turned out to be false.

Two facts settled it. First, a unit test: `find_first_segment_url` fed an MP4 decoded as lossy text returns `Some(garbage)`, **not** `None`. The MP4's binary "lines" don't start with `#`, aren't empty, and don't end in `.m3u8`, so the very first one is returned as a (garbage) "segment." `None` only happens when *every* non-comment line is itself a playlist. Second, timing: the 28 MB test VOD downloaded in 2.7 s, well under `fetch_text`'s 10 s budget — so the fetch succeeds.

Following the real Test-button path, then: `resolve_url` yields one `.mp4`, `fetch_text` succeeds, `find_segment_with_descent` returns a garbage segment, `probe_cors` HEADs that malformed googlevideo URL, gets no `Access-Control-Allow-Origin`, and returns `false`. The probe caches **`Some(false)` → ☁ Proxied** — the correct verdict, reached by accident through the safe proxy default rather than by detecting a real segment. A blank `Unknown` *does* occur, but only when `fetch_text` times out (a VOD too large to download in 10 s) or `yt-dlp` resolution fails — rare, and a different cause than first assumed.

Also worth keeping in view: the guide's background sweep skips YouTube entirely (`probe_and_cache_cors` bails on `needs_resolution`), so every YouTube channel — live or VOD — shows a blank badge in the guide until the admin Test button resolves and probes it. The blank badge was never VOD-specific in the first place.

## Production reality: the ⚡ that wasn't CORS

Then production handed us a contradiction. A real channel — `/admin/channels/9`, playing `watch?v=dQw4w9WgXcQ` — wore a **⚡** badge after the Test button. But we had just proven, twice, that googlevideo sends no CORS. A YouTube video flying the lightning bolt should have been impossible.

The network tab told a consistent story: one `next` request, then two `videoplayback` requests, one of which pulled down an **11 MB** body. No HLS, no proxy, no small segments — just a big file being fetched straight from googlevideo. So we chased every thread to ground instead of hand-waving.

The 11 MB resolved first. `resolve_url` selects `-f "b[ext=mp4]/b"`, and for this video that is **format 18 — a single 360p progressive muxed MP4, ~11.8 MB.** The "segment" the network tab showed wasn't a segment at all; it was the entire video file. The two `videoplayback` hits are ordinary `<video src>` behavior for a progressive file: a redirect to the `rrN---…googlevideo.com` host (and/or a metadata range read), then the bulk fetch.

The ⚡ took longer, because it forced us to confront that the badge means two different things. We ran the *actual* probe code against the exact video — an `#[ignore]`'d test calling `probe_source_cors` on the resolved URL — and watched it return `Some(false)`, with a `FIRST SEGMENT` of `https://…googlevideo.com/\0\0\0\u{18}ftypmp42…`: the garbage-segment path, exactly as predicted, landing on ☁. So the probe was *not* the source of the ⚡.

The source was `playlist_item_test`, which for any resolution-needed VOD item does this and nothing more:

```rust
if needs_resolution(&item.url) {
    cors_cache.insert(host, true);   // ⚡ by fiat — no probe
}
```

A YouTube VOD item is marked Direct **outright**, because a VOD item skips the proxy entirely: `tune_vod_at` returns the resolved MP4 with `skip_proxy = true`, and the browser plays it with a native `<video>` element — which loads media cross-origin **without** needing CORS. CORS only ever gated the JavaScript/hls.js fetch path; a media element was never subject to it.

That is the real lesson of the day, and it reframes everything above. **The ⚡ glyph is overloaded:**

- On a **live source or direct HLS**, ⚡ means "hls.js can `fetch()` segments cross-origin because the CDN sends `Access-Control-Allow-Origin: *`." It is a statement about CORS.
- On a **VOD item**, ⚡ means "played directly by `<video>`, the proxy is skipped." It is a statement about the *player*, and CORS is irrelevant.

Both readings honestly imply "no proxy bandwidth in the segment path," so the badge isn't lying. But it explains the otherwise-baffling split: the same YouTube CDN, the same absent CORS header, yet a YouTube **VOD** wears ⚡ while a YouTube **live** wears ☁ — not because their networks differ, but because one is played by a media element and the other by a JavaScript player. The earlier "VOD costs the same as live" analysis was about the *live*-source probe path; the VOD-item path never probes at all. Production didn't contradict the investigation — it revealed a second meaning of the badge we hadn't yet named.

## What remains

- **No reliable Direct *live* stream** — the evergreen Akamai test live streams are retired (Bitmovin Sintel 403s, `moctobpltc` 404s on its level playlists). The confirmed ⚡ streams are all VOD. A live ⚡ fixture would need a source whose CDN sends bare `*` on live segments.
- **The reflected-origin case** (`cph-p2p-msl`) is a good *negative* test for the badge: a stream that sends a CORS header yet still — correctly — does not earn ⚡.
- **The ⚡ glyph carries two meanings** — "CORS-direct segments" (live/HLS) and "proxy-skipped direct play" (VOD items). It's accurate either way, but a viewer can't tell *which* from the icon alone. If that ambiguity ever bites, the fix is a distinct glyph or tooltip for the VOD-direct case, not a change to the probe.

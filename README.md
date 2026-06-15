# MyTV

[![CI](https://github.com/Wkkkkk/MyTV/actions/workflows/ci.yml/badge.svg)](https://github.com/Wkkkkk/MyTV/actions/workflows/ci.yml)

A personal web app that repackages live internet streams and VOD content into a cable TV–style experience. Browse channels through an EPG grid, click to watch. No app installs, no playlist management at watch time.

---

## What it does

- **EPG guide** at `/guide` — channel grid organized by category, 24-hour window, now-line, category tabs and time scrolling
- **Live channels** — multiple failover sources per channel; yt-dlp resolves YouTube and Twitch URLs to direct HLS at tune-in time
- **VOD loop channels** — playlists that run continuously like a FAST channel; every viewer sees the same position. YouTube VOD plays as direct MP4 straight from the CDN (no proxy hop)
- **On-demand VOD channels** — a viewer-controlled playlist: items play in order, you click any item to jump or replay, and the player remembers your position in the browser. No shared clock and no loop — playback stops after the last item. Ideal for self-hosted MP4 files; seeking uses the native video timeline
- **Ended-live → VOD** — when a YouTube live broadcast ends, the player shows a brief overlay, auto-advances to the next channel, and the dead channel is converted into a replayable VOD loop in the background
- **Budget badges** — the guide marks each channel ⚡ (loads direct from the CDN) or ☁ (routed through the proxy) based on probed CORS support, including resolved YouTube/Twitch live streams
- **Admin UI** at `/admin` — manage channels, sources, and playlist items
- **Discovery** at `/admin/discover` — find streams via the iptv-org M3U index, YouTube search, or manual URL entry
- **JSON API & CLI** at `/api/admin` — every admin operation (channel/source/playlist CRUD, discovery, tune-testing) is scriptable as JSON, with a `mytvctl` command-line client in front of it

---

## How it works

**VOD sync** — each VOD *loop* channel has a fixed anchor timestamp. The server computes the current playback position by dividing elapsed time since the anchor by the total playlist duration. Every viewer requesting `/tune` at the same moment gets the same offset, giving the illusion of a shared broadcast.

**On-demand playback** — an on-demand channel has no anchor and no shared clock. The browser drives it: it loads the item list from `/channel/:id/playlist`, plays each item via `/channel/:id/item/:item_id`, advances to the next on `ended`, and stores the current item and offset in `localStorage` so you resume where you left off. Playback stops after the last item (no loop).

**Live failover** — sources are tried in order. When a source fails mid-stream, the player calls `/next` with the failed URL so the server can skip it and return the next available source. The cycle restarts from the top once all sources have been tried.

**Stream proxy** — browsers block cross-origin HLS manifests. `/stream-proxy?url=…` fetches the manifest server-side and rewrites segment and sub-manifest URLs so subsequent requests also go through the proxy. This makes any public HLS stream embeddable without CORS issues. When a CDN already sends permissive CORS headers, segments load direct from origin and only the manifest is proxied; resolved YouTube VOD MP4s skip the proxy entirely.

**yt-dlp resolution** — YouTube and Twitch URLs are not stored as stream URLs. At tune-in time the server calls yt-dlp to resolve the current direct URL — an HLS manifest for live, a single combined MP4 for VOD. This keeps streams working as platforms rotate their internal URLs.

**Ended-live conversion** — yt-dlp marks a finished live broadcast with a `force_finished` manifest. The server detects this at tune time, returns an "ended" signal so the player can move on, and converts the channel into a VOD loop pointing at the recording (`watch?v=<id>`) — appending a playlist item, flipping the channel type, and deactivating the now-dead live sources. The conversion is idempotent and needs no schema change.

**JSON API & CLI** — `/api/admin/**` mirrors the form admin as JSON CRUD for channels, sources, and playlist items (plus `toggle`/`test`) and exposes discovery (`/discover/m3u`, `/discover/youtube`, `/resolve`, `/channel`, `/add`). It sits behind the same basic-auth as the admin UI; errors funnel through a single `{"error": "..."}` shape (404/422/503/500). The `mytvctl` binary is a thin client over it, configured by `MYTV_BASE_URL` + `MYTV_ADMIN_PASSWORD` (password env-only):

```bash
export MYTV_BASE_URL=http://localhost:3000
export MYTV_ADMIN_PASSWORD=admin
mytvctl channel list
mytvctl channel create --name "News" --category "Live" --type live
mytvctl discover youtube --keyword "lofi" --type video
```

---

## Player controls

| Key | Action |
|-----|--------|
| `Space` | Play / pause |
| `F` | Toggle fullscreen |
| `↑` / `↓` | Previous / next channel (respects active EPG category filter) |
| `←` / `→` | Seek −10s / +10s (VOD loop channels only) |

The channel info bar below the video shows the channel logo (or a coloured initial tile if no logo is set), name, category, and position in the current channel list. On **on-demand** channels, use the on-screen playlist (☰ in the player toolbar) to click between items, and the native video timeline to seek.

---

## Creating an on-demand channel

In the admin UI (`/admin`):

1. **Channels → New Channel** — set **Type** to **On-demand playlist**, add a name and category, and save. (No loop anchor is needed.)
2. Open the channel and use the **Playlist** section's **Add Item** form — one title and media URL per item. For a direct `.mp4`/`.webm`/`.m4v`/`.mov` URL the duration auto-fills from the browser; otherwise type it in. Items play in the order you add them.
3. Watch at `/watch/<id>` (or click the channel in the guide): click any item in the on-screen playlist (☰) to jump or replay, and drag the video timeline to seek.

Scriptable via `mytvctl` (or the JSON API behind it):

```bash
# create the channel — note the id in the JSON output
mytvctl channel create --name "Movie Night" --category "Film" --type vod_on_demand
# add items to it (use the id from above)
mytvctl playlist create --channel 7 --title "Part 1" --url https://example.com/p1.mp4 --duration-secs 1800
mytvctl playlist create --channel 7 --title "Part 2" --url https://example.com/p2.mp4 --duration-secs 1500
```

---

## Scope

MyTV is a single-user personal server. It does not transcode, record, or manage multi-user access. It has no built-in HTTPS — put a reverse proxy in front for public deployments. It assumes you supply the stream URLs; it does not crawl or scrape content automatically.

---

## Quick start

```bash
git clone https://github.com/Wkkkkk/MyTV.git
cd MyTV
cargo run
```

Open `http://localhost:3000/guide` for the EPG guide or `http://localhost:3000/admin` for the admin UI (default password: `admin`).

For full setup instructions, configuration options, and deployment guides, see the [docs](docs/) folder:

- [Setup & development](docs/SETUP.md)
- [Configuration](docs/CONFIGURATION.md)
- [Deployment](docs/DEPLOYMENT.md)

# MyTV

[![CI](https://github.com/Wkkkkk/MyTV/actions/workflows/ci.yml/badge.svg)](https://github.com/Wkkkkk/MyTV/actions/workflows/ci.yml)

A personal web app that repackages live internet streams and VOD content into a cable TV–style experience. Browse channels through an EPG grid, click to watch. No app installs, no playlist management at watch time.

---

## What it does

- **EPG guide** at `/guide` — channel grid organized by category, 24-hour window, now-line, category tabs and time scrolling
- **Live channels** — multiple failover sources per channel; yt-dlp resolves YouTube and Twitch URLs to direct HLS at tune-in time
- **VOD loop channels** — playlists that run continuously like a FAST channel; every viewer sees the same position
- **Admin UI** at `/admin` — manage channels, sources, and playlist items
- **Discovery** at `/admin/discover` — find streams via the iptv-org M3U index, YouTube search, or manual URL entry

---

## How it works

**VOD sync** — each VOD channel has a fixed anchor timestamp. The server computes the current playback position by dividing elapsed time since the anchor by the total playlist duration. Every viewer requesting `/tune` at the same moment gets the same offset, giving the illusion of a shared broadcast.

**Live failover** — sources are tried in order. When a source fails mid-stream, the player calls `/next` with the failed URL so the server can skip it and return the next available source. The cycle restarts from the top once all sources have been tried.

**Stream proxy** — browsers block cross-origin HLS manifests. `/stream-proxy?url=…` fetches the manifest server-side and rewrites segment and sub-manifest URLs so subsequent requests also go through the proxy. This makes any public HLS stream embeddable without CORS issues.

**yt-dlp resolution** — YouTube and Twitch URLs are not stored as stream URLs. At tune-in time the server calls yt-dlp to resolve the current direct HLS URL. This keeps streams working as platforms rotate their internal URLs.

---

## Player controls

| Key | Action |
|-----|--------|
| `Space` | Play / pause |
| `F` | Toggle fullscreen |
| `↑` / `↓` | Previous / next channel (respects active EPG category filter) |
| `←` / `→` | Seek −10s / +10s (VOD channels only) |

The channel info bar below the video shows the channel logo (or a coloured initial tile if no logo is set), name, category, and position in the current channel list.

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

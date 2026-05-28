# MyTV

A personal web app that repackages live internet streams and VOD content into a cable TV–style experience. Browse channels through an EPG grid, click to watch. No app installs, no playlist management at watch time.

---

## What it does

- **EPG guide** at `/guide` — channel grid organized by category, 24h window, now-line, HTMX-powered category tabs and time scrolling
- **Live channels** — multiple failover sources per channel, yt-dlp resolves YouTube streams to direct HLS at tune-in time
- **VOD loop channels** — playlists that loop continuously like a FAST channel; position computed server-side so every viewer is "in sync"
- **Admin UI** at `/admin` — manage channels, sources, and playlist items
- **Discovery tools** at `/admin/discover` — find streams via iptv-org M3U index, YouTube search, or manual URL entry

---

## Requirements

| Dependency | Purpose |
|---|---|
| [Rust](https://rustup.rs) (stable, 1.75+) | Build the binary |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | Resolve YouTube/platform URLs to direct HLS streams at runtime |
| SQLite | Embedded — no separate server needed |

`yt-dlp` must be on `PATH` when the server runs. Install it with:

```bash
# macOS
brew install yt-dlp

# Linux
pip install yt-dlp
# or download the standalone binary from the releases page
```

---

## Running locally

**1. Clone and build**

```bash
git clone <repo-url>
cd MyTV
cargo build --release
```

**2. Create a `.env` file** (optional — all vars have defaults)

```env
DATABASE_URL=sqlite:mytv.db
ADMIN_PASSWORD=changeme
PORT=3000
# YOUTUBE_API_KEY=AIza...   # optional — enables YouTube search in /admin/discover
```

**3. Run**

```bash
cargo run --release
# or with environment variables inline:
ADMIN_PASSWORD=changeme cargo run --release
```

The server starts on `http://localhost:3000`. The SQLite database (`mytv.db`) is created automatically on first run and migrations are applied.

**4. Open the app**

| URL | What |
|---|---|
| `http://localhost:3000/guide` | EPG channel guide |
| `http://localhost:3000/admin` | Admin UI (password required) |
| `http://localhost:3000/health` | Health check (returns 200 OK) |

---

## Environment variables

| Variable | Default | Required | Notes |
|---|---|---|---|
| `DATABASE_URL` | `sqlite:mytv.db` | No | Path to SQLite file. Use an absolute path in production. |
| `ADMIN_PASSWORD` | `admin` | **Yes** | Protects the `/admin` UI via HTTP Basic Auth. Change this. |
| `PORT` | `3000` | No | TCP port the server listens on. |
| `YOUTUBE_API_KEY` | _(unset)_ | No | YouTube Data API v3 key. Without it, the YouTube tab in Discover shows a configuration message. |
| `RUST_LOG` | _(unset)_ | No | Log level filter, e.g. `info`, `mytv=debug`. |

---

## Getting a YouTube API key (optional)

1. Go to [Google Cloud Console](https://console.cloud.google.com)
2. Create a project (or reuse an existing one)
3. Enable **YouTube Data API v3**
4. Create an **API key** credential (no OAuth needed)
5. Set `YOUTUBE_API_KEY=<your-key>` in your environment

The free tier quota (10,000 units/day) is sufficient for personal use — a keyword search costs ~100 units.

---

## Deployment

### What to prepare

**1. Build a release binary**

```bash
cargo build --release
# binary is at: target/release/mytv
```

The binary is self-contained (statically linked via rustls). Copy it to the server alongside the `templates/` and `migrations/` directories — Askama templates and SQL migrations are embedded at compile time, so only the binary is needed at runtime.

> **Note:** Askama compiles templates into the binary. `migrations/` is also embedded via `sqlx::migrate!`. You only need to ship the binary itself.

**2. Install yt-dlp on the server**

```bash
# Debian/Ubuntu
pip install yt-dlp

# Or download the standalone binary:
curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp
chmod +x /usr/local/bin/yt-dlp
```

Verify it's on PATH: `yt-dlp --version`

**3. Choose a database location**

Set `DATABASE_URL` to an absolute path so the file doesn't move if the working directory changes:

```env
DATABASE_URL=sqlite:/var/lib/mytv/mytv.db
```

Create the directory first: `mkdir -p /var/lib/mytv`

**4. Set a strong admin password**

```env
ADMIN_PASSWORD=<long-random-string>
```

**5. Put it behind a reverse proxy (recommended)**

MyTV speaks plain HTTP. For HTTPS and a domain name, put nginx or Caddy in front:

**Caddy** (`Caddyfile`):
```
tv.yourdomain.com {
    reverse_proxy localhost:3000
}
```

**nginx** (`/etc/nginx/sites-available/mytv`):
```nginx
server {
    listen 80;
    server_name tv.yourdomain.com;
    location / {
        proxy_pass http://localhost:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

### Running as a systemd service (Linux)

Create `/etc/systemd/system/mytv.service`:

```ini
[Unit]
Description=MyTV
After=network.target

[Service]
Type=simple
User=mytv
WorkingDirectory=/opt/mytv
ExecStart=/opt/mytv/mytv
Restart=on-failure
RestartSec=5

Environment=DATABASE_URL=sqlite:/var/lib/mytv/mytv.db
Environment=ADMIN_PASSWORD=changeme
Environment=PORT=3000
Environment=RUST_LOG=info
# Environment=YOUTUBE_API_KEY=AIza...

[Install]
WantedBy=multi-user.target
```

```bash
# Create a dedicated user
useradd -r -s /bin/false mytv

# Copy the binary
mkdir -p /opt/mytv
cp target/release/mytv /opt/mytv/

# Start
systemctl daemon-reload
systemctl enable --now mytv
systemctl status mytv
```

### Keeping yt-dlp up to date

YouTube changes its internal APIs frequently and yt-dlp releases patches weekly. Add a cron job:

```bash
# Run weekly, Sunday 3am
0 3 * * 0 /usr/local/bin/yt-dlp -U
```

Or with pip: `0 3 * * 0 pip install -q --upgrade yt-dlp`

---

## Development tips

**Run tests:**
```bash
cargo test
```

**Enable debug logging:**
```bash
RUST_LOG=mytv=debug cargo run
```

**Watch for template changes** (templates are compiled into the binary, so a rebuild is needed on change):
```bash
cargo run  # just re-run; Cargo detects changes
```

**Reset the database:**
```bash
rm mytv.db && cargo run  # migrations run automatically on startup
```

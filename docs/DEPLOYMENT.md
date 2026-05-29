# Deployment

## Preparing a release build

```bash
cargo build --release
# binary is at: target/release/mytv
```

The binary is self-contained. Askama templates and SQL migrations are embedded at compile time — only the binary itself is needed at runtime.

Install yt-dlp on the server:

```bash
# Debian/Ubuntu
pip install yt-dlp

# Or download the standalone binary:
curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp
chmod +x /usr/local/bin/yt-dlp
```

Set `DATABASE_URL` to an absolute path so the file doesn't move if the working directory changes:

```env
DATABASE_URL=sqlite:/var/lib/mytv/mytv.db
```

---

## Running as a systemd service (Linux)

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
useradd -r -s /bin/false mytv
mkdir -p /opt/mytv /var/lib/mytv
cp target/release/mytv /opt/mytv/
systemctl daemon-reload
systemctl enable --now mytv
systemctl status mytv
```

---

## Reverse proxy (recommended)

MyTV speaks plain HTTP. For HTTPS and a custom domain, put nginx or Caddy in front.

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

---

## Deploying to Fly.io

**One-time setup:**

```bash
# Install the Fly CLI: https://fly.io/docs/hands-on/install-flyctl/
fly auth login
fly launch --no-deploy          # say yes to use the existing fly.toml; pick a unique app name
fly volumes create mytv_data --region ams --size 1
fly secrets set ADMIN_PASSWORD=<strong-password>
# fly secrets set YOUTUBE_API_KEY=<key>
fly deploy
```

**Subsequent deploys:**

```bash
fly deploy
```

**Useful commands:**

```bash
fly logs          # tail logs
fly ssh console   # open a shell on the running machine
```

---

## Keeping yt-dlp up to date

YouTube changes its internal APIs frequently; yt-dlp releases patches weekly. Add a cron job on the server:

```bash
# Weekly, Sunday 3am
0 3 * * 0 /usr/local/bin/yt-dlp -U
# or with pip:
0 3 * * 0 pip install -q --upgrade yt-dlp
```

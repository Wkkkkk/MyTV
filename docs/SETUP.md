# Setup & development

## Stack

| Component | Role |
|---|---|
| [Rust / Axum](https://github.com/tokio-rs/axum) | HTTP server |
| [SQLite via sqlx](https://github.com/launchbadge/sqlx) | Embedded database — no separate server |
| [Askama](https://github.com/djc/askama) | HTML templates compiled into the binary |
| [HTMX](https://htmx.org) | Partial-page updates without a JS framework |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | Resolves YouTube / Twitch URLs to direct HLS at runtime |

The server ships as a single binary. Askama templates and SQL migrations are embedded at compile time — only the binary and a writable path for the SQLite file are needed at runtime.

---

## Requirements

| Dependency | Purpose |
|---|---|
| [Rust](https://rustup.rs) (via rustup) | Build the binary — exact version pinned by `rust-toolchain.toml` |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | Resolve YouTube/platform URLs at runtime |
| SQLite | Embedded — no separate server needed |

Install yt-dlp:

```bash
# macOS
brew install yt-dlp

# Linux
pip install yt-dlp
# or download the standalone binary from the releases page
```

yt-dlp must be on `PATH` when the server runs.

---

## Running locally

**1. Clone**

```bash
git clone https://github.com/Wkkkkk/MyTV.git
cd MyTV
```

**2. Install git hooks** (once after cloning)

```bash
./scripts/install-hooks.sh
```

This installs a pre-push hook that runs `cargo fmt --check`, `cargo clippy --all-targets`, and `cargo test` before every push.

**3. Create a `.env` file** (optional — all vars have defaults)

```env
DATABASE_URL=sqlite:mytv.db
ADMIN_PASSWORD=changeme
PORT=3000
# YOUTUBE_API_KEY=AIza...   # optional — enables YouTube search in /admin/discover
```

**4. Run**

```bash
cargo run
```

The server starts on `http://localhost:3000`. The SQLite database (`mytv.db`) is created automatically on first run and migrations are applied.

**5. Open the app**

| URL | What |
|---|---|
| `http://localhost:3000/guide` | EPG channel guide |
| `http://localhost:3000/admin` | Admin UI (password required) |
| `http://localhost:3000/health` | Health check |

---

## Development tips

**Format before committing** (CI will reject any diff):
```bash
cargo fmt
```

**Run tests** (226 tests: unit + integration; integration tests use an in-memory SQLite DB seeded automatically — no setup needed):
```bash
cargo test
```

**Enable debug logging:**
```bash
RUST_LOG=mytv=debug cargo run
```

**Reset the database:**
```bash
rm mytv.db && cargo run
```

Templates are compiled into the binary, so a `cargo run` re-run is all that's needed after template changes — Cargo detects them automatically.

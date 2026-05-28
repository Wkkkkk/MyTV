# GitHub Publishing & CI/CD Design

**Date:** 2026-05-28  
**Project:** MyTV (Rust/Axum web server)  
**Status:** Approved

## Overview

Publish MyTV to a **public** GitHub repository, add a local pre-push git hook for fast feedback, set up GitHub Actions CI (build + Docker validation), and prepare a Fly.io deployment configuration for manual deploys.

---

## Section 1 — GitHub Repository

- Create a new **public** repository on GitHub under the user's account.
- Rename the local `master` branch to `main` (`git branch -m master main`) to match GitHub's default.
- Push to GitHub and set `main` as the default branch.
- `.env` and `*.db` are already gitignored — no secrets will be exposed.
- No changes to `.gitignore` needed.

---

## Section 2 — Local Pre-Push Hook

A pre-push hook that runs automatically before every `git push`.

**File:** `scripts/pre-push.sh` (tracked in repo)  
**Installed to:** `.git/hooks/pre-push` via `scripts/install-hooks.sh`

Hook steps (fail fast — stops on first error):
1. `cargo fmt --check` — reject unformatted code
2. `cargo clippy -- -D warnings` — reject clippy warnings
3. `cargo test` — reject broken tests

**`scripts/install-hooks.sh`** copies `scripts/pre-push.sh` into `.git/hooks/pre-push` and makes it executable. Run once after cloning.

---

## Section 3 — GitHub Actions CI

**File:** `.github/workflows/ci.yml`

**Triggers:**
- Push to `main`
- Pull requests targeting `main`

**Jobs:**

### `ci` job (single job, sequential steps)
1. `actions/checkout@v4`
2. `dtolnay/rust-toolchain@stable` with components: `rustfmt`, `clippy`
3. `Swatinem/rust-cache@v2` — caches `~/.cargo/registry` and `target/`
4. `cargo fmt --check`
5. `cargo clippy -- -D warnings`
6. `cargo test`
7. `docker build .` — validates the Dockerfile builds cleanly (no push)

---

## Section 4 — Dockerfile

Multi-stage build to produce a minimal runtime image.

**Stage 1 — builder** (`rust:1-slim`):
- Install build dependencies (`pkg-config`, `libssl-dev` for linking)
- Copy `Cargo.toml`, `Cargo.lock`, then source
- Run `cargo build --release`

**Stage 2 — runtime** (`debian:bookworm-slim`):
- Copy compiled binary from builder
- Copy `templates/` and `migrations/` directories
- Set `ENV PORT=3000`
- Expose port 3000
- `CMD ["./mytv"]`

Note: `reqwest` uses `rustls-tls` (not OpenSSL), so no SSL runtime library is needed.

---

## Section 5 — Fly.io Configuration

**File:** `fly.toml`

Key settings:
- App name: `mytv` (or user-chosen)
- Primary region: user-chosen at launch time
- Internal port: `3000`
- Persistent volume `mytv_data` (1 GB) mounted at `/data`
- `DATABASE_URL=sqlite:/data/mytv.db` (volume path, not local)
- `RUST_LOG=info` set in `fly.toml`
- Secrets (`ADMIN_PASSWORD`, `YOUTUBE_API_KEY`) set via `fly secrets set` — never in `fly.toml`

**Health check:** `GET /health` (already implemented)

**One-time manual setup** (documented in README):
```
fly auth login
fly launch --no-deploy
fly volumes create mytv_data --region <region> --size 1
fly secrets set ADMIN_PASSWORD=<password> YOUTUBE_API_KEY=<key>
fly deploy
```

**Subsequent deploys:**
```
fly deploy
```

---

## Out of Scope

- Automatic CD (no deploy triggered from GitHub Actions)
- GitHub Container Registry / image tagging
- Staging environment
- Database migrations run automatically on deploy (operator runs `fly ssh console` if needed)

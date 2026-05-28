# GitHub Publishing & CI/CD Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish MyTV to a public GitHub repo, add a local pre-push hook, GitHub Actions CI (build + Docker validation), a production Dockerfile, and a Fly.io config for manual deploys.

**Architecture:** Five independent artifacts land in one branch: git hook scripts, a GitHub Actions workflow, a multi-stage Dockerfile, and a `fly.toml`. Each commit is self-contained and leaves the repo in a working state.

**Tech Stack:** Rust/Cargo, GitHub Actions, Docker (multi-stage), Fly.io, yt-dlp (runtime dep)

---

## File Map

| Action | Path | Purpose |
|--------|------|---------|
| Create | `scripts/pre-push.sh` | The actual hook that runs `fmt/clippy/test` |
| Create | `scripts/install-hooks.sh` | One-shot script to install the hook after cloning |
| Create | `.github/workflows/ci.yml` | GitHub Actions CI pipeline |
| Create | `Dockerfile` | Multi-stage build; runtime image is binary + yt-dlp only |
| Create | `fly.toml` | Fly.io app config with persistent volume for SQLite |
| Modify | `README.md` | Add: install-hooks step, CI badge, Fly.io deploy section |

---

## Task 1: Publish the repo to GitHub

**Files:** none (git operations only)

- [ ] **Step 1: Rename the local branch from `master` to `main`**

```bash
git branch -m master main
```

- [ ] **Step 2: Create a public GitHub repo and push**

Requires the `gh` CLI authenticated with your GitHub account (`gh auth status` to check). If not installed: https://cli.github.com

```bash
gh repo create MyTV --public --source=. --remote=origin --push
```

This creates `github.com/<your-username>/MyTV`, sets it as `origin`, and pushes `main`.

- [ ] **Step 3: Verify**

```bash
gh repo view --web
```

Expected: browser opens to your new public repo showing the README.

---

## Task 2: Add the pre-push git hook

**Files:**
- Create: `scripts/pre-push.sh`
- Create: `scripts/install-hooks.sh`

- [ ] **Step 1: Create `scripts/pre-push.sh`**

```bash
mkdir -p scripts
```

Contents of `scripts/pre-push.sh`:

```sh
#!/bin/sh
set -e

echo "pre-push: cargo fmt --check"
cargo fmt --check

echo "pre-push: cargo clippy"
cargo clippy -- -D warnings

echo "pre-push: cargo test"
cargo test

echo "pre-push: all checks passed"
```

- [ ] **Step 2: Create `scripts/install-hooks.sh`**

Contents of `scripts/install-hooks.sh`:

```sh
#!/bin/sh
set -e

cp scripts/pre-push.sh .git/hooks/pre-push
chmod +x .git/hooks/pre-push
echo "Installed .git/hooks/pre-push"
```

- [ ] **Step 3: Make both scripts executable**

```bash
chmod +x scripts/pre-push.sh scripts/install-hooks.sh
```

- [ ] **Step 4: Install the hook locally**

```bash
./scripts/install-hooks.sh
```

Expected output:
```
Installed .git/hooks/pre-push
```

- [ ] **Step 5: Verify the hook runs**

```bash
sh .git/hooks/pre-push
```

Expected: `cargo fmt --check`, `cargo clippy`, `cargo test` all pass, ending with `pre-push: all checks passed`.

If clippy fails with existing warnings, fix them before continuing (run `cargo clippy -- -D warnings` to see what to fix).

- [ ] **Step 6: Commit**

```bash
git add scripts/pre-push.sh scripts/install-hooks.sh
git commit -m "chore: add pre-push hook and install script"
```

- [ ] **Step 7: Push to GitHub**

```bash
git push origin main
```

---

## Task 3: Add GitHub Actions CI

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create `.github/workflows/ci.yml`**

```bash
mkdir -p .github/workflows
```

Contents of `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always

jobs:
  ci:
    name: CI
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - uses: Swatinem/rust-cache@v2

      - name: fmt
        run: cargo fmt --check

      - name: clippy
        run: cargo clippy -- -D warnings

      - name: test
        run: cargo test

      - name: docker build
        run: docker build .
```

- [ ] **Step 2: Commit and push**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add GitHub Actions CI workflow"
git push origin main
```

- [ ] **Step 3: Verify CI runs on GitHub**

```bash
gh run list --limit 5
```

Expected: a run named `CI` appears with status `in_progress` or `success`. To watch live:

```bash
gh run watch
```

---

## Task 4: Add the Dockerfile

**Files:**
- Create: `Dockerfile`

Key facts that shape this Dockerfile:
- Askama compiles templates into the binary at build time — runtime image needs no `templates/` dir.
- `sqlx::migrate!` embeds migrations into the binary at build time — runtime image needs no `migrations/` dir.
- `reqwest` uses `rustls-tls` (no OpenSSL) — no SSL runtime libs needed.
- `yt-dlp` is called via `Command::new("yt-dlp")` at runtime — must be on PATH in the runtime image.

- [ ] **Step 1: Create `Dockerfile`**

```dockerfile
# syntax=docker/dockerfile:1

# ── Stage 1: build ──────────────────────────────────────────────────────────
FROM rust:1-slim AS builder

WORKDIR /app

# Build dependencies layer (cached unless Cargo.toml/Cargo.lock change)
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Build the real binary (migrations dir required by sqlx::migrate! at compile time)
COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs && cargo build --release

# ── Stage 2: runtime ────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# ca-certificates: needed by reqwest (rustls) for HTTPS
# curl: used to download yt-dlp standalone binary
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp \
         -o /usr/local/bin/yt-dlp \
    && chmod +x /usr/local/bin/yt-dlp \
    && apt-get purge -y curl \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/mytv ./mytv

ENV PORT=8080
ENV RUST_LOG=info

EXPOSE 8080

CMD ["./mytv"]
```

- [ ] **Step 2: Verify the image builds**

```bash
docker build -t mytv-test .
```

Expected: build completes, last line is something like `Successfully built <id>` or `=> exporting to image`. This will take a few minutes on first run (downloading Rust toolchain layer).

- [ ] **Step 3: Smoke-test the image locally**

```bash
docker run --rm -p 8080:8080 \
  -e DATABASE_URL=sqlite:/tmp/test.db \
  -e ADMIN_PASSWORD=test \
  mytv-test
```

In a second terminal:

```bash
curl -s http://localhost:8080/health
```

Expected: `200 OK` response (the health route returns a plain 200).

Stop the container with Ctrl-C.

- [ ] **Step 4: Commit**

```bash
git add Dockerfile
git commit -m "build: add multi-stage Dockerfile with yt-dlp"
git push origin main
```

---

## Task 5: Add fly.toml

**Files:**
- Create: `fly.toml`

Note: The app name in `fly.toml` must be globally unique on Fly.io. Replace `mytv-app` below with your chosen name (you'll pick it during `fly launch`).

- [ ] **Step 1: Create `fly.toml`**

```toml
# fly.toml — Fly.io application configuration
# Deploy manually with: fly deploy

app = "mytv-app"
primary_region = "ams"

[build]

[env]
  PORT = "8080"
  RUST_LOG = "info"
  DATABASE_URL = "sqlite:/data/mytv.db"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = true
  auto_start_machines = true
  min_machines_running = 0

  [http_service.concurrency]
    type = "connections"
    hard_limit = 25
    soft_limit = 20

[[vm]]
  memory = "256mb"
  cpu_kind = "shared"
  cpus = 1

[[mounts]]
  source = "mytv_data"
  destination = "/data"
  initial_size = "1gb"
```

- [ ] **Step 2: Validate the config (requires Fly CLI)**

If you don't have the Fly CLI: https://fly.io/docs/hands-on/install-flyctl/

```bash
fly config validate
```

Expected: `App config is valid`

If you don't have the CLI yet, skip this step and validate after installing it.

- [ ] **Step 3: Commit**

```bash
git add fly.toml
git commit -m "deploy: add fly.toml for Fly.io"
git push origin main
```

---

## Task 6: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a CI badge just below the title in `README.md`**

After the `# MyTV` heading, add:

```markdown
[![CI](https://github.com/<your-username>/MyTV/actions/workflows/ci.yml/badge.svg)](https://github.com/<your-username>/MyTV/actions/workflows/ci.yml)
```

Replace `<your-username>` with your actual GitHub username.

- [ ] **Step 2: Add a "Development setup" section after the "Requirements" section**

Add the following block after the existing `## Requirements` section:

```markdown
## Development setup

**Install git hooks** (run once after cloning):

```bash
./scripts/install-hooks.sh
```

This installs a pre-push hook that runs `cargo fmt --check`, `cargo clippy`, and `cargo test` before every push.
```

- [ ] **Step 3: Add a "Deploying to Fly.io" section at the end of the Deployment section**

Add the following block after the existing `## Deployment` section (before `## Development tips`):

```markdown
### Deploying to Fly.io

**One-time setup:**

1. Install the Fly CLI: https://fly.io/docs/hands-on/install-flyctl/
2. Log in: `fly auth login`
3. Create the app (choose a unique name when prompted):
   ```bash
   fly launch --no-deploy
   ```
   When asked about an existing `fly.toml`, say **yes** to use it.
4. Create the persistent volume for the SQLite database:
   ```bash
   fly volumes create mytv_data --region ams --size 1
   ```
5. Set secrets:
   ```bash
   fly secrets set ADMIN_PASSWORD=<strong-password>
   # Optional: fly secrets set YOUTUBE_API_KEY=<key>
   ```
6. Deploy:
   ```bash
   fly deploy
   ```

**Subsequent deploys:**

```bash
fly deploy
```

**Check logs:**

```bash
fly logs
```

**Open a console on the running machine:**

```bash
fly ssh console
```
```

- [ ] **Step 4: Commit and push**

```bash
git add README.md
git commit -m "docs: add CI badge, install-hooks step, and Fly.io deploy guide"
git push origin main
```

---

## Verification Checklist

After all tasks are complete:

- [ ] `github.com/<your-username>/MyTV` is public and shows the README with a green CI badge
- [ ] `git push` triggers the pre-push hook locally
- [ ] A push to `main` triggers the GitHub Actions `CI` workflow and all steps pass
- [ ] `docker build .` completes without errors locally
- [ ] `fly config validate` reports `App config is valid`

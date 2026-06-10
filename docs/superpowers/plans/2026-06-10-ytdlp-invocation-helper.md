# yt-dlp Invocation Helper (U8 + kill_on_drop) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kill orphaned yt-dlp processes on timeout/cancellation, then fold the five duplicated yt-dlp invocation bodies in `src/media/resolver.rs` into one capped, kill-on-drop helper.

**Architecture:** Two commits per the approved spec (`docs/superpowers/specs/2026-06-10-ytdlp-invocation-helper-design.md`). Commit 1 adds `.kill_on_drop(true)` at the five existing call sites — a minimal, deployable correctness fix. Commit 2 introduces a private `YtDlpError` enum and two private helpers — `yt_dlp_output` (scheme check, concurrency cap, timeout, kill_on_drop, `--` guard, exactly once) and `yt_dlp_print` (the three `--print` fetchers) — then rewrites the five public functions on top of them. Public signatures and error strings are preserved.

**Tech Stack:** Rust 1.96 (pinned), tokio 1.52 (`process`, `sync::Semaphore`, `time::timeout`), anyhow. All work is in `src/media/resolver.rs`; tests live in the same file's `#[cfg(test)] mod tests`.

**Context for the worker:**
- `cargo fmt` MUST be run before every commit (CI fails on any formatting diff).
- `cargo clippy -- -D warnings` must be clean at each commit.
- Current suite: 313 tests (243 unit + 70 integration, 5 ignored needing yt-dlp/network). All must keep passing.
- `run_under_cap`, `yt_dlp_semaphore`, `interpret_is_live`, `LiveStatus`, and all five public functions already exist in `src/media/resolver.rs` — read the whole file first; it is ~460 lines.
- Why `kill_on_drop` matters: tokio's `Command::output()` does NOT set it (default `false`). When `tokio::time::timeout` fires, the output future is dropped and the semaphore permit frees, but the yt-dlp process survives orphaned (~73 MB) — defeating the 2-permit OOM cap. `.kill_on_drop(true)` makes dropping the future SIGKILL the child.

---

### Task 1: Add `.kill_on_drop(true)` at the five existing call sites (Commit 1)

**Files:**
- Modify: `src/media/resolver.rs` (functions `probe_live`, `resolve_url`, `fetch_title`, `fetch_video_id`, `fetch_duration_secs`)

No new test: asserting kill-on-drop means spawning real processes and inspecting the process table — flaky for little value (decision recorded in the spec). The 313 existing tests guard against regressions.

- [ ] **Step 1: Edit the five `Command::new("yt-dlp")` chains**

In each of the five functions, insert `.kill_on_drop(true)` immediately after `Command::new("yt-dlp")`. Example — in `probe_live`:

```rust
            Command::new("yt-dlp")
                .kill_on_drop(true)
                .args(["--print", "is_live", "--no-playlist", "--", url])
                .output(),
```

Apply identically in `resolve_url` (args `["-g", "--no-playlist", "-f", "b[ext=mp4]/b", "--", url]`), `fetch_title` (`["--print", "title", "--no-playlist", "--", url]`), `fetch_video_id` (`["--print", "id", "--no-playlist", "--", url]`), and `fetch_duration_secs` (`["--print", "duration", "--no-playlist", "--", url]`). Five insertions total — verify with:

Run: `grep -c "kill_on_drop(true)" src/media/resolver.rs`
Expected: `5`

- [ ] **Step 2: Verify build, lints, tests**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: clippy clean; 313 tests pass (243 unit: 239 passed + 4 ignored; 70 integration: 69 passed + 1 ignored).

- [ ] **Step 3: Commit**

```bash
git add src/media/resolver.rs
git commit -m "fix: kill timed-out yt-dlp processes instead of orphaning them

tokio's Command::output() does not set kill_on_drop (default false), so
when the command timeout fired, the output future was dropped and the
semaphore permit freed — but yt-dlp kept running orphaned at ~73 MB,
defeating the 2-permit OOM cap. kill_on_drop(true) makes dropping the
future SIGKILL the child, so permit lifetime again covers subprocess
lifetime. Documented rather than tested: asserting it requires real
process-table inspection (see spec 2026-06-10-ytdlp-invocation-helper).

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: Write the failing tests for the new helper seams

**Files:**
- Modify: `src/media/resolver.rs` (`#[cfg(test)] mod tests` at the bottom)

- [ ] **Step 1: Add three tests to `mod tests`**

Add after the existing `run_under_cap_*` tests:

```rust
    #[tokio::test]
    async fn yt_dlp_output_rejects_non_http_scheme() {
        let err = yt_dlp_output(
            &["--print", "title", "--no-playlist"],
            "ftp://example.com/video",
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, YtDlpError::InvalidScheme));
    }

    #[tokio::test]
    async fn yt_dlp_print_maps_invalid_scheme_to_existing_message() {
        let err = yt_dlp_print("title", "ftp://example.com/video")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid URL scheme"));
    }

    #[tokio::test]
    async fn probe_live_non_http_is_unknown() {
        assert_eq!(probe_live("not-a-url").await, LiveStatus::Unknown);
    }
```

Note: the first two are the red tests (they reference symbols that don't exist yet). The third passes already — it pins existing behavior so Task 4's rewrite of `probe_live` can't regress it.

- [ ] **Step 2: Run tests to verify the red state**

Run: `cargo test --lib media::resolver 2>&1 | head -20`
Expected: compile FAILURE with E0425 `cannot find function yt_dlp_output` (and `yt_dlp_print`).

---

### Task 3: Implement `YtDlpError`, `yt_dlp_output`, `yt_dlp_anyhow`, `yt_dlp_print`

**Files:**
- Modify: `src/media/resolver.rs` (insert between `run_under_cap` and the `LiveStatus` enum)

- [ ] **Step 1: Add the error enum and layer-1 helper**

```rust
/// Why a yt-dlp invocation produced no usable `Output`.
#[derive(Debug)]
enum YtDlpError {
    InvalidScheme,
    /// No permit free within the wait — load-shed, not queued.
    Busy,
    /// Permit held, but the command exceeded its timeout.
    Timeout,
    Spawn(std::io::Error),
}

/// Single entry point for spawning yt-dlp. Owns the invariants every caller
/// must uphold: the URL scheme check, the global concurrency cap
/// (`run_under_cap`), the command timeout, `kill_on_drop` (a timed-out or
/// cancelled invocation must not leave an orphaned ~73 MB process behind),
/// and the `--` argument guard. A non-zero exit is `Ok` — callers inspect
/// `status`/`stderr` (`probe_live` reads stderr of failed runs).
async fn yt_dlp_output(
    args: &[&str],
    url: &str,
    wait: Duration,
    cmd_timeout: Duration,
) -> Result<std::process::Output, YtDlpError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(YtDlpError::InvalidScheme);
    }
    run_under_cap(yt_dlp_semaphore(), wait, || {
        tokio::time::timeout(
            cmd_timeout,
            Command::new("yt-dlp")
                .kill_on_drop(true)
                .args(args)
                .args(["--", url])
                .output(),
        )
    })
    .await
    .ok_or(YtDlpError::Busy)?
    .map_err(|_| YtDlpError::Timeout)?
    .map_err(YtDlpError::Spawn)
}
```

(Type walk-through for the chained combinators: `run_under_cap(...).await` is `Option<Result<io::Result<Output>, Elapsed>>`; `.ok_or(Busy)?` unwraps the Option, `.map_err(Timeout)?` unwraps the timeout layer, `.map_err(Spawn)` converts the io layer.)

- [ ] **Step 2: Add the error-mapping helper and layer 2**

Immediately after `yt_dlp_output`:

```rust
/// Maps a `YtDlpError` to the error strings the admin UI already shows.
fn yt_dlp_anyhow(err: YtDlpError, url: &str) -> anyhow::Error {
    match err {
        YtDlpError::InvalidScheme => anyhow::anyhow!("invalid URL scheme: {}", url),
        YtDlpError::Busy => {
            anyhow::anyhow!("yt-dlp resolver busy (no free slot) for {}", url)
        }
        YtDlpError::Timeout => anyhow::anyhow!("yt-dlp timed out after 30s for {}", url),
        YtDlpError::Spawn(e) => e.into(),
    }
}

/// Runs `yt-dlp --print <field>` under the cap and returns trimmed,
/// non-empty stdout. Shared body of `fetch_title`, `fetch_video_id`,
/// and `fetch_duration_secs`.
async fn yt_dlp_print(field: &str, url: &str) -> Result<String> {
    let output = yt_dlp_output(
        &["--print", field, "--no-playlist"],
        url,
        Duration::from_secs(15),
        Duration::from_secs(30),
    )
    .await
    .map_err(|e| yt_dlp_anyhow(e, url))?;
    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        bail!("yt-dlp returned empty output for {}", url);
    }
    Ok(value)
}
```

- [ ] **Step 3: Run the new tests to verify green**

Run: `cargo test --lib media::resolver`
Expected: PASS — all resolver tests including the three new ones. (`cargo clippy` without `--all-targets` may warn about dead code here since the helpers are only used by tests until Task 4; that is expected mid-commit and resolves in Task 4.)

---

### Task 4: Rewrite the five callers on the helpers, verify, commit (Commit 2)

**Files:**
- Modify: `src/media/resolver.rs` (functions `probe_live`, `resolve_url`, `fetch_title`, `fetch_video_id`, `fetch_duration_secs`)
- Modify: `CLAUDE.md` (test-count line)

- [ ] **Step 1: Rewrite `probe_live`**

Replace the entire body (keep the existing doc comment):

```rust
pub async fn probe_live(url: &str) -> LiveStatus {
    match yt_dlp_output(
        &["--print", "is_live", "--no-playlist"],
        url,
        Duration::from_secs(8),
        Duration::from_secs(8),
    )
    .await
    {
        Ok(output) => interpret_is_live(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Err(_) => LiveStatus::Unknown,
    }
}
```

- [ ] **Step 2: Rewrite `resolve_url`**

Keep the doc comment, the leading scheme check, and the passthrough — the scheme check MUST stay before `needs_resolution` (today an `ftp://` URL bails even when no resolution is needed; layer 1's check alone would let it pass through):

```rust
pub async fn resolve_url(url: &str) -> Result<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    if !needs_resolution(url) {
        return Ok(url.to_string());
    }
    let output = yt_dlp_output(
        &["-g", "--no-playlist", "-f", "b[ext=mp4]/b"],
        url,
        Duration::from_secs(15),
        Duration::from_secs(30),
    )
    .await
    .map_err(|e| yt_dlp_anyhow(e, url))?;
    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let resolved = String::from_utf8_lossy(&output.stdout).into_owned();
    let first_line = resolved.lines().next().unwrap_or("").trim().to_string();
    if first_line.is_empty() {
        bail!("yt-dlp returned empty output for {}", url);
    }
    Ok(first_line)
}
```

- [ ] **Step 3: Rewrite the three fetchers**

Keep each doc comment; replace the bodies:

```rust
pub async fn fetch_title(url: &str) -> Result<String> {
    yt_dlp_print("title", url).await
}
```

```rust
pub async fn fetch_video_id(url: &str) -> Result<String> {
    yt_dlp_print("id", url).await
}
```

```rust
pub async fn fetch_duration_secs(url: &str) -> Result<i64> {
    let raw = yt_dlp_print("duration", url).await?;
    let duration: f64 = raw
        .parse()
        .map_err(|_| anyhow::anyhow!("could not parse yt-dlp duration: {:?}", raw))?;
    if !duration.is_finite() || duration < 0.0 {
        bail!("yt-dlp returned invalid duration: {}", duration);
    }
    Ok(duration.round() as i64)
}
```

Known benign message changes (recorded in the spec, nothing parses these):
`"empty title"`/`"empty id"` → `"empty output"`; empty duration now reports
`"empty output"` instead of a parse error.

- [ ] **Step 4: Full verification**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: clippy clean (helpers now have non-test callers); 316 tests pass (246 unit: 242 passed + 4 ignored; 70 integration: 69 passed + 1 ignored).

- [ ] **Step 5: Update the test count in CLAUDE.md**

In `CLAUDE.md`, change:

```
cargo test             # 313 tests: 243 unit + 70 integration (5 ignored — need yt-dlp/network)
```

to:

```
cargo test             # 316 tests: 246 unit + 70 integration (5 ignored — need yt-dlp/network)
```

- [ ] **Step 6: Commit**

```bash
git add src/media/resolver.rs CLAUDE.md
git commit -m "refactor: fold five yt-dlp invocation bodies into capped helpers

yt_dlp_output owns the invariants once — scheme check, concurrency cap,
command timeout, kill_on_drop, and the -- argument guard; yt_dlp_print
serves the three --print fetchers. Public signatures and error strings
unchanged (self-review U8; spec 2026-06-10-ytdlp-invocation-helper).

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Manual verification (optional, needs yt-dlp + network)

The five ignored tests remain the end-to-end check:

Run: `cargo test -- --ignored`
Expected: 5 pass on a machine with `yt-dlp` installed and network access.

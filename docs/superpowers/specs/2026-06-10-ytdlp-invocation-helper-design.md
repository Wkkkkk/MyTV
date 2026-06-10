# yt-dlp invocation helper (U8) — design

Date: 2026-06-10
Status: approved approach, pending spec review

## Problem

`src/media/resolver.rs` invokes yt-dlp at five call sites: `resolve_url`,
`fetch_title`, `fetch_video_id`, `fetch_duration_secs`, and `probe_live`.
Each repeats the same ~20-line scaffold: URL scheme check, `run_under_cap`
(permit wait + command timeout), `Command::new("yt-dlp")` with a `--` guard,
and the busy/timeout/non-zero-exit error mapping.

Two problems:

1. **DRY** (self-review U8): the scaffold differs only in yt-dlp arguments
   and output parsing, yet is copied five times.
2. **Correctness**: tokio 1.52's `Command::output()` does **not** set
   `kill_on_drop` (verified in vendored source, `process/mod.rs:1065`;
   default is `false`). When the command timeout fires, the output future is
   dropped and the semaphore permit is released — but the yt-dlp process
   survives, orphaned, still holding ~73 MB until it exits on its own. The
   invariant the 2-permit cap is meant to enforce ("permit lifetime covers
   subprocess lifetime") breaks on exactly the timeout path, in all five
   copies. A few hung processes reopen the OOM class fixed in
   `docs/bug-logs/2026-06-10-live-status-badge-ytdlp-oom.md`.

## Design

Two layers inside `src/media/resolver.rs`. No public API changes; the five
existing `pub` functions keep their signatures and behavior.

### Layer 1 — `yt_dlp_output`: the one place the cap invariant lives

```rust
enum YtDlpError {
    InvalidScheme,
    Busy,            // no permit within `wait` (load-shed)
    Timeout,         // permit held, command exceeded `cmd_timeout`
    Spawn(std::io::Error),
}

async fn yt_dlp_output(
    args: &[&str],
    url: &str,
    wait: Duration,
    cmd_timeout: Duration,
) -> Result<std::process::Output, YtDlpError>
```

Owns, exactly once:
- the `http://`/`https://` scheme check → `InvalidScheme`
- `run_under_cap(yt_dlp_semaphore(), wait, ...)` → `Busy` on shed
- `tokio::time::timeout(cmd_timeout, ...)` → `Timeout`
- `Command::new("yt-dlp").kill_on_drop(true).args(args).args(["--", url])`
  — `kill_on_drop(true)` restores the invariant: dropping the output future
  (timeout or caller cancellation) SIGKILLs the child before the permit frees
- the trailing `--` argument-injection guard

A non-zero exit status is `Ok(output)`, **not** an error: `probe_live` must
inspect stderr of failed runs ("not currently live" → `Offline`).

### Layer 2 — `yt_dlp_print`: the three `--print` fetchers

```rust
async fn yt_dlp_print(field: &str, url: &str) -> anyhow::Result<String>
```

Calls `yt_dlp_output(&["--print", field, "--no-playlist"], url, 15s, 30s)`,
then maps to the existing error strings (preserved verbatim for log
continuity):
- `InvalidScheme` → `"invalid URL scheme: {url}"`
- `Busy` → `"yt-dlp resolver busy (no free slot) for {url}"`
- `Timeout` → `"yt-dlp timed out after 30s for {url}"`
- `Spawn(e)` → propagate `e`
- non-zero exit → `"yt-dlp failed for {url}: {stderr}"`
- empty trimmed stdout → `"yt-dlp returned empty output for {url}"`

### Call-site mapping

| Caller | Uses | Keeps locally |
|---|---|---|
| `fetch_title` | `yt_dlp_print("title", url)` | nothing |
| `fetch_video_id` | `yt_dlp_print("id", url)` | nothing |
| `fetch_duration_secs` | `yt_dlp_print("duration", url)` | f64 parse + finite/≥0 check |
| `resolve_url` | layer 1, args `["-g", "--no-playlist", "-f", "b[ext=mp4]/b"]`, 15s/30s | its own scheme check **before** the `needs_resolution` passthrough (today an `ftp://` URL bails even when no resolution is needed — that ordering must survive; layer 1's check is then a harmless double-check); first-line extraction; non-empty check |
| `probe_live` | layer 1, args `["--print", "is_live", "--no-playlist"]`, 8s/8s | maps any `YtDlpError` → `Unknown`; `Ok(output)` → `interpret_is_live` |

`run_under_cap` itself is unchanged; it remains separately unit-tested.

### Behavior changes

Only one, and it is the point: timed-out or cancelled yt-dlp processes are
now killed instead of orphaned. Empty-output messages for title/id collapse
from `"empty title"`/`"empty id"` to `"empty output"` — acceptable; nothing
parses these strings.

## Sequencing — two commits

1. **Bugfix**: add `.kill_on_drop(true)` at the five existing call sites.
   Minimal, deployable, cherry-pickable on its own. Documented in the commit
   message rather than tested — testing it means spawning real processes and
   inspecting the process table, which costs flakiness and buys little.
2. **Refactor**: introduce `YtDlpError`, `yt_dlp_output`, `yt_dlp_print`;
   rewrite the five callers. Behavior-preserving; the existing 313 tests
   (incl. 4 `run_under_cap` tests, scheme/passthrough tests, badge endpoint
   tests) are the safety net and must pass unchanged.

## Testing

- No existing test changes expected; all 313 must pass as-is.
- Add unit tests for the new seams that don't need yt-dlp:
  - `yt_dlp_output` with a non-HTTP URL → `InvalidScheme` (no spawn attempt)
  - `probe_live` on a non-HTTP URL still → `Unknown` (existing behavior,
    now routed through layer 1)
- The five ignored network tests remain the manual end-to-end check.

# Ended-Live → VOD Conversion via live_status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert live channels whose YouTube stream has ended (`was_live`/`post_live`) to `vod_loop` at tune time, using the same single yt-dlp call that resolves the URL.

**Architecture:** `resolve_url`'s yt-dlp invocation gains `--print live_status` ahead of `--print urls` (the `-g` alias), so one subprocess returns status + URL. A new `resolve_url_with_status` owns invocation + parsing; `resolve_url` becomes a thin wrapper so the three status-indifferent call sites are untouched. `next_live` calls a pure `is_ended_live(status, url)` helper — `WasLive | PostLive` or the existing `force_finished/1` manifest fallback — feeding the unchanged idea-#36 conversion machinery. Spec: `docs/superpowers/specs/2026-06-11-ended-live-conversion-via-live-status-design.md`.

**Tech Stack:** Rust 1.96, Axum 0.7, yt-dlp subprocess (2-permit semaphore via `yt_dlp_output`), tokio.

**Conventions:** Run `cargo fmt` before EVERY commit (CI fails on any diff). `cargo clippy --all-targets -- -D warnings` must stay clean. No comments unless the WHY is non-obvious.

**Pinned behavior that must NOT change:** error strings `"invalid URL scheme: {url}"` (checked BEFORE the `needs_resolution` passthrough — see test `resolve_url_rejects_non_http_scheme_before_passthrough`) and `"yt-dlp returned empty output for {url}"` (existing tests pin these).

---

### Task 1: Extract shared `live_status_from_str`

**Files:**
- Modify: `src/media/resolver.rs` — new function, `interpret_live_status` delegates; tests in existing `mod tests`

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/media/resolver.rs`:

```rust
    #[test]
    fn live_status_from_str_maps_tokens() {
        use LiveStatus::*;
        assert_eq!(live_status_from_str("is_live"), Live);
        assert_eq!(live_status_from_str("is_upcoming"), Upcoming(None));
        assert_eq!(live_status_from_str("post_live"), PostLive);
        assert_eq!(live_status_from_str("was_live"), WasLive);
        assert_eq!(live_status_from_str("not_live"), NotLive);
        assert_eq!(live_status_from_str("NA"), Unknown);
        assert_eq!(live_status_from_str("None"), Unknown);
        assert_eq!(live_status_from_str(""), Unknown);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib live_status_from_str`
Expected: COMPILE ERROR — `cannot find function live_status_from_str`

- [ ] **Step 3: Implement and delegate**

In `src/media/resolver.rs`, directly above `interpret_live_status`, add:

```rust
/// Maps a yt-dlp `live_status` token to a `LiveStatus`. Carries no timestamp —
/// `is_upcoming` maps to `Upcoming(None)`; callers that also have a
/// `release_timestamp` attach it themselves.
pub fn live_status_from_str(token: &str) -> LiveStatus {
    match token {
        "is_live" => LiveStatus::Live,
        "is_upcoming" => LiveStatus::Upcoming(None),
        "post_live" => LiveStatus::PostLive,
        "was_live" => LiveStatus::WasLive,
        "not_live" => LiveStatus::NotLive,
        _ => LiveStatus::Unknown,
    }
}
```

Then replace the success branch of `interpret_live_status` (currently a `match status { ... }` over the same tokens) with delegation:

```rust
    if success {
        let out = stdout.lines().next().unwrap_or("").trim();
        let (status, ts) = out.split_once('|').unwrap_or((out, "NA"));
        return match live_status_from_str(status) {
            LiveStatus::Upcoming(_) => LiveStatus::Upcoming(ts.parse::<i64>().ok()),
            other => other,
        };
    }
```

(The stderr-fallback half of `interpret_live_status` is unchanged.)

- [ ] **Step 4: Run tests to verify they pass — including the regression table**

Run: `cargo test --lib resolver`
Expected: PASS, including the UNCHANGED `interpret_live_status_maps_all_cases` (this is the regression check on the delegation refactor — do not modify that test).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/media/resolver.rs
git commit -m "refactor: extract shared live_status token mapping"
```

The commit message must end with:
`Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`

---

### Task 2: `resolve_url_with_status`

**Files:**
- Modify: `src/media/resolver.rs` — new parser + public function, `resolve_url` becomes wrapper; tests in `mod tests`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/media/resolver.rs`:

```rust
    #[test]
    fn parse_status_and_url_two_lines() {
        let (url, status) =
            parse_status_and_url("was_live\nhttps://example.com/v.mp4\n").unwrap();
        assert_eq!(url, "https://example.com/v.mp4");
        assert_eq!(status, LiveStatus::WasLive);
    }

    #[test]
    fn parse_status_and_url_three_lines_takes_first_url() {
        let (url, status) =
            parse_status_and_url("is_live\nhttps://a.test/video\nhttps://a.test/audio\n")
                .unwrap();
        assert_eq!(url, "https://a.test/video");
        assert_eq!(status, LiveStatus::Live);
    }

    #[test]
    fn parse_status_and_url_na_status_is_unknown() {
        let (url, status) = parse_status_and_url("NA\nhttps://a.test/v.m3u8\n").unwrap();
        assert_eq!(status, LiveStatus::Unknown);
        assert_eq!(url, "https://a.test/v.m3u8");
    }

    #[test]
    fn parse_status_and_url_missing_url_line_is_none() {
        assert_eq!(parse_status_and_url("was_live\n"), None);
        assert_eq!(parse_status_and_url(""), None);
    }

    #[tokio::test]
    async fn resolve_url_with_status_passthrough_for_hls() {
        let (url, status) = resolve_url_with_status("https://example.com/stream.m3u8")
            .await
            .unwrap();
        assert_eq!(url, "https://example.com/stream.m3u8");
        assert_eq!(status, LiveStatus::Unknown);
    }

    #[tokio::test]
    async fn resolve_url_with_status_rejects_non_http_scheme() {
        let err = resolve_url_with_status("ftp://example.com/stream.m3u8")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid URL scheme"));
    }

    #[tokio::test]
    #[ignore = "requires yt-dlp and network"]
    async fn resolve_url_with_status_real_vod_is_not_live() {
        // "Me at the zoo" — pins the two-line `--print live_status --print urls`
        // output shape and print ordering against real yt-dlp.
        let (url, status) =
            resolve_url_with_status("https://www.youtube.com/watch?v=jNQXAC9IVRw")
                .await
                .unwrap();
        assert!(url.starts_with("http"), "got: {url}");
        assert_eq!(status, LiveStatus::NotLive);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib resolve_url_with_status`
Expected: COMPILE ERROR — `cannot find function parse_status_and_url` / `resolve_url_with_status`

- [ ] **Step 3: Implement**

In `src/media/resolver.rs`, REPLACE the whole existing `resolve_url` function (currently `yt-dlp -g --no-playlist -f b[ext=mp4]/b`, doc comment included) with:

```rust
/// Parses `--print live_status --print urls` stdout: line 1 is the status
/// token, line 2 the first playable URL (later lines are additional formats,
/// e.g. separate audio — the first-URL rule matches the old `-g` behavior).
fn parse_status_and_url(stdout: &str) -> Option<(String, LiveStatus)> {
    let mut lines = stdout.lines();
    let status = live_status_from_str(lines.next().unwrap_or("").trim());
    let url = lines.next().unwrap_or("").trim();
    if url.is_empty() {
        return None;
    }
    Some((url.to_string(), status))
}

/// Returns a directly playable URL plus the stream's lifecycle state.
/// HLS/IPTV URLs are returned unchanged (status Unknown, no yt-dlp spawn).
/// YouTube/Twitch are resolved via a single yt-dlp call that also reports
/// `live_status` — `next_live` uses it to detect ended broadcasts.
pub async fn resolve_url_with_status(url: &str) -> Result<(String, LiveStatus)> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("invalid URL scheme: {}", url);
    }
    if !needs_resolution(url) {
        return Ok((url.to_string(), LiveStatus::Unknown));
    }
    let output = yt_dlp_output(
        &[
            "--print",
            "live_status",
            "--print",
            "urls",
            "--no-playlist",
            "-f",
            "b[ext=mp4]/b",
        ],
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
    parse_status_and_url(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| anyhow::anyhow!("yt-dlp returned empty output for {}", url))
}

/// Returns a directly playable URL.
/// HLS/IPTV URLs are returned unchanged. YouTube/Twitch are resolved via yt-dlp.
pub async fn resolve_url(url: &str) -> Result<String> {
    Ok(resolve_url_with_status(url).await?.0)
}
```

Note the preserved pinned error strings: `"invalid URL scheme: {url}"` (still checked BEFORE the `needs_resolution` passthrough) and `"yt-dlp returned empty output for {url}"`.

- [ ] **Step 4: Run the full suite**

Run: `cargo test`
Expected: ALL pass — in particular the pre-existing `resolve_url_rejects_non_http_scheme_before_passthrough`, `test_resolve_url_passthrough_for_hls`, `test_resolve_url_passthrough_for_plain_iptv` (all exercise the wrapper unchanged), and the new tests from Step 1 except the ignored one.

- [ ] **Step 5: Run the ignored pinning test once (yt-dlp + network)**

Run: `cargo test --lib resolve_url_with_status_real_vod -- --ignored`
Expected: PASS. If it fails, inspect raw output with
`yt-dlp --print live_status --print urls --no-playlist -f "b[ext=mp4]/b" -- "https://www.youtube.com/watch?v=jNQXAC9IVRw"`
and adjust `parse_status_and_url` (NOT the test) to match reality; re-run Steps 4–5.

- [ ] **Step 6: Format and commit**

```bash
cargo fmt
git add src/media/resolver.rs
git commit -m "feat: resolve_url_with_status — one yt-dlp call returns URL + live_status"
```

The commit message must end with:
`Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`

---

### Task 3: `is_ended_live` decision in `next_live`

**Files:**
- Modify: `src/routes/player.rs` — new helper + `next_live` wiring; tests in the file's existing `mod tests`

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/routes/player.rs` (check the module's existing `use` lines; `crate::media::resolver` paths below are fully qualified so no new imports should be needed):

```rust
    #[test]
    fn is_ended_live_decision() {
        use crate::media::resolver::LiveStatus::*;
        assert!(is_ended_live(WasLive, "https://x.test/v.mp4"));
        assert!(is_ended_live(PostLive, "https://x.test/v.m3u8"));
        assert!(!is_ended_live(Live, "https://x.test/v.m3u8"));
        assert!(!is_ended_live(NotLive, "https://x.test/v.mp4"));
        assert!(!is_ended_live(Unknown, "https://x.test/v.m3u8"));
        assert!(is_ended_live(
            Unknown,
            "https://r5---sn.googlevideo.com/a/force_finished/1/b/index.m3u8"
        ));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib is_ended_live`
Expected: COMPILE ERROR — `cannot find function is_ended_live`

- [ ] **Step 3: Implement**

In `src/routes/player.rs`, directly above `next_live`, add:

```rust
/// A live source is "ended" when yt-dlp reports the broadcast finished
/// (was_live: recording processed; post_live: just ended) or, as a fallback
/// for extractors without live_status, when the resolved manifest carries the
/// force_finished marker.
fn is_ended_live(status: resolver::LiveStatus, resolved_url: &str) -> bool {
    matches!(
        status,
        resolver::LiveStatus::WasLive | resolver::LiveStatus::PostLive
    ) || resolver::is_finished_live(resolved_url)
}
```

Then in `next_live`, replace the resolve match arm (currently `match resolver::resolve_url(&src.url).await { Ok(url) => { if resolver::is_finished_live(&url) { ... } ... } }`) with:

```rust
        match resolver::resolve_url_with_status(&src.url).await {
            Ok((url, status)) => {
                if is_ended_live(status, &url) {
                    spawn_live_to_vod_conversion(state, ch.id, ch.name.clone(), src.url.clone());
                    return Ok(tune_response_ended(ch));
                }
                return Ok(tune_response(
                    ch,
                    url,
                    0,
                    resolver::needs_resolution(&src.url),
                ));
            }
            Err(e) => {
                // Idea #38 seam: Upcoming/Offline sources fail resolution and land here.
                tracing::warn!(url = %src.url, error = %e, "resolver failed, trying next source")
            }
        }
```

Everything else in `next_live` (the `failed_url` filter, the trailing 503) is unchanged.

- [ ] **Step 4: Run the full suite**

Run: `cargo test`
Expected: ALL pass. Watch specifically:
- `test_tune_finished_live_returns_ended_and_no_url` (tests/http.rs) — its seed source is a direct HLS URL carrying `force_finished/1`, so it flows through the passthrough (`status = Unknown`) into the `is_finished_live` fallback. If this fails, the fallback wiring is wrong — fix the code, not the test.
- `test_tune_live_skips_youtube_source_when_ytdlp_unavailable_and_returns_hls_backup` — the Err branch is unchanged; must still pass.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "feat: convert ended live channels to vod_loop on was_live/post_live (idea #39)"
```

The commit message must end with:
`Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`

---

### Task 4: Docs + final verification

**Files:**
- Modify: `docs/IDEAS.md` (item #39)
- Modify: `docs/architecture/tune-flow.md`

- [ ] **Step 1: Mark idea #39 done**

In `docs/IDEAS.md`, replace the line

```markdown
39. **Reopen idea36** - we have a working ended-live-to-VOD flow, but we need it to be compatible with end-live-to-live (idea38)
```

with

```markdown
39. ~~**Reopen idea36**~~ — done: `next_live` resolves via `resolve_url_with_status` (`--print live_status --print urls`, still one yt-dlp call) and converts on `was_live`/`post_live` in addition to the `force_finished/1` manifest fallback, so fully-processed recordings flip to `vod_loop` on first tune. `Upcoming`/`Offline` still fail resolution and fall through to failover/503 — that error branch is the seam for idea #38. Spec: `docs/superpowers/specs/2026-06-11-ended-live-conversion-via-live-status-design.md`.
```

Check `git diff docs/IDEAS.md` afterwards — it should contain ONLY this hunk. If unrelated pending edits exist, stage only this one and surface the rest.

- [ ] **Step 2: Update the tune-flow architecture doc**

Read `docs/architecture/tune-flow.md` and make these minimal edits (do not restructure the doc):

- Lines 18 and 63 (Mermaid nodes): `resolver::resolve_url(src.url)` / `resolver::resolve_url` → `resolver::resolve_url_with_status(src.url)` / `resolver::resolve_url_with_status`. Leave the `res2` VOD nodes (lines 31, 73) as `resolve_url` — those call sites are unchanged.
- Lines 19 and 64 (Mermaid decision nodes): `is_finished_live?` → `is_ended_live?`.
- The prose at line 80 starting `When `resolve_url` succeeds but `resolver::is_finished_live` detects a `force_finished/1` manifest…` — rewrite the opening to: resolution returns the URL plus yt-dlp's `live_status`; the handler treats the broadcast as ended when the status is `was_live` (recording processed) or `post_live` (just ended), or — as a fallback for extractors without `live_status` — when `resolver::is_finished_live` detects a `force_finished/1` manifest. The rest of that paragraph (what the handler does instead) is unchanged.

- [ ] **Step 3: Full verification**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: clean, clean, all pass (target: 259+ unit + 71 integration, 7 ignored — Tasks 1–3 added 8 fast tests and 1 ignored).

- [ ] **Step 4: Commit**

```bash
git add docs/IDEAS.md docs/architecture/tune-flow.md
git commit -m "docs: mark idea #39 done; tune-flow reflects live_status ended detection"
```

The commit message must end with:
`Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`

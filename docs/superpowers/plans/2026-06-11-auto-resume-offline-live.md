# Auto-resume Offline Live Channels Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a YouTube/Twitch live source is offline or upcoming at tune time, show a "Waiting for stream…" state that auto-retries with backoff and resumes playback when the stream returns, instead of a hard 503.

**Architecture:** Surface live status through the existing tune path (Approach A). `resolve_url_with_status` returns an empty URL + `Offline`/`Upcoming` status instead of erroring; `next_live` gains a `waiting` outcome (HTTP 200, mirroring `ended`); each probe feeds source health; the 15-min background health checker becomes liveness-aware for `youtube_live` so the offline badge reflects real liveness. The client polls `/tune` on a `[15,30,60,120]`s backoff, then settles into an offline state.

**Tech Stack:** Rust/Axum, SQLx (SQLite), Askama + vanilla JS frontend, yt-dlp via `resolver`.

**Spec:** `docs/superpowers/specs/2026-06-11-auto-resume-offline-live-design.md`

---

## File Structure

- `src/media/resolver.rs` — new `recoverable_status` pure fn; failure branch of `resolve_url_with_status` returns offline/upcoming; `resolve_url` guards empty URL.
- `src/health.rs` — new `record_source_liveness` helper; new `live_status_health` pure mapping fn; `do_http_check` becomes liveness-aware for `youtube_live`; `LiveStatusCache` threaded through `run_check`/`check_source`/`probe_source`; `HealthClients` gains `live_cache`.
- `src/main.rs` — pass `live_cache` into `HealthClients`.
- `src/routes/admin/sources.rs` — pass `live_cache` into `probe_source`.
- `src/routes/player.rs` — `TuneResponse.waiting` field + `tune_response_waiting`; `LiveOutcome` enum + `classify_live_outcome`; rewritten `next_live`.
- `templates/base.html`, `templates/guide.html` — waiting overlay markup, CSS, backoff JS.
- `docs/ideas.md`, `CLAUDE.md` — mark idea #38 done; update test count.

---

## Task 1: Resolver surfaces offline/upcoming

**Files:**
- Modify: `src/media/resolver.rs` (failure branch ~285-291; `resolve_url` ~298-300)
- Test: `src/media/resolver.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/media/resolver.rs`:

```rust
#[test]
fn test_recoverable_status_offline_and_upcoming() {
    use LiveStatus::*;
    assert_eq!(
        recoverable_status("ERROR: ... This live event is not currently live ..."),
        Some(Offline)
    );
    assert_eq!(
        recoverable_status("ERROR: ... this live event will begin in 2 hours ..."),
        Some(Upcoming(None))
    );
    // genuine failures do not become a recoverable status
    assert_eq!(recoverable_status("ERROR: HTTP Error 404: Not Found"), None);
    assert_eq!(recoverable_status(""), None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib recoverable_status`
Expected: FAIL — `cannot find function 'recoverable_status'`.

- [ ] **Step 3: Add the `recoverable_status` fn**

Insert just below `interpret_live_status` (after line ~164) in `src/media/resolver.rs`:

```rust
/// Classifies a failed yt-dlp resolve. Returns `Some(status)` when the failure
/// is a recoverable broadcast state the player should wait on (`Offline` /
/// `Upcoming`), or `None` when it is a genuine error that should propagate.
pub fn recoverable_status(stderr: &str) -> Option<LiveStatus> {
    match interpret_live_status(false, "", stderr) {
        s @ (LiveStatus::Offline | LiveStatus::Upcoming(_)) => Some(s),
        _ => None,
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib recoverable_status`
Expected: PASS.

- [ ] **Step 5: Wire it into `resolve_url_with_status`**

Replace the failure branch in `resolve_url_with_status` (currently):

```rust
    if !output.status.success() {
        bail!(
            "yt-dlp failed for {}: {}",
            url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
```

with:

```rust
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if let Some(status) = recoverable_status(&stderr) {
            return Ok((String::new(), status));
        }
        bail!("yt-dlp failed for {}: {}", url, stderr.trim());
    }
```

- [ ] **Step 6: Guard `resolve_url` against the new empty-URL case**

Replace `resolve_url` (currently `Ok(resolve_url_with_status(url).await?.0)`) with:

```rust
pub async fn resolve_url(url: &str) -> Result<String> {
    let (resolved, _) = resolve_url_with_status(url).await?;
    if resolved.is_empty() {
        bail!("no playable URL for {url} (stream offline or upcoming)");
    }
    Ok(resolved)
}
```

- [ ] **Step 7: Verify build + existing resolver tests still pass**

Run: `cargo test --lib media::resolver`
Expected: PASS (all existing resolver tests + the new one).

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/media/resolver.rs
git commit -m "feat(resolver): surface offline/upcoming instead of erroring"
```

---

## Task 2: `record_source_liveness` health helper

**Files:**
- Modify: `src/health.rs` (add helper + import + tests)
- Test: `src/health.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/health.rs`:

```rust
#[tokio::test]
async fn test_record_source_liveness_disables_then_reenables() {
    use crate::model::{channel, source};
    let pool = crate::db::connect("sqlite::memory:").await.unwrap();
    let ch = channel::create(
        &pool,
        channel::NewChannel {
            name: "T".into(),
            category: "t".into(),
            logo_url: None,
            channel_type: channel::ChannelType::Live,
            sort_order: 0,
            loop_anchor: None,
        },
    )
    .await
    .unwrap();
    let mut src = source::create(
        &pool,
        source::NewSource {
            channel_id: ch.id,
            kind: source::SourceKind::YoutubeLive,
            url: "https://youtube.com/watch?v=x".into(),
            priority: 1,
        },
    )
    .await
    .unwrap();

    for _ in 0..FAILURE_THRESHOLD {
        record_source_liveness(&pool, &src, false).await;
        src = source::get(&pool, src.id).await.unwrap().unwrap();
    }
    assert!(!src.is_active, "disabled after threshold offline probes");
    assert_eq!(src.consecutive_failures, FAILURE_THRESHOLD);

    record_source_liveness(&pool, &src, true).await;
    let after = source::get(&pool, src.id).await.unwrap().unwrap();
    assert!(after.is_active, "re-enabled when live again");
    assert_eq!(after.consecutive_failures, 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib test_record_source_liveness`
Expected: FAIL — `cannot find function 'record_source_liveness'`.

- [ ] **Step 3: Implement the helper**

Add to `src/health.rs` (after `run_check`, before `probe_source`):

```rust
/// Records a single liveness probe result against a source's health, reusing
/// the same disable/re-enable lifecycle as the background checker. `ok = true`
/// means the stream is playable (resets failures, re-enables); `ok = false`
/// means offline/ended (counts toward the auto-disable threshold). Used by the
/// interactive tune path so an active poll doubles as a liveness signal.
pub async fn record_source_liveness(pool: &SqlitePool, src: &Source, ok: bool) {
    let (new_failures, action) = process_result(src.is_active, src.consecutive_failures, ok);
    let is_active_change = match action {
        HealthAction::Disable => Some(false),
        HealthAction::Reenable => Some(true),
        HealthAction::None => None,
    };
    let status = if ok { "ok" } else { "error" };
    let reason = if ok { None } else { Some("not currently live") };
    if let Err(e) =
        source::update_health(pool, src.id, status, reason, new_failures, is_active_change).await
    {
        tracing::error!("health: failed to record liveness for {}: {e}", src.url);
        return;
    }
    match action {
        HealthAction::Disable => {
            tracing::warn!("health: {} auto-disabled after {new_failures} offline probes", src.url)
        }
        HealthAction::Reenable => tracing::info!("health: {} re-enabled (live again)", src.url),
        HealthAction::None => {}
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib test_record_source_liveness`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/health.rs
git commit -m "feat(health): record_source_liveness helper for tune-time probes"
```

---

## Task 3: `TuneResponse.waiting` field + builder

**Files:**
- Modify: `src/routes/player.rs` (`TuneResponse` ~20-30; builders ~74-103)

- [ ] **Step 1: Add the `waiting` field**

In `TuneResponse` (after `pub ended: bool,`) add:

```rust
    pub waiting: bool,
```

- [ ] **Step 2: Update the two existing builders**

In `tune_response`, add `waiting: false,` after `ended: false,`.
In `tune_response_ended`, add `waiting: false,` after `ended: true,`.

- [ ] **Step 3: Add the waiting builder**

Add after `tune_response_ended`:

```rust
fn tune_response_waiting(ch: &channel::Channel) -> Json<TuneResponse> {
    Json(TuneResponse {
        url: String::new(),
        start_offset_secs: 0,
        name: ch.name.clone(),
        logo_url: ch.logo_url.clone(),
        category: ch.category.clone(),
        channel_type: ch.r#type.clone(),
        skip_proxy: false,
        ended: false,
        waiting: true,
    })
}
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build`
Expected: compiles (a `tune_response_waiting` is never used warning is fine — Task 4 uses it; if `-D warnings` blocks the build, proceed directly to Task 4 before running clippy).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "feat(player): add waiting flag to TuneResponse"
```

---

## Task 4: `classify_live_outcome` + `next_live` waiting outcome

**Files:**
- Modify: `src/routes/player.rs` (`next_live` ~116-149; add enum/fn near `is_ended_live` ~105-114)
- Test: `src/routes/player.rs` (tests module, near `is_ended_live_decision` ~909)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/routes/player.rs`:

```rust
#[test]
fn classify_live_outcome_decision() {
    use crate::media::resolver::LiveStatus::*;
    assert!(matches!(
        classify_live_outcome("https://x/v.m3u8", Live),
        Some(LiveOutcome::Play)
    ));
    assert!(matches!(
        classify_live_outcome("https://x/v.m3u8", Unknown),
        Some(LiveOutcome::Play)
    ));
    assert!(matches!(
        classify_live_outcome("https://x/v.mp4", WasLive),
        Some(LiveOutcome::Ended)
    ));
    assert!(matches!(
        classify_live_outcome("https://x/a/force_finished/1/i.m3u8", Unknown),
        Some(LiveOutcome::Ended)
    ));
    assert!(matches!(
        classify_live_outcome("", Offline),
        Some(LiveOutcome::Waiting)
    ));
    assert!(matches!(
        classify_live_outcome("", Upcoming(None)),
        Some(LiveOutcome::Waiting)
    ));
    assert!(matches!(
        classify_live_outcome("", Upcoming(Some(1234))),
        Some(LiveOutcome::Waiting)
    ));
    assert!(classify_live_outcome("", Unknown).is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib classify_live_outcome_decision`
Expected: FAIL — `cannot find ... LiveOutcome` / `classify_live_outcome`.

- [ ] **Step 3: Add the enum + classifier**

Insert directly above `async fn next_live` in `src/routes/player.rs`:

```rust
enum LiveOutcome {
    Play,
    Ended,
    Waiting,
}

/// Maps a resolver result to the action `next_live` takes. `None` means "not
/// usable" (a genuine failure, or an empty URL whose status is not
/// offline/upcoming) — the caller should try the next source.
fn classify_live_outcome(url: &str, status: resolver::LiveStatus) -> Option<LiveOutcome> {
    if is_ended_live(status, url) {
        return Some(LiveOutcome::Ended);
    }
    if url.is_empty() {
        return matches!(
            status,
            resolver::LiveStatus::Offline | resolver::LiveStatus::Upcoming(_)
        )
        .then_some(LiveOutcome::Waiting);
    }
    Some(LiveOutcome::Play)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib classify_live_outcome_decision`
Expected: PASS.

- [ ] **Step 5: Rewrite the `next_live` loop**

Replace the body of `next_live` (the `for` loop + trailing `Err(...)`, lines ~125-148) with:

```rust
    let mut saw_waiting = false;
    for src in sources
        .iter()
        .filter(|s| Some(s.url.as_str()) != failed_url)
    {
        match resolver::resolve_url_with_status(&src.url).await {
            Ok((url, status)) => match classify_live_outcome(&url, status) {
                Some(LiveOutcome::Ended) => {
                    spawn_live_to_vod_conversion(state, ch.id, ch.name.clone(), src.url.clone());
                    return Ok(tune_response_ended(ch));
                }
                Some(LiveOutcome::Play) => {
                    crate::health::record_source_liveness(&state.pool, src, true).await;
                    return Ok(tune_response(ch, url, 0, resolver::needs_resolution(&src.url)));
                }
                Some(LiveOutcome::Waiting) => {
                    saw_waiting = true;
                    if status == resolver::LiveStatus::Offline {
                        crate::health::record_source_liveness(&state.pool, src, false).await;
                    }
                }
                None => {
                    tracing::warn!(url = %src.url, ?status, "resolver returned no usable URL")
                }
            },
            Err(e) => {
                tracing::warn!(url = %src.url, error = %e, "resolver failed, trying next source")
            }
        }
    }

    if saw_waiting {
        Ok(tune_response_waiting(ch))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
```

- [ ] **Step 6: Verify build + full lib tests**

Run: `cargo test --lib routes::player`
Expected: PASS. Then `cargo build` — no `tune_response_waiting`/`LiveOutcome` dead-code warnings.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/routes/player.rs
git commit -m "feat(player): waiting outcome in next_live for offline/upcoming sources"
```

---

## Task 5: Liveness-aware background health checker

**Files:**
- Modify: `src/health.rs` (import, `live_status_health`, `do_http_check`, `run_check`, `check_source`, `probe_source`, `check_playlist_item`/`probe_playlist_item` callers, `check_all`, `HealthClients`, `start`)
- Modify: `src/main.rs` (`HealthClients` construction ~44-48)
- Modify: `src/routes/admin/sources.rs` (`probe_source` call ~96)
- Test: `src/health.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/health.rs`:

```rust
#[test]
fn test_live_status_health_mapping() {
    use crate::media::resolver::LiveStatus::*;
    assert_eq!(live_status_health(Live), (true, None));
    assert_eq!(live_status_health(Upcoming(None)), (true, None));
    assert_eq!(live_status_health(Unknown), (true, None));
    assert_eq!(live_status_health(Offline), (false, Some("not currently live")));
    assert_eq!(live_status_health(NotLive), (false, Some("not currently live")));
    assert_eq!(live_status_health(WasLive), (false, Some("broadcast ended")));
    assert_eq!(live_status_health(PostLive), (false, Some("broadcast ended")));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib test_live_status_health_mapping`
Expected: FAIL — `cannot find function 'live_status_health'`.

- [ ] **Step 3: Add the import + mapping fn**

At the top of `src/health.rs`, add the resolver import next to the existing `use crate::model::source...`:

```rust
use crate::media::resolver::{self, LiveStatus};
```

Add the mapping fn (near `process_result`):

```rust
/// Maps a probed live status to a `(healthy, reason)` health result for a
/// `youtube_live` source. `Upcoming`/`Unknown` never penalize (a scheduled
/// stream isn't broken; `Unknown` is a load-shed or extractor gap).
fn live_status_health(status: LiveStatus) -> (bool, Option<&'static str>) {
    match status {
        LiveStatus::Live | LiveStatus::Upcoming(_) | LiveStatus::Unknown => (true, None),
        LiveStatus::Offline | LiveStatus::NotLive => (false, Some("not currently live")),
        LiveStatus::WasLive | LiveStatus::PostLive => (false, Some("broadcast ended")),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib test_live_status_health_mapping`
Expected: PASS.

- [ ] **Step 5: Thread `LiveStatusCache` into `do_http_check`**

Change the signature and the `youtube_live` branch of `do_http_check`:

```rust
async fn do_http_check(
    client: &reqwest::Client,
    url: &str,
    kind: &str,
    live_cache: Option<&crate::LiveStatusCache>,
) -> (bool, Option<String>) {
```

Replace the existing `youtube_live` early-return:

```rust
    if kind == "youtube_live" {
        return (true, None);
    }
```

with:

```rust
    if kind == "youtube_live" {
        let (ok, reason) = match live_cache {
            Some(c) => live_status_health(resolver::cached_live_status(c, url).await),
            None => (true, None),
        };
        return (ok, reason.map(|s| s.to_string()));
    }
```

- [ ] **Step 6: Thread the cache through `run_check`**

Add the param to `run_check` (after `manage_lifecycle: bool,`):

```rust
    live_cache: Option<&crate::LiveStatusCache>,
```

and update its `do_http_check` call:

```rust
    let (ok, reason) = do_http_check(client, url, kind, live_cache).await;
```

- [ ] **Step 7: Update the four `run_check` callers**

- `check_source`: add a `live_cache: &crate::LiveStatusCache` param to the fn signature, and pass `Some(live_cache)` as the new `run_check` argument (after the `true` manage_lifecycle arg).
- `probe_source`: add a `live_cache: &crate::LiveStatusCache` param to the fn signature, and pass `Some(live_cache)` (after the `false` arg).
- `check_playlist_item`: pass `None` (after the `true` arg) — no signature change.
- `probe_playlist_item`: pass `None` (after the `false` arg) — no signature change.

Final `check_source` signature:

```rust
async fn check_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    live_cache: &crate::LiveStatusCache,
    src: &Source,
) -> bool {
```

Final `probe_source` signature:

```rust
pub async fn probe_source(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    live_cache: &crate::LiveStatusCache,
    src: &Source,
) {
```

- [ ] **Step 8: Thread cache through `check_all`, `HealthClients`, `start`**

Add to `HealthClients`:

```rust
    pub live_cache: crate::LiveStatusCache,
```

In `start`, pass it to `check_all`:

```rust
            check_all(&clients.pool, &clients.http_client, &clients.cors_cache, &clients.live_cache)
                .await;
```

Change `check_all` signature and its `check_source` call:

```rust
async fn check_all(
    pool: &SqlitePool,
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    live_cache: &crate::LiveStatusCache,
) {
```

```rust
        let ok = check_source(pool, client, live_cache, &src).await;
```

- [ ] **Step 9: Update `main.rs` and the admin caller**

In `src/main.rs`, add to the `HealthClients { ... }` literal:

```rust
        live_cache: state.live_cache.clone(),
```

In `src/routes/admin/sources.rs:96`, change the `probe_source` call to:

```rust
    crate::health::probe_source(&state.pool, &state.http_client, &state.cors_cache, &state.live_cache, &src).await;
```

- [ ] **Step 10: Build, lint, and run lib tests**

Run: `cargo build && cargo clippy -- -D warnings && cargo test --lib health`
Expected: compiles clean; all health tests pass.

- [ ] **Step 11: Commit**

```bash
cargo fmt
git add src/health.rs src/main.rs src/routes/admin/sources.rs
git commit -m "feat(health): liveness-aware youtube_live health check"
```

---

## Task 6: Frontend waiting overlay + backoff

**Files:**
- Modify: `templates/base.html` (CSS ~26; JS vars ~128; `enterWaitingState` new; `applyTuneResponse` ~371; `tune` ~378)
- Modify: `templates/guide.html` (player-panel ~3-7)

> This project has no JS test harness; verification is `cargo build` + a manual browser check described in Step 7.

- [ ] **Step 1: Add the overlay markup**

In `templates/guide.html`, inside `#player-panel`, add after the `#player-ended` div:

```html
  <div id="player-waiting">Waiting for stream…</div>
```

- [ ] **Step 2: Add the CSS**

In `templates/base.html`, after the `#player-ended{...}` rule (line ~26):

```css
    #player-waiting{display:none;padding:32px;text-align:center;color:#fff;background:#000;font-size:1rem}
```

- [ ] **Step 3: Add the JS state vars**

In `templates/base.html`, after `var endedAdvanceGen = 0;` (line ~129):

```javascript
      var waitingGen = 0;
      var waitingStep = 0;
      var WAITING_BACKOFF = [15, 30, 60, 120];
```

- [ ] **Step 4: Add `enterWaitingState`**

In `templates/base.html`, add just above `function applyTuneResponse(d) {`:

```javascript
      function enterWaitingState() {
        if (video) video.style.display = 'none';
        var el = document.getElementById('player-waiting');
        if (waitingStep >= WAITING_BACKOFF.length) {
          if (el) { el.textContent = 'Channel offline'; el.style.display = 'block'; }
          return;
        }
        if (el) { el.textContent = 'Waiting for stream…'; el.style.display = 'block'; }
        var delay = WAITING_BACKOFF[waitingStep];
        waitingStep++;
        var gen = ++waitingGen;
        setTimeout(function() {
          if (gen !== waitingGen || !currentChannelId) return;
          fetch('/channel/' + currentChannelId + '/tune')
            .then(function(r) { if (!r.ok) { showPlayerError(); return null; } return r.json(); })
            .then(function(d) { if (gen === waitingGen) applyTuneResponse(d); })
            .catch(function() { showPlayerError(); });
        }, delay * 1000);
      }
```

- [ ] **Step 5: Handle `waiting` in `applyTuneResponse`**

Replace `applyTuneResponse` with:

```javascript
      function applyTuneResponse(d) {
        if (!d) return;
        if (d.waiting) { enterWaitingState(); return; }
        waitingStep = 0;
        waitingGen++;
        var wEl = document.getElementById('player-waiting');
        if (wEl) wEl.style.display = 'none';
        if (video) video.style.display = '';
        if (d.ended) { advanceEndedChannel(); return; }
        endedHops = 0;
        if (d.url) _loadSource(d.url, d.start_offset_secs, d.skip_proxy);
      }
```

- [ ] **Step 6: Cancel waiting on manual tune**

In `function tune(channelId)`, after `endedAdvanceGen++;` (line ~382) add:

```javascript
        waitingGen++;
        waitingStep = 0;
        var waitingNotice = document.getElementById('player-waiting');
        if (waitingNotice) waitingNotice.style.display = 'none';
        if (video) video.style.display = '';
```

- [ ] **Step 7: Build + manual verification**

Run: `cargo build`
Expected: compiles.

Manual check (single command to start, then browser):
Run: `cargo run` and open `http://localhost:3000/guide`. Tune a channel whose YouTube source is offline. Expected: video hides, "Waiting for stream…" appears; the network tab shows `/channel/<id>/tune` re-fetched at ~15s, 30s, 60s, 120s; after the last attempt the overlay reads "Channel offline". Tuning another channel clears the overlay immediately. (If no offline source is handy, this step can be confirmed during PR review.)

- [ ] **Step 8: Commit**

```bash
git add templates/base.html templates/guide.html
git commit -m "feat(player): waiting overlay with backoff auto-retry"
```

---

## Task 7: Docs + final verification

**Files:**
- Modify: `docs/ideas.md` (idea #38 line ~53)
- Modify: `CLAUDE.md` (test count line ~28)

- [ ] **Step 1: Mark idea #38 done**

In `docs/ideas.md`, change the idea #38 entry to strike-through with a done note, e.g.:

```markdown
38. ~~**Auto-resume offline live channels**~~ — done: `resolve_url_with_status` returns an empty URL + `Offline`/`Upcoming` status on resolve failure (`recoverable_status`); `next_live` classifies each source (`classify_live_outcome`) and returns `TuneResponse { waiting: true }` when no source is playable but ≥1 is offline/upcoming; each probe feeds `health::record_source_liveness`, and the 15-min checker is now liveness-aware for `youtube_live` (`live_status_health` via `cached_live_status`). The player shows a "Waiting for stream…" overlay and re-polls `/tune` on a 15→30→60→120s backoff, settling into "Channel offline". Spec: `docs/superpowers/specs/2026-06-11-auto-resume-offline-live-design.md`.
```

- [ ] **Step 2: Update the test count in CLAUDE.md**

Run `cargo test` first to get the new totals, then update the `cargo test` line in `CLAUDE.md` (currently "332 tests: 262 unit + 70 integration") to match the new unit-test count (4 tests added: `recoverable_status`, `record_source_liveness`, `classify_live_outcome_decision`, `live_status_health`).

- [ ] **Step 3: Full verification**

Run: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
Expected: no fmt diff; no clippy warnings; all tests pass (network-dependent tests remain `#[ignore]`).

- [ ] **Step 4: Commit**

```bash
git add docs/ideas.md CLAUDE.md
git commit -m "docs: mark idea #38 done; update test count"
```

---

## Self-Review Notes

- **Spec coverage:** resolver offline/upcoming (Task 1) ✓; `next_live` waiting outcome + health feed (Tasks 2,4) ✓; liveness-aware checker (Task 5) ✓; frontend overlay + backoff (Task 6) ✓; pure-fn tests for `recoverable_status`/`classify_live_outcome`/`live_status_health` ✓; no migration ✓.
- **Type consistency:** `record_source_liveness(&pool, src, bool)`, `classify_live_outcome(&str, LiveStatus) -> Option<LiveOutcome>`, `live_status_health(LiveStatus) -> (bool, Option<&'static str>)`, `do_http_check(..., Option<&LiveStatusCache>)` used identically across tasks.
- **Upcoming asymmetry (intentional):** `next_live` skips the health write for `Upcoming` (Task 4); the background checker maps `Upcoming → (true, None)` (Task 5). Both avoid penalizing a scheduled stream — consistent intent, different code paths.

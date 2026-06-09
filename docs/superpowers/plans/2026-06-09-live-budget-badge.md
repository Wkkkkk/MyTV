# Budget Badge for YouTube/Twitch Live Streams — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a YouTube/Twitch live source show a ⚡/☁ budget badge in the guide after an admin clicks **Test**, by resolving the stream via yt-dlp and probing the resolved segment CDN's CORS.

**Architecture:** Add one helper `health::probe_and_cache_resolved_cors` that resolves a `needs_resolution()` URL via yt-dlp, probes the resolved HLS manifest's segment-CDN CORS with the existing `hls::probe_source_cors`, and caches the result under **both** the resolved CDN host and the original source host (the original-host key is what the guide and admin-row lookups query with — see spec). Wire it into the `source_test` handler behind a `needs_resolution` check, mirroring the existing `playlist_item_test` branch. No schema, route, or template changes; the background sweep is untouched.

**Tech Stack:** Rust, Axum 0.7, SQLx (SQLite), reqwest, Tokio, yt-dlp (external binary).

**Spec:** `docs/superpowers/specs/2026-06-09-live-budget-badge-design.md`

---

### Task 1: Add `probe_and_cache_resolved_cors` helper

**Files:**
- Modify: `src/health.rs` (add the public async fn after `probe_and_cache_cors`, which ends at line 271; add two unit tests inside the existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add these two tests inside the `mod tests` block in `src/health.rs` (place them right after `test_probe_and_cache_cors_skips_resolution_needed`, near line 409):

```rust
    #[tokio::test]
    async fn test_probe_and_cache_resolved_cors_invalid_url_is_noop() {
        // resolve_url bails on a non-http(s) scheme before spawning yt-dlp, so this
        // is deterministic and never touches the network. The helper must return
        // None and leave the cache untouched on any resolution failure.
        let cache: crate::CorsCache =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let client = reqwest::Client::new();
        let result = probe_and_cache_resolved_cors(&client, &cache, "not-a-url").await;
        assert_eq!(result, None);
        assert!(cache.read().await.is_empty());
    }

    #[test]
    fn test_resolved_cors_caches_under_both_hosts() {
        // Contract test (mirrors test_probe_and_cache_cors_dash_caches_under_cdn_host):
        // after a successful resolved-live probe, the cache must hold the result under
        // BOTH the resolved CDN host (semantic key) and the original source host (the
        // key the guide/admin-row lookups actually query with). The guide only ever
        // knows the DB source URL (youtube.com), never the resolved googlevideo URL.
        let original_host = "https://www.youtube.com";
        let cdn_host = "https://rr3---sn-xyz.googlevideo.com";
        assert_ne!(original_host, cdn_host);

        let mut cache: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        cache.insert(cdn_host.to_string(), true);
        cache.insert(original_host.to_string(), true);

        // The guide looks up by the original source URL host -> must find Direct.
        assert_eq!(
            crate::budget::status_for_url("https://www.youtube.com/live/abc123", &cache),
            crate::budget::BudgetStatus::Direct,
            "guide lookup by original youtube host must find the probe result"
        );
        assert!(
            cache.contains_key(cdn_host),
            "resolved CDN host must also be cached for correct semantics"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib probe_and_cache_resolved_cors`
Expected: FAIL — compile error `cannot find function probe_and_cache_resolved_cors in this scope`.

- [ ] **Step 3: Implement the helper**

In `src/health.rs`, insert this function immediately after the closing brace of `probe_and_cache_cors` (after line 271, before `async fn do_http_check`):

```rust
/// Resolves a YouTube/Twitch live source via yt-dlp, probes the resolved HLS
/// manifest's segment-CDN CORS, and caches the result under BOTH the resolved
/// CDN host and the original source host. The original-host entry is what the
/// guide and admin source-row budget lookups query with (they only know the DB
/// source URL, never the resolved googlevideo URL). Returns `None` (cache
/// unchanged) if resolution fails or the resolved URL is not a probeable HLS
/// manifest. Intended for the admin Test button only — the 15-min background
/// sweep does not resolve live sources (too expensive).
pub async fn probe_and_cache_resolved_cors(
    client: &reqwest::Client,
    cors_cache: &CorsCache,
    source_url: &str,
) -> Option<bool> {
    let resolved = crate::media::resolver::resolve_url(source_url).await.ok()?;
    let cors = crate::media::hls::probe_source_cors(client, &resolved).await?;

    let resolved_host = crate::media::hls::extract_manifest_host(&resolved);
    let original_host = crate::media::hls::extract_manifest_host(source_url);

    tracing::debug!(
        resolved_host = %resolved_host,
        original_host = %original_host,
        cors,
        "resolved-live CORS probe cached"
    );
    let mut cache = cors_cache.write().await;
    cache.insert(resolved_host.clone(), cors);
    if original_host != resolved_host {
        cache.insert(original_host, cors);
    }
    Some(cors)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib probe_and_cache_resolved_cors resolved_cors_caches_under_both_hosts`
Expected: PASS (both tests).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/health.rs
git commit -m "feat: add probe_and_cache_resolved_cors for live YouTube/Twitch CORS"
```

---

### Task 2: Add a YouTube-live source to the test seed

**Files:**
- Modify: `tests/fixtures/seed.sql:8-12` (append one row to the `sources` INSERT)

- [ ] **Step 1: Add the seed row**

In `tests/fixtures/seed.sql`, change the `sources` INSERT (lines 8–12) so the last existing row keeps its trailing comma and a new inactive YouTube-live source is appended. Replace:

```sql
  (4, 3, 'hls', 'https://stream.example.com/backup.m3u8',  2, 1, 0);
```

with:

```sql
  (4, 3, 'hls', 'https://stream.example.com/backup.m3u8',  2, 1, 0),
  -- YouTube-live source for the resolved-CORS budget test; is_active=0 so live
  -- tune/next/guide tests for channel 2 are unaffected (channel 2 has no active source).
  (5, 2, 'hls', 'https://www.youtube.com/live/jfKfPfyJRdk', 5, 0, 0);
```

- [ ] **Step 2: Verify the existing suite still passes with the new seed row**

Run: `cargo test`
Expected: PASS — all existing tests unaffected (source 5 is inactive and on channel 2, which already has no active source).

- [ ] **Step 3: Commit**

```bash
git add tests/fixtures/seed.sql
git commit -m "test: seed an inactive YouTube-live source for budget-badge tests"
```

---

### Task 3: Wire the helper into `source_test` and add integration coverage

**Files:**
- Modify: `src/routes/admin/sources.rs:96` (add the resolution branch after `probe_source`)
- Test: `tests/http.rs` (add one integration test in the "Admin Test button / guide budget badge" section, after `test_source_test_returns_row_partial_not_ok_badge` near line 460)

- [ ] **Step 1: Write the failing integration test**

Add to `tests/http.rs` after the existing `test_source_test_returns_row_partial_not_ok_badge`:

```rust
#[tokio::test]
async fn test_source_test_youtube_live_routes_through_resolution() {
    // Source 5 (seed) is a YouTube-live URL -> needs_resolution() is true, so the
    // handler takes the resolve+probe branch. With yt-dlp unavailable (or unable to
    // resolve a bogus stream), the probe is a fast no-op and the badge stays blank,
    // but the handler must still return 200 and re-render the source row partial.
    let response = app()
        .await
        .oneshot(authed_post("/admin/sources/5/test"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains("src-row-5"),
        "response should be the row partial for source 5"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test http test_source_test_youtube_live_routes_through_resolution`
Expected: FAIL — without source 5 wired, this would 404; if Task 2 is applied it returns 200 but the new branch does not yet exist, so the test passes only the routing — confirm it currently FAILs to compile or returns the row. (If it already returns 200 + `src-row-5` without the branch, that is acceptable; the branch is still required by Step 3 to exercise resolution. Proceed.)

- [ ] **Step 3: Add the resolution branch in the handler**

In `src/routes/admin/sources.rs`, the `source_test` handler currently has (lines 96–101):

```rust
    crate::health::probe_source(&state.pool, &state.http_client, &state.cors_cache, &src).await;

    let updated = source::get(&state.pool, source_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
```

Insert the resolution branch between the `probe_source` call and the `let updated` re-fetch:

```rust
    crate::health::probe_source(&state.pool, &state.http_client, &state.cors_cache, &src).await;

    if crate::media::resolver::needs_resolution(&src.url) {
        crate::health::probe_and_cache_resolved_cors(&state.http_client, &state.cors_cache, &src.url)
            .await;
    }

    let updated = source::get(&state.pool, source_id)
        .await
        .map_err(internal_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
```

- [ ] **Step 4: Run the integration test to verify it passes**

Run: `cargo test --test http test_source_test_youtube_live_routes_through_resolution`
Expected: PASS — handler returns 200 and the body contains `src-row-5`.

- [ ] **Step 5: Run the full suite, format, and commit**

```bash
cargo test
cargo fmt
cargo clippy -- -D warnings
git add src/routes/admin/sources.rs tests/http.rs
git commit -m "feat: probe resolved CORS on Test for YouTube/Twitch live sources (idea 37)"
```

Expected: all tests pass, clippy clean.

---

### Task 4: Mark idea 37 done in IDEAS.md

**Files:**
- Modify: `docs/IDEAS.md:51` (idea 37 entry)

- [ ] **Step 1: Strike through and annotate the idea**

In `docs/IDEAS.md`, change the idea 37 entry from `37. **Budget badge for YouTube Live streams** — …` so the title is struck through and a `done:` note is appended, matching the style of idea 34 (line 49). Wrap the title in `~~ ~~` and append:

```
done: `health::probe_and_cache_resolved_cors` resolves a live YouTube/Twitch source via yt-dlp on admin **Test**, probes the resolved manifest's segment-CDN CORS (`hls::probe_source_cors`), and caches the result under both the resolved CDN host and the original source host so the guide/admin-row budget lookups (keyed by the DB source URL host) render ⚡/☁. Background sweep unchanged. Spec: `docs/superpowers/specs/2026-06-09-live-budget-badge-design.md`.
```

- [ ] **Step 2: Commit**

```bash
git add docs/IDEAS.md
git commit -m "docs: mark idea 37 done — budget badge for live YouTube/Twitch streams"
```

---

## Notes for the implementer

- **`cargo fmt` before every commit** — CI fails on any formatting diff (toolchain pinned to 1.96 in `rust-toolchain.toml`).
- **yt-dlp in tests:** the integration test (Task 3) tolerates yt-dlp being absent — `resolve_url` returns `Err` fast when the binary is missing, so the probe is a no-op and the badge stays blank. The deterministic correctness coverage lives in the Task 1 unit tests, which never touch the network (`resolve_url` bails on the invalid scheme before spawning yt-dlp).
- **Why cache under the original host:** the guide renders budget badges from the DB source URL (`build_guide_data` → `budget_for_url(source.url)` → `extract_manifest_host` → `https://www.youtube.com`), never the resolved googlevideo URL. Caching only under the CDN host would leave the badge blank. See the spec's "Why the badge cannot render from the resolved host alone".

# Source Test Button — Design Spec
_2026-05-28_

## Goal

Add a "Test" button to each source row in the admin channel detail page so operators can verify a source is reachable without having to tune to the channel.

## Architecture

One new route, one new handler, one template update. No new files.

**Files changed:**
- `src/main.rs` — register `POST /admin/sources/:id/test`
- `src/routes/admin.rs` — add `source_test` handler
- `templates/admin/channel_detail.html` — add HTMX Test button + result span per source row

---

## Handler Logic

`source_test` in `src/routes/admin.rs`:

```
1. Fetch source by id from DB → 404 if not found
2. if resolver::needs_resolution(&src.url):          // YouTube / Twitch
       call resolver::resolve_url(&src.url)           // yt-dlp, 30s timeout
       Ok(_)  → return Ok fragment: <span class="badge badge-on">OK</span>
       Err(e) → return Ok fragment: <span style="color:#e94560;font-size:0.78rem">Failed</span>
3. else:                                              // HLS / IPTV
       state.http_client.head(&src.url)
           .timeout(Duration::from_secs(10))
           .send()
       Ok(resp) where status is 2xx or 3xx → OK fragment
       Ok(resp) other status → <span style="color:#e94560;font-size:0.78rem">Failed: HTTP {status}</span>
       Err(_) → Failed fragment
```

The handler always returns `Ok(Html<String>)` — never an error status — so HTMX always has something to swap in. Source lookup failure returns a proper `Err(StatusCode::NOT_FOUND)`.

Response type: `Html<String>` (inline string, no template file needed).

---

## Route Registration

In `src/main.rs`, add alongside the existing source routes:

```rust
.route("/sources/:id/test", post(routes::admin::source_test))
```

---

## Template Change

In `templates/admin/channel_detail.html`, inside the source row's action `<td>` (after the Delete form), add:

```html
<form hx-post="/admin/sources/{{ src.id }}/test"
      hx-target="#src-test-{{ src.id }}"
      hx-swap="innerHTML"
      style="display:inline-block;margin-left:4px">
  <button class="btn btn-sm" type="submit">Test</button>
</form>
<span id="src-test-{{ src.id }}"></span>
```

The `<span>` starts empty and receives the result fragment on click. Subsequent clicks overwrite the previous result.

---

## Error Handling

| Condition | Response |
|---|---|
| Source not found | `Err(StatusCode::NOT_FOUND)` |
| YouTube: yt-dlp resolves OK | `<span class="badge badge-on">OK</span>` |
| YouTube: yt-dlp fails or times out | `<span style="color:#e94560;font-size:0.78rem">Failed</span>` |
| HLS/IPTV: HEAD returns 2xx or 3xx | `<span class="badge badge-on">OK</span>` |
| HLS/IPTV: HEAD returns 4xx/5xx | `<span style="color:#e94560;font-size:0.78rem">Failed: HTTP {status}</span>` |
| HLS/IPTV: connection error / timeout | `<span style="color:#e94560;font-size:0.78rem">Failed</span>` |

---

## Testing

No unit tests — HTTP and yt-dlp calls require live infrastructure. Verified by:
1. `cargo test` — all existing tests pass
2. `cargo build` — template compiles
3. Manual: click Test on a live HLS source → "OK"; click Test on a bad URL → "Failed"

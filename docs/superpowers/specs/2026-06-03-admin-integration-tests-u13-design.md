# Admin Mutation Route Integration Tests — Design Spec (U13)

## Goal

Add HTTP-level integration tests for the admin mutation routes that currently have no test coverage: channel CRUD, source create/delete/toggle, and the discover page GET. The existing test suite covers auth middleware, player routes, and guide rendering but leaves all admin write operations untested.

## Architecture

All tests live in `tests/http.rs`, consistent with every other integration test. A new helper `authed_form_post` handles form-encoded POST bodies. Tests use the existing `oneshot` pattern and the existing seed data.

No new test infrastructure beyond the one helper. No separate test file.

## New Helper

```rust
fn authed_form_post(uri: &str, body: &str) -> Request<Body>
```

- Method: POST
- Content-Type: `application/x-www-form-urlencoded`
- Authorization: `Basic dXNlcjp0ZXN0` (user:test, same as all other authed helpers)
- Body: URL-encoded string passed by caller

Added immediately after the existing `authed_post` function.

## Test Coverage (17 tests)

All tests are grouped under a `// ── Admin mutations ──` section header, matching the file's existing comment style. Sub-sections per resource.

### Channel create — `POST /admin/channels`

| Test | Input | Expected |
|------|-------|----------|
| `channel_create_redirects_on_success` | valid form, authed | 303, `location: /admin/channels` |
| `channel_create_rejects_invalid_type` | `channel_type=invalid`, authed | 422 |
| `channel_create_requires_auth` | valid form, no auth | 401 |

### Channel update — `POST /admin/channels/:id`

| Test | Input | Expected |
|------|-------|----------|
| `channel_update_redirects_on_success` | channel 1, valid form, authed | 303 |
| `channel_update_rejects_invalid_type` | channel 1, `channel_type=invalid`, authed | 422 |
| `channel_update_returns_404_for_missing_channel` | channel 9999, authed | 404 |

### Channel delete — `POST /admin/channels/:id/delete`

| Test | Input | Expected |
|------|-------|----------|
| `channel_delete_redirects_on_success` | channel 1, authed | 303 |
| `channel_delete_returns_404_for_missing_channel` | channel 9999, authed | 404 |

### Channel edit form — `GET /admin/channels/:id/edit`

| Test | Input | Expected |
|------|-------|----------|
| `channel_edit_form_returns_200` | channel 1, authed | 200 |
| `channel_edit_form_returns_404_for_missing_channel` | channel 9999, authed | 404 |

### Source create — `POST /admin/channels/:id/sources`

| Test | Input | Expected |
|------|-------|----------|
| `source_create_redirects_on_success` | channel 1, valid form, authed | 303, `location: /admin/channels/1` |
| `source_create_rejects_invalid_kind` | `kind=rtmp`, authed | 422 |
| `source_create_rejects_empty_url` | `url=`, authed | 422 |

### Source delete — `POST /admin/sources/:id/delete`

| Test | Input | Expected |
|------|-------|----------|
| `source_delete_redirects_on_success` | source 1, authed | 303 |
| `source_delete_returns_404_for_missing_source` | source 9999, authed | 404 |

### Source toggle — `POST /admin/sources/:id/toggle`

| Test | Input | Expected |
|------|-------|----------|
| `source_toggle_redirects_on_success` | source 1, authed | 303 |
| `source_toggle_returns_404_for_missing_source` | source 9999, authed | 404 |

### Discover page — `GET /admin/discover`

| Test | Input | Expected |
|------|-------|----------|
| `admin_discover_page_returns_200` | authed | 200 |
| `admin_discover_page_requires_auth` | no auth | 401 |

## Seed Data Used

- Channel 1 ("Live OK", `live` type) — used for update/delete/edit/source-create
- Source 1 (channel 1, `https://stream.example.com/live.m3u8`, active) — used for delete/toggle
- ID 9999 — guaranteed absent; used for 404 cases

## Out of Scope

- Admin POST mutation response bodies (redirect-only responses have no meaningful body)
- Discover POST actions (m3u search, youtube search, manual resolve, add) — these trigger outbound network calls and are better covered by unit tests on the underlying functions
- Playlist item CRUD — separate concern, can be a follow-up

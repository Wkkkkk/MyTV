# Incident: stream-proxy 502s on Fly.io due to forwarded hop-by-hop headers

**Date:** 2026-06-03  
**Affected endpoint:** `GET /stream-proxy?url=...`  
**Symptom:** HTTP 502 from `kunstv.fly.dev` for all proxied HLS streams  
**Fix commit:** `46fc219`

---

## Behaviour

After deploying the proxy response fidelity feature (item 16), all playback through
the stream proxy broke on the live Fly.io instance. Requests to the proxy URL, e.g.

```
https://kunstv.fly.dev/stream-proxy?url=https://stream.mux.com/...m3u8
```

returned HTTP 502 with no body and only Fly.io infrastructure headers:

```
HTTP/2 502
server: Fly/b59e3505 (2026-05-27)
via: 2 fly.io
fly-request-id: ...
```

The same URL worked correctly when the local dev server was hit directly.

---

## Diagnosis

### Step 1 — check the deployment logs

`fly logs` showed the error immediately:

```
error.message="could not complete HTTP request to instance: hyper error: connection closed before message completed"
proxy[e82e550c226228] ams [error]
request.url="http://worker-dp-ams1-da12/stream-proxy?url=https://stream.mux.com/...m3u8"
```

Two signals in one line:

- **Source: `proxy[...]`** — the error came from Fly.io's Envoy proxy layer, not from
  our app. If the app had returned its own 502, the response would have contained
  our headers (e.g. `access-control-allow-origin: *`). Their absence confirmed Fly.io
  generated the 502 itself because it could not parse our app's response.
- **Message: `connection closed before message completed`** — this is a hyper-level
  HTTP framing error. The proxy received headers from our backend, then found the body
  bytes inconsistent with what the headers described.

### Step 2 — identify what changed

The most recent deployment introduced **item 16 (proxy response fidelity)**, which
added forwarding of all upstream response headers. Before that change, the proxy only
set two response headers itself: `Access-Control-Allow-Origin: *` and `Content-Type`.

### Step 3 — trace the header path

Fetching the upstream URL over HTTP/1.1 (which reqwest uses, since the `http2` feature
is not enabled in `Cargo.toml`) revealed the upstream CDN includes:

```
Transfer-Encoding: chunked
Connection: keep-alive
```

`reqwest` automatically decodes chunked bodies when reading them, but it preserves the
raw response headers in `Response::headers()`. The proxy was forwarding those headers
verbatim to the client.

Fly.io's Envoy proxy sits between the browser and our app. When it received our
response with `Transfer-Encoding: chunked` in the headers, it expected a
chunked-encoded body. But the bytes it received were already decoded plain text
(reqwest had decoded them). The chunk framing was missing, so Envoy could not complete
parsing and closed the connection — logging `connection closed before message completed`.

The bug did not appear locally because curl talks directly to axum/hyper with no
intermediary proxy. Hyper strips `Transfer-Encoding` internally before writing wire
bytes in that case, so curl never saw the conflicting header.

---

## Root cause

`Transfer-Encoding`, `Connection`, and a small set of related headers are
**hop-by-hop headers** defined in RFC 7230 §6.1. They describe the properties of a
single transport hop and must never be forwarded by an intermediary:

> A proxy or gateway MUST parse a received Connection header field before forwarding a
> message and, for each connection-option in this field, remove any header field(s)
> from the message with the same name as the connection-option, and then remove the
> Connection header field itself (RFC 7230 §6.1).

The full list of hop-by-hop headers that must not be forwarded:

| Header | Reason |
|---|---|
| `Transfer-Encoding` | Describes the encoding for this hop only; body is already decoded by the time we read it |
| `Connection` | Names which headers are hop-by-hop for this hop |
| `TE` | Transfer extension negotiation, per-hop |
| `Trailer` | Lists headers sent after chunked body, per-hop |
| `Upgrade` | Protocol upgrade negotiation, per-hop |
| `Keep-Alive` | Connection persistence, per-hop |
| Any header named in `Connection:` | Explicitly marked hop-by-hop by the sender |

---

## Fix

In `src/routes/player.rs`, the header-forwarding loop was updated to skip all
hop-by-hop headers:

```rust
// RFC 7230 §6.1: collect headers listed in Connection so we can strip them too.
let connection_options: Vec<String> = upstream
    .headers()
    .get(axum::http::header::CONNECTION)
    .and_then(|v| v.to_str().ok())
    .map(|s| s.split(',').map(|t| t.trim().to_lowercase()).collect())
    .unwrap_or_default();

for (key, val) in upstream.headers() {
    if key == axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN
        || key == axum::http::header::CONNECTION
        || key == axum::http::header::TRANSFER_ENCODING
        || key == axum::http::header::TE
        || key == axum::http::header::TRAILER
        || key == axum::http::header::UPGRADE
        || connection_options.iter().any(|o| o == key.as_str())
    {
        continue;
    }
    headers.append(key.clone(), val.clone());
}
```

---

## Why it was hard to catch

### The environment gap

The bug only manifested behind a reverse proxy. Local development has no intermediary,
so hyper's internal header handling masked the problem. Any feature that touches raw
HTTP header forwarding should be tested against the actual deployment topology (behind
nginx, Envoy, or a local proxy container) before shipping.

### reqwest preserves raw headers after decoding

`reqwest` transparently decodes `Transfer-Encoding: chunked` when reading the response
body, but does not remove the header from `Response::headers()`. The header and the
body are therefore in an inconsistent state from the moment we read them. Blindly
forwarding `upstream.headers()` into a proxy response will always reproduce this
inconsistency downstream.

---

## Lessons and suggestions

### 1. Check deployment logs first

The log line `hyper error: connection closed before message completed` from the proxy
layer — not the app layer — told the full story in one step. The source of the error
(proxy vs. app) and the error type (framing/encoding, not timeout or application logic)
together pointed directly at header forwarding as the cause. Fetching logs before
reading code saves significant time.

### 2. Add a test for hop-by-hop header absence

The integration test suite mocks HTTP at the tower layer. A test could stand up a local
upstream that returns `Transfer-Encoding: chunked` and `Connection: keep-alive`, then
assert those headers are absent from the proxy response. That test would have caught
this before deployment.

### 3. The general invariant

The specific rule (strip hop-by-hop headers) is an instance of a broader principle:
**forwarded data must not include metadata that was only valid for the upstream
connection.** A proxy response is not the same as a response you built yourself — the
upstream's headers describe *its* connection to *its* client, not your connection to
your client. This applies equally to hop-by-hop headers, internal `Server:` headers
that expose infrastructure, or upstream `Set-Cookie` headers from untrusted origins.

When reviewing any code that iterates `upstream.headers()` to build a forwarded
response, the first question should be: *does each forwarded header describe the
upstream's connection, or does it describe the content itself?* Only the latter should
cross the proxy boundary.

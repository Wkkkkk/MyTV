# Incident: live-status badge fan-out OOM'd the 256 MB Fly VM

**Date:** 2026-06-10
**Affected:** entire app on `kunstv.fly.dev` (not a single endpoint)
**Symptom:** the whole site became unresponsive — `GET /health` and `/` timed out (curl `HTTP 000` after 20 s) — shortly after the live-status visibility feature was deployed and exercised in production
**Fix commits:** `8c62e7d` (global cap), `629a18f` (acquire-before-timeout), `b9682d9` (bounded acquire wait)

---

## Behaviour

The live-status feature adds lazy HTMX badges that fetch `GET /admin/live-status?url=…`
once per YouTube source on page load and swap in `● live` / `○ offline`. It was
deployed, `/health` returned 200, and a quick check passed.

Then, while testing the badges in production (opening the admin discover and channel
pages, which render many badges), the entire app stopped responding. Both `/health`
and `/` timed out with no response at all. The same pages worked fine locally.

A restart (`fly apps restart kunstv`) brought it straight back — and would have knocked
over again on the next admin page load, because nothing about the trigger had changed.

---

## Diagnosis

### Step 1 — confirm the symptom from outside

```
curl -m 20 https://kunstv.fly.dev/health   → HTTP 000, time=20.0s (timeout)
curl -m 20 https://kunstv.fly.dev/          → HTTP 000, time=20.0s (timeout)
```

The machine itself was wedged — even the trivial `/health` handler couldn't answer.
That points at resource exhaustion (OOM thrash / restart loop), not an application
logic error on one route.

### Step 2 — rule out the obvious

`yt-dlp` is invoked via `Command::new("yt-dlp")` for every YouTube resolve and every
live-status probe. First hypothesis: yt-dlp missing from the production image. The
Dockerfile disproved it:

```dockerfile
RUN ... pip3 install --break-system-packages yt-dlp==2026.3.17 ...
```

yt-dlp is present, same version as local. Not the cause.

### Step 3 — measure the real cost

```
/usr/bin/time -l yt-dlp --print is_live --no-playlist -- https://www.youtube.com/@LofiGirl/live
→ peak RSS: 73 MB, 2.08s real
```

Each yt-dlp invocation is a Python process that peaks around **73 MB**.

### Step 4 — connect it to the feature

The badges are **lazy and independent**: a page renders one
`<span hx-get="/admin/live-status?url=…" hx-trigger="load">` per YouTube source, and
each fires its own request on load. The keyword channel-search renders up to **12**
results; a channel page renders one per source. So a single page load fans out to
~12 simultaneous `/admin/live-status` requests, each spawning a 73 MB yt-dlp process.

The VM (`fly.toml`):

```toml
[[vm]]
  memory = "256mb"
  cpu_kind = "shared"
```

12 × 73 MB ≈ **876 MB** of yt-dlp against a **256 MB** box → OOM. The kernel killed
processes (including the app), the machine thrashed/restarted, and `/health` could not
respond. The fan-out was a self-inflicted denial of service.

---

## Root cause

**Unbounded concurrent `yt-dlp` subprocesses.** Nothing limited how many yt-dlp
processes could run at once. The lazy-badge design multiplied one cheap-looking UI
element into a swarm of memory-heavy subprocesses on a memory-constrained host.

This was foreseeable: the design spec even carried a cost note —

> a 12-result channel search fires up to 12 background probes … bounded, parallel,
> cached. Acceptable.

— but "acceptable" was asserted without sizing it against the actual production limits
(256 MB RAM, `hard_limit = 25` request slots). 12 × 73 MB was never multiplied out.
That is the real miss: a hand-waved cost estimate that turned out to be an OOM.

---

## Fix

Three commits, each addressing one layer:

1. **Global concurrency cap (`8c62e7d`).** A process-wide `tokio::sync::Semaphore`
   with **2 permits** gates every `yt-dlp` spawn (badges *and* tune-time resolution).
   2 × 73 MB ≈ 150 MB leaves headroom under 256 MB. This alone stops the OOM.

2. **Acquire the permit *before* starting the command timeout (`629a18f`).** The first
   cut wrapped `tokio::time::timeout(8s, cmd)` as an argument to the cap helper — but
   Rust evaluates arguments first, so the 8 s deadline started *before* the permit was
   acquired. A caller that waited 7 s for a slot then had 1 s left to actually run.
   Fixed by passing a **closure** that builds the timeout only after the permit is held.

3. **Bound the wait for a permit (`b9682d9`).** `acquire()` originally waited
   indefinitely, so a burst of badges could park many request handlers for tens of
   seconds against the 25-slot limit. The helper now waits a bounded time and returns
   `None` (load-shed) if no slot frees — probes render `Unknown` (`?`), resolve/fetch
   return a "resolver busy" error. Handlers free quickly instead of piling up.

Final shape:

```rust
async fn run_under_cap<F, Fut>(sem: &Semaphore, wait: Duration, f: F) -> Option<Fut::Output>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future,
{
    let _permit = match tokio::time::timeout(wait, sem.acquire()).await {
        Ok(Ok(permit)) => permit,
        _ => return None, // no slot within `wait`
    };
    Some(f().await)
}
```

---

## Why it was missed

### The environment gap

Local dev and CI run on machines with gigabytes of RAM and no request-slot pressure.
The fan-out is invisible there: 12 concurrent yt-dlp processes (≈900 MB) are nothing on
a laptop, so every test and every local smoke check passed. The bug only exists relative
to the 256 MB / 25-slot production envelope. Resource-bounded failures don't reproduce
in resource-rich environments.

### A cost note without a number

The spec acknowledged the 12-probe fan-out but rated it "acceptable" qualitatively. No
one multiplied 12 × (per-process memory) and compared it to the VM's RAM. A cost note
that doesn't carry the arithmetic against the target host is just a feeling.

### Subprocess spawning is an amplifier

Spawning an OS process per request is categorically different from in-process work: it
multiplies a UI element into heavyweight, memory-owning units the scheduler can't pack.
There was no global guardrail on yt-dlp spawning *at all* — the cap should have existed
from the day yt-dlp was first invoked, independent of any feature.

---

## Lessons and suggestions

1. **Size fan-out against the real deployment, not "it's bounded."** Bounded ≠ safe.
   `N` concurrent × `per-unit cost` must be computed against actual host RAM/CPU/slots.
   For this app: any per-request work is implicitly capped by `hard_limit = 25` *and*
   256 MB — both ceilings matter.

2. **Cap any per-request external-process spawn globally, by default.** A process-wide
   semaphore on `yt-dlp` is defense-in-depth that should exist regardless of which
   feature triggers spawns. Treat "spawns a subprocess per request/item" as a red flag
   that demands a concurrency bound at review time.

3. **Quantify cost notes.** When a spec says a fan-out is "acceptable," require the
   number and the target environment in the same sentence, or it isn't a decision.

4. **Roll out resource-sensitive features behind a single probe first.** When testing a
   fan-out feature in production, exercise one badge before opening a 12-result page.
   The blast radius of "load the whole page on a tiny box" is the incident itself.

5. **Restart restores, it doesn't fix.** `fly apps restart` cleared the stuck processes
   and bought time, but the durable fix is the cap + load-shed. Don't mistake recovery
   for resolution.

### The general invariant

**Any code that launches an OS process per request or per item must have a global
concurrency bound sized to the host, and should shed load rather than queue unboundedly
when that bound is reached.** Memory-heavy subprocesses fanned out by a UI are a
self-inflicted DoS waiting for the first busy page. The question to ask at review of any
`Command::new(...)` on a request path is: *how many of these can run at once on the
smallest box we deploy to, and what happens to caller number N+1?*

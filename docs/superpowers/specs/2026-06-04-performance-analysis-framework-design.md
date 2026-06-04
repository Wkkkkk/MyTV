# Performance Analysis Framework — Design

**Date**: 2026-06-04
**Status**: Approved
**Goal**: Proactive performance analysis framework for MyTV covering memory and latency. No specific problem observed; the aim is a structured map of what to measure, plus the measurement tooling to establish baselines and catch regressions.

## Decisions made during brainstorming

| Question | Decision |
|----------|----------|
| Driver | Proactive — framework doc + measurement tooling, no observed problem |
| Environment | Both local and live, local-first; light runtime metrics observable on kunstv.fly.dev |
| Instrumentation footprint | Light in-app metrics (small endpoint + atomics), no heavy observability stack |
| Structure | Resource-first: Latency organized by user journey, Memory organized by subsystem |

## Grounded facts (verified in code/config)

- Fly VM: **256MB RAM, 1 shared vCPU**, HTTP concurrency hard limit 25 / soft 20 (`fly.toml`)
- `min_machines_running = 0` with `auto_stop_machines = true` → **cold starts are a real user-facing latency journey**
- Stream proxy **streams segment bodies** (`Body::from_stream(upstream.bytes_stream())`, `src/routes/player.rs:317`) — bounded per-connection memory; manifests are fully buffered before rewrite
- SQLite on a 1GB Fly volume at `/data/mytv.db`

## Deliverables

| # | Deliverable | Location |
|---|------------|----------|
| 1 | Framework doc — mind map + measurement plan + baseline table | `docs/performance/FRAMEWORK.md` |
| 2 | Criterion micro-benchmarks for pure hot functions | `benches/` |
| 3 | Macro harness — load-test scripts + profiling recipes | `scripts/perf/` |
| 4 | Light in-app metrics endpoint | `src/` (`/admin/metrics`) |

## 1. Framework doc (`docs/performance/FRAMEWORK.md`)

Top-level structure: **Envelope → Latency → Memory**. Every leaf gets four fields: *what to measure / tool / hypothesis / baseline slot*.

### Envelope (constraints everything lives under)

- 256MB RAM, 1 shared vCPU
- 25-connection hard limit
- Cold starts (`min_machines_running = 0`)
- SQLite on a 1GB volume

### Latency branch — organized by user journey

| Journey | Path | Suspected hot spots |
|---------|------|---------------------|
| Cold start | Fly machine boot → app start → first response | Machine boot dominates; worst-case "turn on the TV" |
| Tune live | DB lookup → SSRF check (DNS, 60s cache hit/miss) → manifest fetch + `rewrite_hls_urls` → JSON | Upstream manifest fetch; DNS on cache miss |
| Tune VOD | Position calc + playlist query (+ yt-dlp resolve when applicable) | yt-dlp subprocess — suspected dominant cost (seconds) |
| Steady-state playback | Each manifest refresh through `/stream-proxy`: upstream fetch + rewrite; segment TTFB proxy vs direct | Per-refresh upstream round-trip |
| Failover (`/next`) | Failure detection + retry chain | Chain length × per-attempt timeout |
| Guide load | EPG window calc → layout → CORS budget badges → Askama render; HTMX partial | Layout/badges scaling with channel count |
| Admin discover | YouTube API round-trip; large M3U download + parse | Network-bound; M3U parse on large playlists |
| Background interference | Health checker 15-min tick sharing CPU/connection pool with foreground | Probe bursts on 1 shared vCPU |

### Memory branch — organized by subsystem

| Subsystem | Question | Risk |
|-----------|----------|------|
| Stream proxy | Segments streamed (verified ✓). Manifests fully buffered — is there a size cap? Worst case = 25 conns × buffer | Medium |
| yt-dlp children | Python interpreter RSS ~100–200MB per spawn. Concurrency cap? | **#1 OOM risk on 256MB VM** |
| SQLite | Pool size × per-connection page cache | Low |
| SSRF hostname cache | 60s TTL — growth bound under hostname churn? | Low |
| Health checker | Does it buffer probe response bodies? | Low–Medium |
| Askama renders | Guide page allocation size as channel count grows | Low |
| Baseline | Tokio runtime + binary RSS at idle | Reference point |

The doc ends with a **baseline table to fill in**. The framework's definition of done is *baselines recorded*, not optimizations made. Optimization work becomes separate, evidence-driven follow-ups.

## 2. Micro-benchmarks (`benches/`)

- Tool: **criterion** (dev-dependency), `[[bench]]` with `harness = false`
- Targets (pure functions, no I/O):
  - `epg` time-window calculation
  - `guide/layout` computation
  - `budget` badge computation
  - `rewrite_hls_urls` + M3U parse with a large fixture manifest
- Excluded from CI test run; kept compilable via `cargo bench --no-run` locally

## 3. Macro harness (`scripts/perf/`)

- `oha` load-test recipes for `/guide` and `/stream-proxy`
- `hyperfine` recipe for the tune endpoint
- A tiny local mock HLS origin (static fixture manifests/segments) so the proxy can be load-tested offline without real upstreams
- Documented profiling recipes: CPU via `cargo flamegraph`, heap via `dhat`/Instruments
- Scripts assume a locally running server seeded with test data

## 4. In-app metrics (`/admin/metrics`)

- Behind existing admin auth, JSON response
- Hand-rolled `Arc<Metrics>` with atomics in `AppState` — **no new heavy dependencies**
- Exposes:
  - Process RSS — read from `/proc/self/statm` on Linux (Fly); `null` on macOS
  - Per-route request counts + fixed-bucket latency histogram, recorded by a small tower middleware
  - Proxy: cumulative bytes proxied, active-streams gauge (increment/decrement guard)
  - SSRF cache entry count
- Observable on the live instance at kunstv.fly.dev

## Testing

- Integration tests (in `tests/http.rs` style, `tower::ServiceExt::oneshot`):
  - `/admin/metrics` requires auth
  - Returns expected JSON shape
  - Route counter increments after a request passes through the middleware
- Benches and scripts carry no test burden beyond compiling/running locally

## Error handling

- RSS read failure (non-Linux or `/proc` unreadable) → `null` field, never an error response
- Metrics recording is fire-and-forget atomics — cannot fail or add latency on the request path

## Out of scope

- Any actual optimization work (follows from baselines, separately)
- Prometheus/metrics-crate exporters, jemalloc, structured tracing overhaul
- CI performance gates (possible follow-up once baselines exist)

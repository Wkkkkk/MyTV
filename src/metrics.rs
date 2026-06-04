use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Upper bounds in µs for latency buckets (1ms, 10ms, 50ms, 250ms, 1s); a sixth bucket catches everything above.
pub const BUCKET_BOUNDS_MICROS: [u64; 5] = [1_000, 10_000, 50_000, 250_000, 1_000_000];

#[derive(Default)]
struct RouteStats {
    count: AtomicU64,
    total_micros: AtomicU64,
    buckets: [AtomicU64; 6],
}

#[derive(Serialize)]
pub struct RouteSnapshot {
    pub count: u64,
    pub total_micros: u64,
    /// Counts per bucket: ≤1ms, ≤10ms, ≤50ms, ≤250ms, ≤1s, >1s.
    pub buckets: [u64; 6],
}

#[derive(Default)]
pub struct Metrics {
    routes: RwLock<HashMap<String, Arc<RouteStats>>>,
    pub proxy_bytes: AtomicU64,
    pub active_streams: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, route: &str, micros: u64) {
        let stats = {
            let routes = self.routes.read().unwrap();
            routes.get(route).cloned()
        };
        let stats = match stats {
            Some(s) => s,
            None => {
                let mut routes = self.routes.write().unwrap();
                // or_default() makes concurrent first-inserts converge on one Arc, so the
                // read-then-write upgrade can't lose recordings.
                routes.entry(route.to_string()).or_default().clone()
            }
        };
        stats.count.fetch_add(1, Ordering::Relaxed);
        stats.total_micros.fetch_add(micros, Ordering::Relaxed);
        let idx = BUCKET_BOUNDS_MICROS
            .iter()
            .position(|&bound| micros <= bound)
            .unwrap_or(BUCKET_BOUNDS_MICROS.len());
        stats.buckets[idx].fetch_add(1, Ordering::Relaxed);
    }

    pub fn route_snapshots(&self) -> BTreeMap<String, RouteSnapshot> {
        let routes = self.routes.read().unwrap();
        routes
            .iter()
            .map(|(route, s)| {
                (
                    route.clone(),
                    RouteSnapshot {
                        count: s.count.load(Ordering::Relaxed),
                        total_micros: s.total_micros.load(Ordering::Relaxed),
                        buckets: std::array::from_fn(|i| s.buckets[i].load(Ordering::Relaxed)),
                    },
                )
            })
            .collect()
    }
}

/// RAII gauge: increments `active_streams` on creation, decrements on drop.
pub struct ActiveStreamGuard(Arc<Metrics>);

impl ActiveStreamGuard {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        metrics.active_streams.fetch_add(1, Ordering::Relaxed);
        Self(metrics)
    }
}

impl Drop for ActiveStreamGuard {
    fn drop(&mut self) {
        self.0.active_streams.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Resident set size in bytes from /proc/self/statm. 4096-byte pages on Fly's Linux.
#[cfg(target_os = "linux")]
pub fn rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(resident_pages * 4096)
}

#[cfg(not(target_os = "linux"))]
pub fn rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_places_latency_in_correct_buckets() {
        let m = Metrics::new();
        m.record("/guide", 500); // 0.5ms → bucket 0
        m.record("/guide", 1_500); // 1.5ms → bucket 1
        m.record("/guide", 30_000); // 30ms → bucket 2
        m.record("/guide", 2_000_000); // 2s → overflow bucket 5
        let snap = m.route_snapshots();
        let g = &snap["/guide"];
        assert_eq!(g.count, 4);
        assert_eq!(g.buckets, [1, 1, 1, 0, 0, 1]);
        assert_eq!(g.total_micros, 2_032_000);
    }

    #[test]
    fn record_bucket_boundaries_are_inclusive() {
        let m = Metrics::new();
        m.record("/x", 0); // → bucket 0
        m.record("/x", 1_000); // exactly 1ms → bucket 0
        m.record("/x", 100_000); // 100ms → bucket 3
        m.record("/x", 1_000_000); // exactly 1s → bucket 4
        m.record("/x", 1_000_001); // just over 1s → bucket 5
        let snap = m.route_snapshots();
        assert_eq!(snap["/x"].buckets, [2, 0, 0, 1, 1, 1]);
    }

    #[test]
    fn record_tracks_routes_independently() {
        let m = Metrics::new();
        m.record("/guide", 1000);
        m.record("/channel/:id/tune", 1000);
        let snap = m.route_snapshots();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap["/guide"].count, 1);
        assert_eq!(snap["/channel/:id/tune"].count, 1);
    }

    #[test]
    fn snapshots_empty_when_nothing_recorded() {
        assert!(Metrics::new().route_snapshots().is_empty());
    }

    #[test]
    fn active_stream_guard_decrements_on_drop() {
        let m = Arc::new(Metrics::new());
        let guard = ActiveStreamGuard::new(m.clone());
        assert_eq!(m.active_streams.load(Ordering::Relaxed), 1);
        drop(guard);
        assert_eq!(m.active_streams.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn rss_bytes_is_some_on_linux_none_elsewhere() {
        if cfg!(target_os = "linux") {
            assert!(rss_bytes().unwrap() > 0);
        } else {
            assert!(rss_bytes().is_none());
        }
    }
}

use chrono::{Duration, TimeZone, Utc};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mytv::media::{hls, m3u};
use mytv::model::playlist_item::PlaylistItem;

fn bench_vod_schedule(c: &mut Criterion) {
    let items: Vec<PlaylistItem> = (0..200)
        .map(|i| PlaylistItem {
            id: i,
            channel_id: 1,
            title: format!("Episode {i}"),
            url: format!("https://example.com/ep{i}.mp4"),
            duration_secs: 1500,
            sort_order: i,
        })
        .collect();
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let end = start + Duration::hours(4);
    c.bench_function("epg::vod_schedule/200items_4h", |b| {
        b.iter(|| mytv::epg::vod_schedule(black_box(1), black_box(&items), 0, start, end))
    });
}

fn bench_rewrite_hls(c: &mut Criterion) {
    let mut manifest = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:6\n");
    for i in 0..2000 {
        manifest.push_str(&format!("#EXTINF:6.0,\nseg{i}.ts\n"));
    }
    c.bench_function("hls::rewrite_hls_urls/2000segments", |b| {
        b.iter(|| {
            hls::rewrite_hls_urls(
                black_box(&manifest),
                "https://example.com/live/index.m3u8",
                false,
            )
        })
    });
}

fn bench_parse_m3u(c: &mut Criterion) {
    let mut playlist = String::from("#EXTM3U\n");
    for i in 0..10_000 {
        playlist.push_str(&format!(
            "#EXTINF:-1 tvg-id=\"ch{i}\" group-title=\"News\",Channel {i}\nhttps://example.com/ch{i}/index.m3u8\n"
        ));
    }
    c.bench_function("m3u::parse_m3u/10k_channels", |b| {
        b.iter(|| m3u::parse_m3u(black_box(&playlist)))
    });
}

fn bench_budget_status(c: &mut Criterion) {
    let mut cache = std::collections::HashMap::new();
    for i in 0..100 {
        cache.insert(format!("https://cdn{i}.example.com"), i % 2 == 0);
    }
    c.bench_function("budget::status_for_url/cache_hit", |b| {
        b.iter(|| {
            mytv::budget::status_for_url(
                black_box("https://cdn42.example.com/x.m3u8"),
                black_box(&cache),
            )
        })
    });
}

criterion_group!(
    benches,
    bench_vod_schedule,
    bench_rewrite_hls,
    bench_parse_m3u,
    bench_budget_status
);
criterion_main!(benches);

use chrono::Utc;
use sqlx::SqlitePool;

use crate::{
    budget::budget_badge,
    epg,
    model::{
        channel::{self, Channel, ChannelType},
        playlist_item, source,
    },
};

use super::badges::{
    budget_for_url, category_icon, derive_channel_status, vod_budget_url, SourceFacts,
};
use super::layout::{
    compute_window, entry_to_slot, now_line_pct, time_labels, ProgramSlot, TimeLabel,
};

pub(super) struct ChannelRow {
    pub name: String,
    pub category_icon: &'static str,
    pub status_color: &'static str,
    pub status_glyph: &'static str,
    pub status_title: String,
    pub budget_badge_class: &'static str,
    pub budget_badge_char: &'static str,
    pub programs: Vec<ProgramSlot>,
}

pub(super) struct GuideData {
    pub categories: Vec<String>,
    pub active_category: String,
    pub offset_hours: i64,
    pub offset_prev: i64,
    pub offset_next: i64,
    pub window_label: String,
    pub labels: Vec<TimeLabel>,
    pub now_pct: Option<f64>,
    pub rows: Vec<ChannelRow>,
    pub channels_json: String,
}

pub(super) async fn build_guide_data(
    pool: &SqlitePool,
    cors_cache: &std::collections::HashMap<String, bool>,
    live_snapshot: &std::collections::HashMap<String, crate::media::resolver::LiveStatus>,
    category: &str,
    offset_hours: i64,
) -> anyhow::Result<GuideData> {
    let now = Utc::now();
    let (window_start, window_end) = compute_window(now.timestamp(), offset_hours);

    let all_channels = channel::list(pool).await?;
    let categories = channel::distinct_categories(&all_channels);

    let channels_json = serde_json::to_string(
        &all_channels
            .iter()
            .map(|c| serde_json::json!({"id": c.id, "name": c.name, "type": c.r#type}))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string())
    .replace("</", r"<\/");

    let channels: Vec<Channel> = if category == "all" {
        all_channels
    } else {
        all_channels
            .into_iter()
            .filter(|c| c.category == category)
            .collect()
    };

    let sources_by_channel: std::collections::HashMap<i64, Vec<source::Source>> =
        source::list_all(pool).await?.into_iter().fold(
            std::collections::HashMap::new(),
            |mut acc, s| {
                acc.entry(s.channel_id).or_default().push(s);
                acc
            },
        );

    let all_playlist_items: std::collections::HashMap<i64, Vec<playlist_item::PlaylistItem>> =
        playlist_item::list_all(pool)
            .await?
            .into_iter()
            .filter(|item| item.is_active)
            .fold(std::collections::HashMap::new(), |mut acc, item| {
                acc.entry(item.channel_id).or_default().push(item);
                acc
            });

    let mut rows = Vec::new();
    for ch in &channels {
        let (entries, budget_url) = match ch.channel_type() {
            ChannelType::Live => {
                let first_active_url = sources_by_channel
                    .get(&ch.id)
                    .and_then(|v| v.iter().find(|s| s.is_active).map(|s| s.url.clone()));
                (
                    vec![epg::live_entry(ch.id, &ch.name, window_start, window_end)],
                    first_active_url,
                )
            }
            ChannelType::VodLoop | ChannelType::VodOnDemand => {
                let items = all_playlist_items.get(&ch.id).cloned().unwrap_or_default();
                let entries = match ch.loop_anchor {
                    Some(anchor) => epg::vod_schedule(
                        ch.id,
                        &items,
                        anchor.timestamp(),
                        window_start,
                        window_end,
                    ),
                    None => vec![],
                };
                let budget_url = vod_budget_url(&items, ch.loop_anchor, now);
                (entries, budget_url)
            }
        };
        let programs: Vec<ProgramSlot> = entries
            .iter()
            .filter_map(|e| entry_to_slot(e, window_start, window_end))
            .collect();
        let empty: Vec<source::Source> = Vec::new();
        let chan_sources = sources_by_channel.get(&ch.id).unwrap_or(&empty);
        let facts: Vec<SourceFacts> = chan_sources
            .iter()
            .map(|s| SourceFacts {
                kind: s.kind.clone(),
                is_active: s.is_active,
                last_status: s.last_status.clone(),
                failure_reason: s.failure_reason.clone(),
            })
            .collect();
        let source_urls: Vec<String> = chan_sources.iter().map(|s| s.url.clone()).collect();
        let status = derive_channel_status(&ch.channel_type(), &facts, live_snapshot, &source_urls);
        let status_badge = crate::status::status_badge(&status);
        let budget = budget_for_url(budget_url.as_deref(), cors_cache);
        let (budget_badge_class, budget_badge_char) = budget_badge(budget);
        rows.push(ChannelRow {
            name: ch.name.clone(),
            category_icon: category_icon(&ch.category),
            status_color: status_badge.color,
            status_glyph: status_badge.glyph,
            status_title: status_badge.title,
            budget_badge_class,
            budget_badge_char,
            programs,
        });
    }

    Ok(GuideData {
        categories,
        active_category: category.to_string(),
        offset_hours,
        offset_prev: offset_hours - 2,
        offset_next: offset_hours + 2,
        window_label: format!(
            "{} – {}",
            window_start.format("%H:%M"),
            window_end.format("%H:%M")
        ),
        labels: time_labels(window_start, window_end),
        now_pct: now_line_pct(now, window_start, window_end),
        rows,
        channels_json,
    })
}

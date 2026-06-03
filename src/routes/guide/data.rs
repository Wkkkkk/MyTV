use chrono::Utc;
use sqlx::SqlitePool;

use crate::{
    budget::budget_badge,
    epg,
    model::{
        channel::{self, Channel, ChannelType},
        playlist_item,
    },
};

use super::badges::{
    budget_for_url, category_icon, derive_health_status, health_badge, vod_budget_url,
};
use super::layout::{
    compute_window, entry_to_slot, now_line_pct, time_labels, ProgramSlot, TimeLabel,
};

pub(super) struct ChannelRow {
    pub name: String,
    pub category_icon: &'static str,
    pub health_badge_class: &'static str,
    pub health_badge_char: &'static str,
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
            .map(|c| serde_json::json!({"id": c.id, "name": c.name}))
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

    let all_source_ids: std::collections::HashSet<i64> =
        sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();

    let active_source_ids: std::collections::HashSet<i64> =
        sqlx::query_scalar::<_, i64>("SELECT DISTINCT channel_id FROM sources WHERE is_active = 1")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();

    #[derive(sqlx::FromRow)]
    struct SourceUrlRow {
        channel_id: i64,
        url: String,
    }

    let source_url_rows = sqlx::query_as::<_, SourceUrlRow>(
        "SELECT channel_id, url FROM sources WHERE is_active = 1 ORDER BY channel_id, priority",
    )
    .fetch_all(pool)
    .await?;

    let first_active_urls: std::collections::HashMap<i64, String> = source_url_rows
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut acc, row| {
            acc.entry(row.channel_id).or_insert(row.url);
            acc
        });

    let all_playlist_items: std::collections::HashMap<i64, Vec<playlist_item::PlaylistItem>> =
        playlist_item::list_all(pool).await?.into_iter().fold(
            std::collections::HashMap::new(),
            |mut acc, item| {
                acc.entry(item.channel_id).or_default().push(item);
                acc
            },
        );

    let mut rows = Vec::new();
    for ch in &channels {
        let (entries, budget_url) = match ch.channel_type() {
            ChannelType::Live => (
                vec![epg::live_entry(ch.id, &ch.name, window_start, window_end)],
                first_active_urls.get(&ch.id).cloned(),
            ),
            ChannelType::VodLoop => {
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
        let health = derive_health_status(
            ch.id,
            &ch.channel_type(),
            &all_source_ids,
            &active_source_ids,
        );
        let budget = budget_for_url(budget_url.as_deref(), cors_cache);
        let (health_badge_class, health_badge_char) = health_badge(health);
        let (budget_badge_class, budget_badge_char) = budget_badge(budget);
        rows.push(ChannelRow {
            name: ch.name.clone(),
            category_icon: category_icon(&ch.category),
            health_badge_class,
            health_badge_char,
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

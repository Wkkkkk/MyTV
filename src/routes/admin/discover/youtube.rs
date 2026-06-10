pub(super) fn normalize_channel_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(handle) = trimmed.strip_prefix('@') {
        if handle.is_empty() {
            return None;
        }
        return Some(format!("https://www.youtube.com/@{}/live", handle));
    }
    if !trimmed.to_ascii_lowercase().contains("youtube.com") {
        return None;
    }
    let base = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let base = base.trim_end_matches('/');
    if base.ends_with("/live") {
        return Some(base.to_string());
    }
    Some(format!("{}/live", base))
}

pub(super) fn channel_title_from_url(url: &str) -> String {
    let trimmed = url.trim_end_matches("/live").trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("YouTube Channel")
        .to_string()
}

pub struct YoutubeResultRow {
    pub title: String,
    pub channel_title: String,
    pub is_live: bool,
    pub is_upcoming: bool,
    pub duration_secs: i64,
    pub scheduled_start: String,
    pub thumbnail_url: String,
    pub url: String,
    pub source_kind: String,
    pub form_id: usize,
}

pub(super) fn build_video_rows(
    items: &[serde_json::Value],
    duration_map: &std::collections::HashMap<String, i64>,
    scheduled_map: &std::collections::HashMap<String, String>,
) -> Vec<YoutubeResultRow> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let video_id = item["id"]["videoId"].as_str()?;
            let snippet = &item["snippet"];
            let title = snippet["title"].as_str().unwrap_or("Unknown").to_string();
            let channel_title = snippet["channelTitle"].as_str().unwrap_or("").to_string();
            let broadcast = snippet["liveBroadcastContent"].as_str().unwrap_or("none");
            let is_live = broadcast == "live";
            let is_upcoming = broadcast == "upcoming";
            let duration_secs = *duration_map.get(video_id).unwrap_or(&0);
            let scheduled_start = if is_upcoming {
                scheduled_map
                    .get(video_id)
                    .map(|ts| format_scheduled_start(ts))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let thumbnail_url = snippet["thumbnails"]["default"]["url"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let source_kind = if is_live || is_upcoming {
                "youtube_live"
            } else {
                "youtube_vod"
            }
            .to_string();
            let url = format!("https://www.youtube.com/watch?v={}", video_id);
            Some(YoutubeResultRow {
                title,
                channel_title,
                is_live,
                is_upcoming,
                duration_secs,
                scheduled_start,
                thumbnail_url,
                url,
                source_kind,
                form_id: i,
            })
        })
        .collect()
}

pub(super) fn build_channel_rows(items: &[serde_json::Value]) -> Vec<YoutubeResultRow> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let channel_id = item["id"]["channelId"].as_str()?;
            let snippet = &item["snippet"];
            let title = snippet["title"].as_str().unwrap_or("Unknown").to_string();
            let thumbnail_url = snippet["thumbnails"]["default"]["url"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let url = format!("https://www.youtube.com/channel/{}/live", channel_id);
            Some(YoutubeResultRow {
                title,
                channel_title: String::new(),
                is_live: true,
                is_upcoming: false,
                duration_secs: 0,
                scheduled_start: String::new(),
                thumbnail_url,
                url,
                source_kind: "youtube_live".to_string(),
                form_id: i,
            })
        })
        .collect()
}

pub(super) async fn fetch_youtube_channels(
    keyword: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<YoutubeResultRow>> {
    let search_resp: serde_json::Value = client
        .get("https://www.googleapis.com/youtube/v3/search")
        .query(&[
            ("part", "snippet"),
            ("type", "channel"),
            ("maxResults", "12"),
            ("q", keyword),
            ("key", api_key),
        ])
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = search_resp.get("error") {
        let msg = err["message"]
            .as_str()
            .unwrap_or("YouTube API error")
            .to_string();
        anyhow::bail!("{}", msg);
    }

    let items = match search_resp["items"].as_array() {
        Some(v) => v,
        None => return Ok(vec![]),
    };

    Ok(build_channel_rows(items))
}

pub(super) fn format_scheduled_start(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| crate::media::format_utc_short(dt.with_timezone(&chrono::Utc)))
        .unwrap_or_default()
}

pub fn parse_iso8601_duration(s: &str) -> i64 {
    let s = s.strip_prefix("PT").unwrap_or(s);
    let mut total = 0i64;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '0'..='9' => current.push(ch),
            'H' => {
                total += current.parse::<i64>().unwrap_or(0) * 3600;
                current.clear();
            }
            'M' => {
                total += current.parse::<i64>().unwrap_or(0) * 60;
                current.clear();
            }
            'S' => {
                total += current.parse::<i64>().unwrap_or(0);
                current.clear();
            }
            _ => current.clear(),
        }
    }
    total
}

pub(super) async fn fetch_youtube_results(
    keyword: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<YoutubeResultRow>> {
    let search_resp: serde_json::Value = client
        .get("https://www.googleapis.com/youtube/v3/search")
        .query(&[
            ("part", "snippet"),
            ("type", "video"),
            ("maxResults", "12"),
            ("q", keyword),
            ("key", api_key),
        ])
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = search_resp.get("error") {
        let msg = err["message"]
            .as_str()
            .unwrap_or("YouTube API error")
            .to_string();
        anyhow::bail!("{}", msg);
    }

    let items = match search_resp["items"].as_array() {
        Some(v) => v,
        None => return Ok(vec![]),
    };

    let video_ids: Vec<&str> = items
        .iter()
        .filter_map(|item| item["id"]["videoId"].as_str())
        .collect();

    if video_ids.is_empty() {
        return Ok(vec![]);
    }

    let ids_joined = video_ids.join(",");
    let details_resp: serde_json::Value = client
        .get("https://www.googleapis.com/youtube/v3/videos")
        .query(&[
            ("part", "contentDetails,liveStreamingDetails"),
            ("id", ids_joined.as_str()),
            ("key", api_key),
        ])
        .send()
        .await?
        .json()
        .await?;

    let mut duration_map = std::collections::HashMap::<String, i64>::new();
    let mut scheduled_map = std::collections::HashMap::<String, String>::new();
    if let Some(detail_items) = details_resp["items"].as_array() {
        for item in detail_items {
            let id = item["id"].as_str().unwrap_or("").to_string();
            let dur_str = item["contentDetails"]["duration"]
                .as_str()
                .unwrap_or("PT0S");
            duration_map.insert(id.clone(), parse_iso8601_duration(dur_str));
            if let Some(ts) = item["liveStreamingDetails"]["scheduledStartTime"].as_str() {
                scheduled_map.insert(id, ts.to_string());
            }
        }
    }

    let rows = build_video_rows(items, &duration_map, &scheduled_map);

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_rows_build_live_urls() {
        let items = vec![serde_json::json!({
            "id": {"channelId": "UC123"},
            "snippet": {"title": "NASA", "channelTitle": "NASA",
                        "thumbnails": {"default": {"url": "https://yt3.ggpht.com/nasa.jpg"}}}
        })];
        let rows = build_channel_rows(&items);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].url, "https://www.youtube.com/channel/UC123/live");
        assert!(rows[0].is_live);
        assert_eq!(rows[0].duration_secs, 0);
        assert_eq!(rows[0].source_kind, "youtube_live");
        assert_eq!(rows[0].title, "NASA");
        assert_eq!(rows[0].channel_title, "");
        assert!(!rows[0].is_upcoming);
        assert_eq!(rows[0].thumbnail_url, "https://yt3.ggpht.com/nasa.jpg");
    }

    #[test]
    fn test_parse_iso8601_duration() {
        assert_eq!(parse_iso8601_duration("PT4M13S"), 253);
        assert_eq!(parse_iso8601_duration("PT1H30M"), 5400);
        assert_eq!(parse_iso8601_duration("PT2H"), 7200);
        assert_eq!(parse_iso8601_duration("PT0S"), 0);
        assert_eq!(parse_iso8601_duration("PT45S"), 45);
    }

    #[test]
    fn normalize_channel_url_cases() {
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/channel/UC123"),
            Some("https://www.youtube.com/channel/UC123/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/@NASA"),
            Some("https://www.youtube.com/@NASA/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("@NASA"),
            Some("https://www.youtube.com/@NASA/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/c/NASA/"),
            Some("https://www.youtube.com/c/NASA/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/user/NASAtelevision"),
            Some("https://www.youtube.com/user/NASAtelevision/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/@NASA/live"),
            Some("https://www.youtube.com/@NASA/live".to_string())
        );
        assert_eq!(
            normalize_channel_url("https://www.youtube.com/channel/UC123?ab=1"),
            Some("https://www.youtube.com/channel/UC123/live".to_string())
        );
        assert_eq!(normalize_channel_url("https://example.com/foo"), None);
        assert_eq!(normalize_channel_url(""), None);
    }

    #[test]
    fn channel_title_from_url_cases() {
        assert_eq!(
            channel_title_from_url("https://www.youtube.com/@NASA/live"),
            "@NASA"
        );
        assert_eq!(
            channel_title_from_url("https://www.youtube.com/channel/UC123/live"),
            "UC123"
        );
    }

    #[test]
    fn video_rows_label_vod_when_not_live() {
        let items = vec![
            serde_json::json!({
                "id": {"videoId": "abc"},
                "snippet": {"title": "A VOD", "channelTitle": "Chan",
                            "liveBroadcastContent": "none"}
            }),
            serde_json::json!({
                "id": {"videoId": "def"},
                "snippet": {"title": "A Live", "channelTitle": "Chan",
                            "liveBroadcastContent": "live"}
            }),
        ];
        let mut dur = std::collections::HashMap::new();
        dur.insert("abc".to_string(), 253i64);
        let rows = build_video_rows(&items, &dur, &std::collections::HashMap::new());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].source_kind, "youtube_vod");
        assert!(!rows[0].is_live);
        assert_eq!(rows[0].duration_secs, 253);
        assert_eq!(rows[1].source_kind, "youtube_live");
        assert!(rows[1].is_live);
    }

    #[test]
    fn format_scheduled_start_cases() {
        assert_eq!(
            format_scheduled_start("2026-06-12T18:00:00Z"),
            "Jun 12 18:00 UTC"
        );
        // offset timestamps are normalized to UTC
        assert_eq!(
            format_scheduled_start("2026-06-12T20:30:00+02:00"),
            "Jun 12 18:30 UTC"
        );
        assert_eq!(format_scheduled_start("not-a-date"), "");
        assert_eq!(format_scheduled_start(""), "");
    }

    #[test]
    fn video_rows_mark_upcoming_as_live_source_with_schedule() {
        let items = vec![serde_json::json!({
            "id": {"videoId": "up1"},
            "snippet": {"title": "Launch", "channelTitle": "SpaceX",
                        "liveBroadcastContent": "upcoming",
                        "thumbnails": {"default": {"url": "https://i.ytimg.com/vi/up1/default.jpg"}}}
        })];
        let dur = std::collections::HashMap::new();
        let mut sched = std::collections::HashMap::new();
        sched.insert("up1".to_string(), "2026-06-12T18:00:00Z".to_string());
        let rows = build_video_rows(&items, &dur, &sched);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_upcoming);
        assert!(!rows[0].is_live);
        assert_eq!(rows[0].source_kind, "youtube_live");
        assert_eq!(rows[0].scheduled_start, "Jun 12 18:00 UTC");
        assert_eq!(
            rows[0].thumbnail_url,
            "https://i.ytimg.com/vi/up1/default.jpg"
        );
    }

    #[test]
    fn rows_without_thumbnails_get_empty_thumbnail_url() {
        let items = vec![serde_json::json!({
            "id": {"videoId": "abc"},
            "snippet": {"title": "A VOD", "channelTitle": "Chan",
                        "liveBroadcastContent": "none"}
        })];
        let dur = std::collections::HashMap::new();
        let sched = std::collections::HashMap::new();
        let rows = build_video_rows(&items, &dur, &sched);
        assert_eq!(rows[0].thumbnail_url, "");
        assert_eq!(rows[0].scheduled_start, "");
        assert!(!rows[0].is_upcoming);
    }
}

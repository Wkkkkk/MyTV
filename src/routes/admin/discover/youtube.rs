pub struct YoutubeResultRow {
    pub title: String,
    pub channel_title: String,
    pub is_live: bool,
    pub duration_secs: i64,
    pub url: String,
    pub source_kind: String,
    pub form_id: usize,
}

pub(super) fn build_video_rows(
    items: &[serde_json::Value],
    duration_map: &std::collections::HashMap<String, i64>,
) -> Vec<YoutubeResultRow> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let video_id = item["id"]["videoId"].as_str()?;
            let snippet = &item["snippet"];
            let title = snippet["title"].as_str().unwrap_or("Unknown").to_string();
            let channel_title = snippet["channelTitle"].as_str().unwrap_or("").to_string();
            let is_live = snippet["liveBroadcastContent"].as_str() == Some("live");
            let duration_secs = *duration_map.get(video_id).unwrap_or(&0);
            let source_kind = if is_live {
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
                duration_secs,
                url,
                source_kind,
                form_id: i,
            })
        })
        .collect()
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
            ("part", "contentDetails"),
            ("id", ids_joined.as_str()),
            ("key", api_key),
        ])
        .send()
        .await?
        .json()
        .await?;

    let mut duration_map = std::collections::HashMap::<String, i64>::new();
    if let Some(detail_items) = details_resp["items"].as_array() {
        for item in detail_items {
            let id = item["id"].as_str().unwrap_or("").to_string();
            let dur_str = item["contentDetails"]["duration"]
                .as_str()
                .unwrap_or("PT0S");
            duration_map.insert(id, parse_iso8601_duration(dur_str));
        }
    }

    let rows = build_video_rows(items, &duration_map);

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iso8601_duration() {
        assert_eq!(parse_iso8601_duration("PT4M13S"), 253);
        assert_eq!(parse_iso8601_duration("PT1H30M"), 5400);
        assert_eq!(parse_iso8601_duration("PT2H"), 7200);
        assert_eq!(parse_iso8601_duration("PT0S"), 0);
        assert_eq!(parse_iso8601_duration("PT45S"), 45);
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
        let rows = build_video_rows(&items, &dur);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].source_kind, "youtube_vod");
        assert!(!rows[0].is_live);
        assert_eq!(rows[0].duration_secs, 253);
        assert_eq!(rows[1].source_kind, "youtube_live");
        assert!(rows[1].is_live);
    }
}

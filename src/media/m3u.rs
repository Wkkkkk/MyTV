pub struct M3uChannel {
    pub name: String,
    pub group: String,
    pub country: String,
    pub url: String,
}

pub fn parse_m3u(input: &str) -> Vec<M3uChannel> {
    fn extract_attr(line: &str, attr: &str) -> String {
        let key = format!("{}=\"", attr);
        line.find(&key)
            .map(|start| {
                let after = &line[start + key.len()..];
                after
                    .find('"')
                    .map(|end| after[..end].to_string())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    let mut channels = Vec::new();
    let mut lines = input.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.starts_with("#EXTINF:") {
            continue;
        }
        let name = line
            .rfind(',')
            .map(|pos| line[pos + 1..].trim().to_string())
            .unwrap_or_default();
        let group = extract_attr(line, "group-title");
        let country = extract_attr(line, "country");
        let url = loop {
            match lines.peek() {
                Some(next) if next.trim().is_empty() => {
                    lines.next();
                }
                Some(next) if next.starts_with("#EXTINF:") => break String::new(),
                Some(next) if next.starts_with('#') => {
                    lines.next();
                }
                Some(next) => {
                    let u = next.trim().to_string();
                    lines.next();
                    break u;
                }
                None => break String::new(),
            }
        };
        if !url.is_empty() {
            channels.push(M3uChannel {
                name,
                group,
                country,
                url,
            });
        }
    }
    channels
}

pub fn filter_m3u<'a>(
    channels: &'a [M3uChannel],
    country: &str,
    group: &str,
) -> Vec<&'a M3uChannel> {
    let country_lower = country.trim().to_lowercase();
    let group_lower = group.trim().to_lowercase();
    channels
        .iter()
        .filter(|ch| {
            let country_ok =
                country_lower.is_empty() || ch.country.to_lowercase().contains(&country_lower);
            let group_ok = group_lower.is_empty() || ch.group.to_lowercase().contains(&group_lower);
            country_ok && group_ok
        })
        .take(50)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_m3u_single_channel() {
        let input = "#EXTM3U\n#EXTINF:-1 group-title=\"News\" country=\"US\",CNN\nhttps://example.com/cnn.m3u8\n";
        let result = parse_m3u(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "CNN");
        assert_eq!(result[0].group, "News");
        assert_eq!(result[0].country, "US");
        assert_eq!(result[0].url, "https://example.com/cnn.m3u8");
    }

    #[test]
    fn test_parse_m3u_missing_optional_attrs() {
        let input = "#EXTINF:-1,MyChannel\nhttps://example.com/stream.m3u8\n";
        let result = parse_m3u(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "MyChannel");
        assert_eq!(result[0].group, "");
        assert_eq!(result[0].country, "");
    }

    #[test]
    fn test_parse_m3u_skips_entry_without_url() {
        let input = "#EXTINF:-1,CNN\n#EXTINF:-1,ESPN\nhttps://espn.com/stream.m3u8\n";
        let result = parse_m3u(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ESPN");
    }

    #[test]
    fn test_parse_m3u_multiple_channels() {
        let input = concat!(
            "#EXTM3U\n",
            "#EXTINF:-1 group-title=\"News\" country=\"US\",CNN\nhttps://cnn.com/stream.m3u8\n",
            "#EXTINF:-1 group-title=\"Sports\" country=\"US\",ESPN\nhttps://espn.com/stream.m3u8\n",
        );
        let result = parse_m3u(input);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_parse_m3u_skips_extvlcopt_directive() {
        let input = concat!(
            "#EXTINF:-1 group-title=\"News\" country=\"US\",CNN\n",
            "#EXTVLCOPT:network-caching=1000\n",
            "https://cnn.com/stream.m3u8\n",
        );
        let result = parse_m3u(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "CNN");
        assert_eq!(result[0].url, "https://cnn.com/stream.m3u8");
    }

    #[test]
    fn test_filter_m3u_by_country_case_insensitive() {
        let channels = vec![
            M3uChannel {
                name: "CNN".into(),
                group: "News".into(),
                country: "US".into(),
                url: "https://a.com".into(),
            },
            M3uChannel {
                name: "BBC".into(),
                group: "News".into(),
                country: "UK".into(),
                url: "https://b.com".into(),
            },
        ];
        let result = filter_m3u(&channels, "us", "");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "CNN");
    }

    #[test]
    fn test_filter_m3u_by_group_case_insensitive() {
        let channels = vec![
            M3uChannel {
                name: "CNN".into(),
                group: "News".into(),
                country: "US".into(),
                url: "https://a.com".into(),
            },
            M3uChannel {
                name: "ESPN".into(),
                group: "Sports".into(),
                country: "US".into(),
                url: "https://b.com".into(),
            },
        ];
        let result = filter_m3u(&channels, "", "sports");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ESPN");
    }

    #[test]
    fn test_filter_m3u_both_filters_must_match() {
        let channels = vec![
            M3uChannel {
                name: "CNN".into(),
                group: "News".into(),
                country: "US".into(),
                url: "https://a.com".into(),
            },
            M3uChannel {
                name: "BBC".into(),
                group: "News".into(),
                country: "UK".into(),
                url: "https://b.com".into(),
            },
            M3uChannel {
                name: "ESPN".into(),
                group: "Sports".into(),
                country: "US".into(),
                url: "https://c.com".into(),
            },
        ];
        let result = filter_m3u(&channels, "US", "news");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "CNN");
    }

    #[test]
    fn test_filter_m3u_no_filter_capped_at_50() {
        let channels: Vec<M3uChannel> = (0..60)
            .map(|i| M3uChannel {
                name: format!("Ch{}", i),
                group: "Test".into(),
                country: "US".into(),
                url: format!("https://example.com/{}", i),
            })
            .collect();
        let result = filter_m3u(&channels, "", "");
        assert_eq!(result.len(), 50);
    }
}

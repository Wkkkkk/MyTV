use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::io::Cursor;
use std::time::Duration;

/// Rewrites a DASH MPD manifest:
/// - Relative <BaseURL> text → resolved to absolute (always, even when direct=true)
/// - Absolute <BaseURL> text → left unchanged
/// - Absolute <SegmentTemplate media/initialization> attrs → wrapped in /stream-proxy (unless direct=true)
/// - Absolute <SegmentURL media> attrs → wrapped in /stream-proxy (unless direct=true)
/// - Relative segment URLs → left unchanged (resolve against BaseURL on CDN)
pub fn rewrite_mpd_urls(xml: &str, base_url: &str, direct: bool) -> String {
    let mut reader = Reader::from_str(xml);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut in_base_url = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"BaseURL" => {
                    in_base_url = true;
                    writer.write_event(Event::Start(e)).unwrap();
                }
                b"SegmentTemplate" if !direct => {
                    let rewritten = rewrite_url_attrs(e, &[b"media", b"initialization"]);
                    writer.write_event(Event::Start(rewritten)).unwrap();
                }
                _ => {
                    writer.write_event(Event::Start(e)).unwrap();
                }
            },
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                b"SegmentTemplate" if !direct => {
                    let rewritten = rewrite_url_attrs(e, &[b"media", b"initialization"]);
                    writer.write_event(Event::Empty(rewritten)).unwrap();
                }
                b"SegmentURL" if !direct => {
                    let rewritten = rewrite_url_attrs(e, &[b"media"]);
                    writer.write_event(Event::Empty(rewritten)).unwrap();
                }
                _ => {
                    writer.write_event(Event::Empty(e)).unwrap();
                }
            },
            Ok(Event::Text(e)) => {
                if in_base_url {
                    let text = e.decode().unwrap_or_default();
                    let trimmed = text.trim();
                    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                        // Absolute BaseURL — leave unchanged
                        writer.write_event(Event::Text(e)).unwrap();
                    } else {
                        // Relative BaseURL — resolve to absolute
                        let resolved = resolve_relative_url(trimmed, base_url);
                        writer
                            .write_event(Event::Text(BytesText::new(&resolved)))
                            .unwrap();
                    }
                } else {
                    writer.write_event(Event::Text(e)).unwrap();
                }
            }
            Ok(Event::End(e)) => {
                if e.local_name().as_ref() == b"BaseURL" {
                    in_base_url = false;
                }
                writer.write_event(Event::End(e)).unwrap();
            }
            Ok(Event::Eof) => break,
            Ok(e) => {
                writer.write_event(e).unwrap();
            }
            Err(e) => {
                tracing::warn!(error = %e, "MPD XML parse error; rewrite may be truncated");
                break;
            }
        }
    }

    String::from_utf8(writer.into_inner().into_inner()).unwrap_or_else(|_| xml.to_string())
}

/// Like `pct_encode` but preserves `$` so DASH template variables
/// (`$Number$`, `$RepresentationID$`, etc.) survive proxy URL wrapping.
fn pct_encode_template(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'$' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

/// Resolves a URL against a base URL.
/// Absolute URLs are returned unchanged.
/// Relative URLs (including `./`) are combined with the base URL's directory.
fn resolve_relative_url(url: &str, base_url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    if url.starts_with('/') {
        let after_scheme = base_url.find("://").map(|i| i + 3).unwrap_or(0);
        let host_len = base_url[after_scheme..]
            .find('/')
            .unwrap_or(base_url[after_scheme..].len());
        let origin = &base_url[..after_scheme + host_len];
        return format!("{}{}", origin, url);
    }
    let base_dir = base_url
        .rsplit_once('/')
        .map(|(b, _)| b)
        .unwrap_or(base_url);
    let stripped = url.trim_start_matches("./");
    if stripped.is_empty() {
        format!("{}/", base_dir)
    } else {
        format!("{}/{}", base_dir, stripped)
    }
}

/// Rewrites named URL attributes on a start/empty element.
/// Absolute HTTP(S) URLs are wrapped in `/stream-proxy?url=…` using
/// `pct_encode_template` (which preserves `$` for DASH template variables).
/// Relative URLs and non-URL attributes are passed through unchanged.
fn rewrite_url_attrs(e: BytesStart<'_>, url_attr_names: &[&[u8]]) -> BytesStart<'static> {
    let name = std::str::from_utf8(e.name().as_ref())
        .unwrap_or_default()
        .to_owned();
    let mut new = BytesStart::new(name);
    for attr in e.attributes().flatten() {
        let key = attr.key.as_ref().to_owned();
        let val = attr
            // XML 1.0 — all real-world MPDs are XML 1.0
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .unwrap_or_default()
            .into_owned();
        let new_val = if url_attr_names.contains(&key.as_slice())
            && (val.starts_with("http://") || val.starts_with("https://"))
        {
            format!("/stream-proxy?url={}", pct_encode_template(&val))
        } else {
            val
        };
        new.push_attribute((key.as_slice(), new_val.as_bytes()));
    }
    new
}

/// Extracts `mediaPresentationDuration` from an MPD XML string and returns seconds.
/// Returns an error for live/dynamic streams where the attribute is absent.
pub fn parse_mpd_duration(xml: &str) -> anyhow::Result<i64> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e))
                if e.local_name().as_ref() == b"MPD" =>
            {
                for attr in e.attributes().flatten() {
                    if attr.key.as_ref() == b"mediaPresentationDuration" {
                        let val = String::from_utf8_lossy(&attr.value);
                        return parse_iso8601_duration_secs(&val);
                    }
                }
                anyhow::bail!("MPD has no mediaPresentationDuration (live stream?)");
            }
            Ok(Event::Eof) => break,
            Err(e) => anyhow::bail!("MPD XML parse error: {e}"),
            _ => {}
        }
    }
    anyhow::bail!("MPD root element not found");
}

/// Converts an ISO 8601 duration string (e.g. `PT1H30M5.5S`) to whole seconds.
fn parse_iso8601_duration_secs(s: &str) -> anyhow::Result<i64> {
    let s = s
        .strip_prefix('P')
        .ok_or_else(|| anyhow::anyhow!("not an ISO 8601 duration: {s}"))?;
    let (date_part, time_part) = s.split_once('T').unwrap_or((s, ""));
    let mut secs = 0i64;
    if let Some(idx) = date_part.find('D') {
        let v: f64 = date_part[..idx].parse()?;
        secs += (v * 86400.0) as i64;
    }
    let mut rest = time_part;
    if let Some(idx) = rest.find('H') {
        let v: f64 = rest[..idx].parse()?;
        secs += (v * 3600.0) as i64;
        rest = &rest[idx + 1..];
    }
    if let Some(idx) = rest.find('M') {
        let v: f64 = rest[..idx].parse()?;
        secs += (v * 60.0) as i64;
        rest = &rest[idx + 1..];
    }
    if let Some(idx) = rest.find('S') {
        let v: f64 = rest[..idx].parse()?;
        secs += v as i64;
    }
    Ok(secs)
}

/// Returns the best HTTPS URL to HEAD-probe for CORS on a DASH MPD.
/// Priority: absolute/resolved <BaseURL> > absolute <SegmentURL media> >
///   absolute <SegmentTemplate initialization> (no template vars) > MPD directory.
pub fn find_mpd_probe_url(xml: &str, mpd_url: &str) -> String {
    let mut reader = Reader::from_str(xml);
    let mut in_base_url = false;
    let mut fallback: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                if e.local_name().as_ref() == b"BaseURL" {
                    in_base_url = true;
                }
            }
            Ok(Event::Empty(ref e)) if fallback.is_none() => match e.local_name().as_ref() {
                b"SegmentURL" => {
                    if let Some(url) = attr_value(e, b"media") {
                        if url.starts_with("https://") {
                            fallback = Some(url);
                        }
                    }
                }
                b"SegmentTemplate" => {
                    if let Some(url) = attr_value(e, b"initialization") {
                        if url.starts_with("https://") && !url.contains('$') {
                            fallback = Some(url);
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Text(ref e)) if in_base_url => {
                let text = e.decode().unwrap_or_default();
                let resolved = resolve_relative_url(text.trim(), mpd_url);
                if resolved.starts_with("https://") {
                    return resolved;
                }
            }
            Ok(Event::End(ref e)) => {
                if e.local_name().as_ref() == b"BaseURL" {
                    in_base_url = false;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    fallback.unwrap_or_else(|| {
        mpd_url
            .rsplit_once('/')
            .map(|(b, _)| format!("{}/", b))
            .unwrap_or_else(|| mpd_url.to_string())
    })
}

fn attr_value(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.as_ref() == name {
            Some(String::from_utf8_lossy(&a.value).into_owned())
        } else {
            None
        }
    })
}

/// Fetches an MPD and HEAD-probes its effective segment origin for `Access-Control-Allow-Origin: *`.
/// Returns `Some(true)` if direct load is possible, `Some(false)` if proxy is needed,
/// `None` if the MPD could not be fetched.
pub async fn probe_mpd_cors(client: &reqwest::Client, mpd_url: &str) -> Option<bool> {
    let body = client
        .get(mpd_url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let probe_url = find_mpd_probe_url(&body, mpd_url);
    if probe_url.starts_with("http://") {
        return Some(false);
    }
    Some(crate::media::hls::probe_cors(client, &probe_url).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_base_url_resolved_to_absolute() {
        let xml = r#"<?xml version="1.0"?><MPD><BaseURL>./</BaseURL></MPD>"#;
        let out = rewrite_mpd_urls(xml, "https://origin.example.com/path/stream.mpd", false);
        assert!(
            out.contains("<BaseURL>https://origin.example.com/path/</BaseURL>"),
            "got: {out}"
        );
    }

    #[test]
    fn absolute_base_url_left_unchanged() {
        let xml = r#"<?xml version="1.0"?><MPD><BaseURL>https://cdn.example.com/</BaseURL></MPD>"#;
        let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
        assert!(
            out.contains("<BaseURL>https://cdn.example.com/</BaseURL>"),
            "got: {out}"
        );
        assert!(
            !out.contains("/stream-proxy"),
            "absolute BaseURL must not be proxied"
        );
    }

    #[test]
    fn rewrite_segment_template_media_absolute() {
        let xml = r#"<?xml version="1.0"?><MPD><SegmentTemplate media="https://cdn.example.com/video/$RepresentationID$/seg-$Number$.m4s" initialization="https://cdn.example.com/video/$RepresentationID$/init.mp4"/></MPD>"#;
        let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
        assert!(
            out.contains("media=\"/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fvideo%2F$RepresentationID$%2Fseg-$Number$.m4s\""),
            "got: {out}"
        );
        assert!(
            out.contains("initialization=\"/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fvideo%2F$RepresentationID$%2Finit.mp4\""),
            "got: {out}"
        );
    }

    #[test]
    fn relative_segment_template_left_unchanged() {
        let xml = r#"<?xml version="1.0"?><MPD><SegmentTemplate media="video/$RepresentationID$/seg-$Number$.m4s"/></MPD>"#;
        let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
        assert!(
            out.contains(r#"media="video/$RepresentationID$/seg-$Number$.m4s""#),
            "got: {out}"
        );
        assert!(
            !out.contains("/stream-proxy"),
            "relative template must not be proxied"
        );
    }

    #[test]
    fn rewrite_segment_url_media_absolute() {
        let xml = r#"<?xml version="1.0"?><MPD><SegmentList><SegmentURL media="https://cdn.example.com/video/seg-1.m4s"/></SegmentList></MPD>"#;
        let out = rewrite_mpd_urls(xml, "https://origin.example.com/stream.mpd", false);
        assert!(
            out.contains(
                "media=\"/stream-proxy?url=https%3A%2F%2Fcdn.example.com%2Fvideo%2Fseg-1.m4s\""
            ),
            "got: {out}"
        );
    }

    #[test]
    fn direct_mode_does_not_proxy_segments_but_still_resolves_base_url() {
        let xml = r#"<?xml version="1.0"?><MPD><BaseURL>./</BaseURL><SegmentTemplate media="https://cdn.example.com/seg-$Number$.m4s"/></MPD>"#;
        let out = rewrite_mpd_urls(xml, "https://origin.example.com/path/stream.mpd", true);
        assert!(
            out.contains("<BaseURL>https://origin.example.com/path/</BaseURL>"),
            "got: {out}"
        );
        assert!(
            !out.contains("/stream-proxy"),
            "direct mode must not proxy segment URLs"
        );
    }

    #[test]
    fn bbb_fixture_resolves_relative_base_url() {
        let xml = include_str!("../../tests/fixtures/bbb_30fps.mpd");
        let out = rewrite_mpd_urls(
            xml,
            "https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd",
            false,
        );
        assert!(
            out.contains("<BaseURL>https://dash.akamaized.net/akamai/bbb_30fps/</BaseURL>"),
            "got BaseURL section: {}",
            &out[..out.find("</BaseURL>").unwrap_or(200) + 10]
        );
        assert!(
            !out.contains("/stream-proxy?url="),
            "relative templates must not be proxied"
        );
        assert!(out.contains("<MPD"));
    }

    #[test]
    fn parse_mpd_duration_seconds_only() {
        let xml = r#"<?xml version="1.0"?><MPD mediaPresentationDuration="PT634.566S"></MPD>"#;
        assert_eq!(parse_mpd_duration(xml).unwrap(), 634);
    }

    #[test]
    fn parse_mpd_duration_hours_minutes_seconds() {
        let xml = r#"<?xml version="1.0"?><MPD mediaPresentationDuration="PT1H30M5S"></MPD>"#;
        assert_eq!(parse_mpd_duration(xml).unwrap(), 5405);
    }

    #[test]
    fn parse_mpd_duration_missing_returns_error() {
        let xml = r#"<?xml version="1.0"?><MPD type="dynamic"></MPD>"#;
        assert!(parse_mpd_duration(xml).is_err());
    }

    #[test]
    fn bbb_fixture_duration() {
        let xml = include_str!("../../tests/fixtures/bbb_30fps.mpd");
        assert_eq!(parse_mpd_duration(xml).unwrap(), 634);
    }

    #[test]
    fn find_mpd_probe_url_absolute_base_url() {
        let xml =
            r#"<?xml version="1.0"?><MPD><BaseURL>https://cdn.example.com/live/</BaseURL></MPD>"#;
        let url = find_mpd_probe_url(xml, "https://origin.example.com/stream.mpd");
        assert_eq!(url, "https://cdn.example.com/live/");
    }

    #[test]
    fn find_mpd_probe_url_relative_base_url_resolved() {
        let xml = r#"<?xml version="1.0"?><MPD><BaseURL>./</BaseURL></MPD>"#;
        let url = find_mpd_probe_url(xml, "https://origin.example.com/path/stream.mpd");
        assert_eq!(url, "https://origin.example.com/path/");
    }

    #[test]
    fn find_mpd_probe_url_segment_url_media() {
        let xml = r#"<?xml version="1.0"?><MPD><SegmentList><SegmentURL media="https://cdn.example.com/seg-1.m4s"/></SegmentList></MPD>"#;
        let url = find_mpd_probe_url(xml, "https://origin.example.com/stream.mpd");
        assert_eq!(url, "https://cdn.example.com/seg-1.m4s");
    }

    #[test]
    fn find_mpd_probe_url_segment_template_initialization() {
        let xml = r#"<?xml version="1.0"?><MPD><SegmentTemplate initialization="https://cdn.example.com/init.mp4" media="https://cdn.example.com/seg-$Number$.m4s"/></MPD>"#;
        let url = find_mpd_probe_url(xml, "https://origin.example.com/stream.mpd");
        assert_eq!(url, "https://cdn.example.com/init.mp4");
    }

    #[test]
    fn find_mpd_probe_url_segment_template_with_only_template_vars_falls_back() {
        let xml = r#"<?xml version="1.0"?><MPD><SegmentTemplate media="https://cdn.example.com/seg-$Number$.m4s" initialization="https://cdn.example.com/$RepresentationID$/init.mp4"/></MPD>"#;
        let url = find_mpd_probe_url(xml, "https://origin.example.com/stream.mpd");
        // initialization has $RepresentationID$ → skip; fallback to MPD directory
        assert_eq!(url, "https://origin.example.com/");
    }

    #[test]
    fn find_mpd_probe_url_no_hints_falls_back_to_mpd_directory() {
        let xml = r#"<?xml version="1.0"?><MPD><SegmentTemplate media="seg-$Number$.m4s"/></MPD>"#;
        let url = find_mpd_probe_url(xml, "https://origin.example.com/path/stream.mpd");
        assert_eq!(url, "https://origin.example.com/path/");
    }

    #[test]
    fn find_mpd_probe_url_bbb_fixture_resolves_to_mpd_directory() {
        let xml = include_str!("../../tests/fixtures/bbb_30fps.mpd");
        // BBB has <BaseURL>./</BaseURL> which resolves to the MPD directory
        let url = find_mpd_probe_url(
            xml,
            "https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd",
        );
        assert_eq!(url, "https://dash.akamaized.net/akamai/bbb_30fps/");
    }
}

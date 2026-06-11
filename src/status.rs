use crate::media::resolver::LiveStatus;

/// The single unified status of a source or playlist item. Replaces the separate
/// Active / Health / Live indicators. `Down` carries the failure reason; `Upcoming`
/// carries the scheduled-start unix timestamp when known.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceStatus {
    Disabled,
    Down(Option<String>),
    Live,
    Ok,
    Upcoming(Option<i64>),
    Recorded,
    Offline,
    Unchecked,
}

/// Rendering parts for a status. `class` + `color` cover both surfaces: the guide
/// uses an inline `color`, the admin rows use `color` for the inline glyph, and
/// `class` is available for future CSS. `title` is the hover tooltip.
pub struct StatusBadge {
    pub class: &'static str,
    pub color: &'static str,
    pub glyph: &'static str,
    pub label: &'static str,
    pub title: String,
}

/// Computes the unified status. `live` is the cached `LiveStatus` for a
/// `youtube_live` source (pass `None` for other kinds, or when the live cache is
/// cold). Precedence: Disabled (manual intent) first; then for youtube_live the
/// live status; otherwise the persisted health (`last_status`).
///
/// Display `Down` = any `last_status == "error"`; the auto-disable threshold is
/// NOT consulted here (it governs only the tune-skip query).
pub fn compute(
    is_active: bool,
    kind: &str,
    last_status: Option<&str>,
    failure_reason: Option<&str>,
    live: Option<LiveStatus>,
) -> SourceStatus {
    if !is_active {
        return SourceStatus::Disabled;
    }
    if kind == "youtube_live" {
        return match live {
            Some(LiveStatus::Live) => SourceStatus::Live,
            Some(LiveStatus::Upcoming(ts)) => SourceStatus::Upcoming(ts),
            Some(LiveStatus::WasLive) | Some(LiveStatus::PostLive) => SourceStatus::Recorded,
            Some(LiveStatus::Offline) | Some(LiveStatus::NotLive) => SourceStatus::Offline,
            Some(LiveStatus::Unknown) | None => SourceStatus::Unchecked,
        };
    }
    match last_status {
        Some("error") => SourceStatus::Down(failure_reason.map(|s| s.to_string())),
        Some("ok") => SourceStatus::Ok,
        _ => SourceStatus::Unchecked,
    }
}

/// Maps a status to its renderable parts.
pub fn status_badge(s: &SourceStatus) -> StatusBadge {
    match s {
        SourceStatus::Disabled => StatusBadge {
            class: "status-disabled",
            color: "#888",
            glyph: "⏸",
            label: "disabled",
            title: "Manually disabled".to_string(),
        },
        SourceStatus::Down(reason) => StatusBadge {
            class: "status-down",
            color: "#e94560",
            glyph: "✕",
            label: "down",
            title: match reason {
                Some(r) => format!("Down — {r}"),
                None => "Last check failed".to_string(),
            },
        },
        SourceStatus::Live => StatusBadge {
            class: "status-live",
            color: "#4caf50",
            glyph: "●",
            label: "live",
            title: "Currently live".to_string(),
        },
        SourceStatus::Ok => StatusBadge {
            class: "status-ok",
            color: "#4caf50",
            glyph: "●",
            label: "ok",
            title: "Reachable".to_string(),
        },
        SourceStatus::Upcoming(ts) => {
            let ts = *ts; // Option<i64> is Copy — copy out of the &SourceStatus borrow
            let title = ts
                .filter(|t| *t > 0)
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                .map(|dt| format!("Scheduled — starts {}", crate::media::format_utc_short(dt)))
                .unwrap_or_else(|| "Scheduled, start time unknown".to_string());
            StatusBadge {
                class: "status-upcoming",
                color: "#db4",
                glyph: "◷",
                label: "upcoming",
                title,
            }
        }
        SourceStatus::Recorded => StatusBadge {
            class: "status-recorded",
            color: "#88f",
            glyph: "⏺",
            label: "recorded",
            title: "Finished broadcast — next tune converts the channel to VOD".to_string(),
        },
        SourceStatus::Offline => StatusBadge {
            class: "status-offline",
            color: "#888",
            glyph: "○",
            label: "offline",
            title: "Not currently live".to_string(),
        },
        SourceStatus::Unchecked => StatusBadge {
            class: "status-unchecked",
            color: "#666",
            glyph: "·",
            label: "?",
            title: "Not yet checked".to_string(),
        },
    }
}

/// Optimism rank, lower = better (more optimistic). Used to aggregate a channel's
/// per-source statuses into one guide badge.
pub fn rank(s: &SourceStatus) -> u8 {
    match s {
        SourceStatus::Live | SourceStatus::Ok => 0,
        SourceStatus::Upcoming(_) => 1,
        SourceStatus::Recorded => 2,
        SourceStatus::Offline => 3,
        SourceStatus::Unchecked => 4,
        SourceStatus::Down(_) => 5,
        SourceStatus::Disabled => 6,
    }
}

/// The most-optimistic (best-case) status across an iterator. Empty → Unchecked.
pub fn most_optimistic<I: IntoIterator<Item = SourceStatus>>(statuses: I) -> SourceStatus {
    statuses
        .into_iter()
        .min_by_key(rank)
        .unwrap_or(SourceStatus::Unchecked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_wins_regardless_of_health_or_live() {
        assert_eq!(
            compute(false, "hls", Some("ok"), None, None),
            SourceStatus::Disabled
        );
        assert_eq!(
            compute(false, "youtube_live", None, None, Some(LiveStatus::Live)),
            SourceStatus::Disabled
        );
    }

    #[test]
    fn regular_source_maps_health() {
        assert_eq!(
            compute(true, "hls", Some("ok"), None, None),
            SourceStatus::Ok
        );
        assert_eq!(
            compute(true, "hls", Some("error"), Some("timeout"), None),
            SourceStatus::Down(Some("timeout".to_string()))
        );
        assert_eq!(
            compute(true, "hls", None, None, None),
            SourceStatus::Unchecked
        );
    }

    #[test]
    fn youtube_live_maps_live_status_not_health() {
        // Offline recorded as last_status='error' must still show Offline, never Down.
        assert_eq!(
            compute(
                true,
                "youtube_live",
                Some("error"),
                Some("not currently live"),
                Some(LiveStatus::Offline)
            ),
            SourceStatus::Offline
        );
        assert_eq!(
            compute(true, "youtube_live", None, None, Some(LiveStatus::Live)),
            SourceStatus::Live
        );
        assert_eq!(
            compute(
                true,
                "youtube_live",
                None,
                None,
                Some(LiveStatus::Upcoming(Some(123)))
            ),
            SourceStatus::Upcoming(Some(123))
        );
        assert_eq!(
            compute(true, "youtube_live", None, None, Some(LiveStatus::WasLive)),
            SourceStatus::Recorded
        );
        assert_eq!(
            compute(true, "youtube_live", None, None, Some(LiveStatus::PostLive)),
            SourceStatus::Recorded
        );
        // Cold cache → Unchecked, never Down.
        assert_eq!(
            compute(true, "youtube_live", Some("error"), None, None),
            SourceStatus::Unchecked
        );
    }

    #[test]
    fn badge_glyphs_and_colors() {
        assert_eq!(status_badge(&SourceStatus::Live).glyph, "●");
        assert_eq!(status_badge(&SourceStatus::Live).color, "#4caf50");
        assert_eq!(status_badge(&SourceStatus::Disabled).glyph, "⏸");
        assert_eq!(status_badge(&SourceStatus::Down(None)).glyph, "✕");
        assert_eq!(
            status_badge(&SourceStatus::Down(Some("boom".into()))).title,
            "Down — boom"
        );
        assert_eq!(status_badge(&SourceStatus::Offline).glyph, "○");
        assert_eq!(
            status_badge(&SourceStatus::Upcoming(None)).title,
            "Scheduled, start time unknown"
        );
    }

    #[test]
    fn most_optimistic_picks_best_case() {
        assert_eq!(
            most_optimistic([
                SourceStatus::Down(None),
                SourceStatus::Live,
                SourceStatus::Disabled
            ]),
            SourceStatus::Live
        );
        assert_eq!(
            most_optimistic([SourceStatus::Down(None), SourceStatus::Disabled]),
            SourceStatus::Down(None)
        );
        assert_eq!(
            most_optimistic([SourceStatus::Disabled]),
            SourceStatus::Disabled
        );
        assert_eq!(most_optimistic(std::iter::empty()), SourceStatus::Unchecked);
    }
}

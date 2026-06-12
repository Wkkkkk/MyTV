use clap::{Parser, Subcommand};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(name = "mytvctl", about = "MyTV admin CLI (talks to /api/admin)")]
struct Cli {
    /// Base URL (else $MYTV_BASE_URL, else http://localhost:3000)
    #[arg(long, global = true)]
    base_url: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(subcommand)]
    Channel(ChannelCmd),
    #[command(subcommand)]
    Source(SourceCmd),
    #[command(subcommand)]
    Playlist(PlaylistCmd),
    #[command(subcommand)]
    Discover(DiscoverCmd),
}

#[derive(Subcommand)]
enum ChannelCmd {
    List,
    Get {
        id: i64,
    },
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        category: String,
        #[arg(long = "type")]
        r#type: String,
        #[arg(long)]
        logo_url: Option<String>,
        #[arg(long, default_value_t = 0)]
        sort_order: i64,
        #[arg(long)]
        loop_anchor: Option<String>,
    },
    Update {
        id: i64,
        #[arg(long)]
        name: String,
        #[arg(long)]
        category: String,
        #[arg(long = "type")]
        r#type: String,
        #[arg(long)]
        logo_url: Option<String>,
        #[arg(long, default_value_t = 0)]
        sort_order: i64,
        #[arg(long)]
        loop_anchor: Option<String>,
    },
    Delete {
        id: i64,
    },
}

#[derive(Subcommand)]
enum SourceCmd {
    List {
        #[arg(long)]
        channel: i64,
    },
    Get {
        id: i64,
    },
    Create {
        #[arg(long)]
        channel: i64,
        #[arg(long)]
        url: String,
        #[arg(long)]
        priority: Option<i64>,
        #[arg(long)]
        kind: Option<String>,
    },
    Update {
        id: i64,
        #[arg(long)]
        url: String,
        #[arg(long, default_value_t = 1)]
        priority: i64,
    },
    Delete {
        id: i64,
    },
    Toggle {
        id: i64,
        #[arg(long, action = clap::ArgAction::Set)]
        active: bool,
    },
    Test {
        id: i64,
    },
}

#[derive(Subcommand)]
enum PlaylistCmd {
    List {
        #[arg(long)]
        channel: i64,
    },
    Get {
        id: i64,
    },
    Create {
        #[arg(long)]
        channel: i64,
        #[arg(long)]
        title: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        duration_secs: i64,
        #[arg(long)]
        sort_order: Option<i64>,
    },
    Update {
        id: i64,
        #[arg(long)]
        title: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        duration_secs: i64,
        #[arg(long, default_value_t = 0)]
        sort_order: i64,
    },
    Delete {
        id: i64,
    },
    Toggle {
        id: i64,
        #[arg(long, action = clap::ArgAction::Set)]
        active: bool,
    },
    Test {
        id: i64,
    },
}

#[derive(Subcommand)]
enum DiscoverCmd {
    M3u {
        #[arg(long, default_value = "")]
        country: String,
        #[arg(long, default_value = "")]
        group: String,
        #[arg(long)]
        limit: Option<usize>,
    },
    Youtube {
        #[arg(long)]
        keyword: String,
        #[arg(long = "type")]
        r#type: Option<String>,
    },
    Resolve {
        #[arg(long)]
        url: String,
    },
    Channel {
        #[arg(long)]
        url: String,
    },
    Add {
        #[arg(long)]
        url: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        source_kind: String,
        #[arg(long)]
        duration_secs: Option<i64>,
        #[arg(long, conflicts_with_all = ["new_name", "new_category", "new_type"])]
        channel: Option<i64>,
        #[arg(long, requires_all = ["new_category", "new_type"])]
        new_name: Option<String>,
        #[arg(long)]
        new_category: Option<String>,
        #[arg(long)]
        new_type: Option<String>,
    },
}

/// A resolved HTTP request to make against the API.
struct ApiRequest {
    method: &'static str,
    path: String,
    body: Option<Value>,
}

fn resolve_base_url(flag: Option<String>, env: Option<String>) -> String {
    flag.or(env)
        .unwrap_or_else(|| "http://localhost:3000".to_string())
}

fn exit_code_for_status(status: u16) -> i32 {
    if (200..300).contains(&status) {
        0
    } else {
        1
    }
}

/// Pure mapping from a parsed command to an HTTP request spec. No I/O.
fn request_for(cmd: &Command) -> ApiRequest {
    match cmd {
        Command::Channel(c) => match c {
            ChannelCmd::List => ApiRequest {
                method: "GET",
                path: "/api/admin/channels".into(),
                body: None,
            },
            ChannelCmd::Get { id } => ApiRequest {
                method: "GET",
                path: format!("/api/admin/channels/{id}"),
                body: None,
            },
            ChannelCmd::Create {
                name,
                category,
                r#type,
                logo_url,
                sort_order,
                loop_anchor,
            } => ApiRequest {
                method: "POST",
                path: "/api/admin/channels".into(),
                body: Some(
                    json!({ "name": name, "category": category, "type": r#type, "logo_url": logo_url, "sort_order": sort_order, "loop_anchor": loop_anchor }),
                ),
            },
            ChannelCmd::Update {
                id,
                name,
                category,
                r#type,
                logo_url,
                sort_order,
                loop_anchor,
            } => ApiRequest {
                method: "PATCH",
                path: format!("/api/admin/channels/{id}"),
                body: Some(
                    json!({ "name": name, "category": category, "type": r#type, "logo_url": logo_url, "sort_order": sort_order, "loop_anchor": loop_anchor }),
                ),
            },
            ChannelCmd::Delete { id } => ApiRequest {
                method: "DELETE",
                path: format!("/api/admin/channels/{id}"),
                body: None,
            },
        },
        Command::Source(c) => match c {
            SourceCmd::List { channel } => ApiRequest {
                method: "GET",
                path: format!("/api/admin/channels/{channel}/sources"),
                body: None,
            },
            SourceCmd::Get { id } => ApiRequest {
                method: "GET",
                path: format!("/api/admin/sources/{id}"),
                body: None,
            },
            SourceCmd::Create {
                channel,
                url,
                priority,
                kind,
            } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/channels/{channel}/sources"),
                body: Some(json!({ "url": url, "priority": priority, "kind": kind })),
            },
            SourceCmd::Update { id, url, priority } => ApiRequest {
                method: "PATCH",
                path: format!("/api/admin/sources/{id}"),
                body: Some(json!({ "url": url, "priority": priority })),
            },
            SourceCmd::Delete { id } => ApiRequest {
                method: "DELETE",
                path: format!("/api/admin/sources/{id}"),
                body: None,
            },
            SourceCmd::Toggle { id, active } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/sources/{id}/toggle"),
                body: Some(json!({ "active": active })),
            },
            SourceCmd::Test { id } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/sources/{id}/test"),
                body: None,
            },
        },
        Command::Playlist(c) => match c {
            PlaylistCmd::List { channel } => ApiRequest {
                method: "GET",
                path: format!("/api/admin/channels/{channel}/playlist"),
                body: None,
            },
            PlaylistCmd::Get { id } => ApiRequest {
                method: "GET",
                path: format!("/api/admin/playlist/{id}"),
                body: None,
            },
            PlaylistCmd::Create {
                channel,
                title,
                url,
                duration_secs,
                sort_order,
            } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/channels/{channel}/playlist"),
                body: Some(
                    json!({ "title": title, "url": url, "duration_secs": duration_secs, "sort_order": sort_order }),
                ),
            },
            PlaylistCmd::Update {
                id,
                title,
                url,
                duration_secs,
                sort_order,
            } => ApiRequest {
                method: "PATCH",
                path: format!("/api/admin/playlist/{id}"),
                body: Some(
                    json!({ "title": title, "url": url, "duration_secs": duration_secs, "sort_order": sort_order }),
                ),
            },
            PlaylistCmd::Delete { id } => ApiRequest {
                method: "DELETE",
                path: format!("/api/admin/playlist/{id}"),
                body: None,
            },
            PlaylistCmd::Toggle { id, active } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/playlist/{id}/toggle"),
                body: Some(json!({ "active": active })),
            },
            PlaylistCmd::Test { id } => ApiRequest {
                method: "POST",
                path: format!("/api/admin/playlist/{id}/test"),
                body: None,
            },
        },
        Command::Discover(c) => match c {
            DiscoverCmd::M3u {
                country,
                group,
                limit,
            } => {
                let mut qs = format!("country={country}&group={group}");
                if let Some(l) = limit {
                    qs.push_str(&format!("&limit={l}"));
                }
                ApiRequest {
                    method: "GET",
                    path: format!("/api/admin/discover/m3u?{qs}"),
                    body: None,
                }
            }
            DiscoverCmd::Youtube { keyword, r#type } => {
                let mut qs = format!("keyword={keyword}");
                if let Some(t) = r#type {
                    qs.push_str(&format!("&type={t}"));
                }
                ApiRequest {
                    method: "GET",
                    path: format!("/api/admin/discover/youtube?{qs}"),
                    body: None,
                }
            }
            DiscoverCmd::Resolve { url } => ApiRequest {
                method: "POST",
                path: "/api/admin/discover/resolve".into(),
                body: Some(json!({ "url": url })),
            },
            DiscoverCmd::Channel { url } => ApiRequest {
                method: "POST",
                path: "/api/admin/discover/channel".into(),
                body: Some(json!({ "url": url })),
            },
            DiscoverCmd::Add {
                url,
                title,
                source_kind,
                duration_secs,
                channel,
                new_name,
                new_category,
                new_type,
            } => {
                let channel_val = if let Some(id) = channel {
                    json!({ "existing_id": id })
                } else {
                    json!({ "new": {
                        "name": new_name, "category": new_category, "type": new_type
                    }})
                };
                let mut body = json!({
                    "url": url, "title": title, "source_kind": source_kind, "channel": channel_val
                });
                if let Some(d) = duration_secs {
                    body["duration_secs"] = json!(d);
                }
                ApiRequest {
                    method: "POST",
                    path: "/api/admin/discover/add".into(),
                    body: Some(body),
                }
            }
        },
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let password = match std::env::var("MYTV_ADMIN_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            eprintln!("error: set MYTV_ADMIN_PASSWORD");
            std::process::exit(2);
        }
    };
    let base_url = resolve_base_url(cli.base_url.clone(), std::env::var("MYTV_BASE_URL").ok());

    let req = request_for(&cli.command);
    let client = reqwest::Client::new();
    let url = format!("{}{}", base_url.trim_end_matches('/'), req.path);
    let method = reqwest::Method::from_bytes(req.method.as_bytes()).unwrap();

    let mut builder = client
        .request(method, &url)
        .basic_auth("user", Some(&password));
    if let Some(body) = req.body {
        builder = builder.json(&body);
    }

    match builder.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            if !text.is_empty() {
                println!("{text}");
            }
            std::process::exit(exit_code_for_status(status));
        }
        Err(e) => {
            eprintln!("request failed: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_prefers_flag_then_env_then_default() {
        assert_eq!(
            resolve_base_url(Some("http://flag".into()), Some("http://env".into())),
            "http://flag"
        );
        assert_eq!(
            resolve_base_url(None, Some("http://env".into())),
            "http://env"
        );
        assert_eq!(resolve_base_url(None, None), "http://localhost:3000");
    }

    #[test]
    fn exit_code_maps_2xx_to_zero_else_one() {
        assert_eq!(exit_code_for_status(200), 0);
        assert_eq!(exit_code_for_status(201), 0);
        assert_eq!(exit_code_for_status(404), 1);
        assert_eq!(exit_code_for_status(500), 1);
    }

    #[test]
    fn request_for_channel_create_builds_post() {
        let cmd = Command::Channel(ChannelCmd::Create {
            name: "N".into(),
            category: "C".into(),
            r#type: "live".into(),
            logo_url: None,
            sort_order: 3,
            loop_anchor: None,
        });
        let req = request_for(&cmd);
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/channels");
        let body = req.body.expect("has body");
        assert_eq!(body["name"], "N");
        assert_eq!(body["type"], "live");
        assert_eq!(body["sort_order"], 3);
    }

    #[test]
    fn request_for_channel_list_is_get_no_body() {
        let req = request_for(&Command::Channel(ChannelCmd::List));
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/api/admin/channels");
        assert!(req.body.is_none());
    }

    #[test]
    fn request_for_source_toggle_builds_body() {
        let req = request_for(&Command::Source(SourceCmd::Toggle {
            id: 4,
            active: false,
        }));
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/sources/4/toggle");
        assert_eq!(req.body.unwrap()["active"], false);
    }

    #[test]
    fn request_for_discover_resolve_posts_url() {
        let req = request_for(&Command::Discover(DiscoverCmd::Resolve {
            url: "https://x/y.m3u8".into(),
        }));
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/discover/resolve");
        assert_eq!(req.body.unwrap()["url"], "https://x/y.m3u8");
    }

    #[test]
    fn request_for_discover_m3u_builds_get_query() {
        let req = request_for(&Command::Discover(DiscoverCmd::M3u {
            country: "us".into(),
            group: "News".into(),
            limit: Some(10),
        }));
        assert_eq!(req.method, "GET");
        assert_eq!(
            req.path,
            "/api/admin/discover/m3u?country=us&group=News&limit=10"
        );
        assert!(req.body.is_none());
    }

    #[test]
    fn request_for_discover_add_existing_channel() {
        let req = request_for(&Command::Discover(DiscoverCmd::Add {
            url: "https://x/y.m3u8".into(),
            title: "T".into(),
            source_kind: "hls".into(),
            duration_secs: None,
            channel: Some(1),
            new_name: None,
            new_category: None,
            new_type: None,
        }));
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/admin/discover/add");
        let body = req.body.unwrap();
        assert_eq!(body["channel"]["existing_id"], 1);
        assert_eq!(body["source_kind"], "hls");
    }

    #[test]
    fn request_for_discover_add_new_channel() {
        let req = request_for(&Command::Discover(DiscoverCmd::Add {
            url: "https://x/y.m3u8".into(),
            title: "T".into(),
            source_kind: "hls".into(),
            duration_secs: None,
            channel: None,
            new_name: Some("NC".into()),
            new_category: Some("test".into()),
            new_type: Some("live".into()),
        }));
        let body = req.body.unwrap();
        assert_eq!(body["channel"]["new"]["name"], "NC");
        assert_eq!(body["channel"]["new"]["type"], "live");
    }
}

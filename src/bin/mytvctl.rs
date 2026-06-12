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
}

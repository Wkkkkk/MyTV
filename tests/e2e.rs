//! End-to-end smoke suite against the LIVE prod instance (kunstv.fly.dev).
//!
//! Gated two ways: every networked test is `#[ignore]` AND skips when
//! `MYTV_BASE_URL` / `MYTV_ADMIN_PASSWORD` are unset, so CI and a plain
//! `cargo test` never touch prod. Run manually:
//!
//!   MYTV_BASE_URL=https://kunstv.fly.dev MYTV_ADMIN_PASSWORD=… \
//!     cargo test --test e2e -- --ignored --nocapture

const E2E_PREFIX: &str = "__e2e__";

/// True for any entity name this suite created. The start/end sweep matches on
/// the prefix alone (the per-run token only disambiguates concurrent runners).
fn is_e2e_name(name: &str) -> bool {
    name.starts_with(E2E_PREFIX)
}

#[test]
fn is_e2e_name_matches_only_prefixed() {
    assert!(is_e2e_name("__e2e__1234__crud"));
    assert!(is_e2e_name(E2E_PREFIX));
    assert!(!is_e2e_name("Real News Channel"));
    assert!(!is_e2e_name("not__e2e__embedded"));
}

struct Config {
    base_url: String,
    password: String,
}

/// Returns the prod config, or `None` (after printing a skip line) when either
/// env var is absent — so the test passes as a no-op for contributors w/o creds.
///
/// Loads `.env` first (like the app's `main.rs`) so `MYTV_BASE_URL` /
/// `MYTV_ADMIN_PASSWORD` can live there instead of being typed inline. Real
/// environment variables already set take precedence (dotenvy does not override).
fn env_or_skip() -> Option<Config> {
    dotenvy::dotenv().ok();
    let base_url = std::env::var("MYTV_BASE_URL").unwrap_or_default();
    let password = std::env::var("MYTV_ADMIN_PASSWORD").unwrap_or_default();
    if base_url.is_empty() || password.is_empty() {
        println!("SKIP e2e: set MYTV_BASE_URL and MYTV_ADMIN_PASSWORD to run against prod");
        return None;
    }
    Some(Config {
        base_url: base_url.trim_end_matches('/').to_string(),
        password,
    })
}

/// Thin reqwest wrapper: prefixes the base URL and attaches HTTP Basic auth.
/// The username half is ignored by the server (password-only auth).
struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    password: String,
}

impl ApiClient {
    fn new(cfg: &Config) -> Self {
        ApiClient {
            http: reqwest::Client::new(),
            base_url: cfg.base_url.clone(),
            password: cfg.password.clone(),
        }
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> reqwest::Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self
            .http
            .request(method, url)
            .basic_auth("e2e", Some(&self.password));
        if let Some(b) = body {
            req = req.json(&b);
        }
        req.send().await
    }
}

/// Delete every channel whose name starts with `E2E_PREFIX`. Cleans up any
/// prior crashed run at start, and our own entities at end. Best-effort per
/// channel; returns how many it removed.
async fn sweep(client: &ApiClient) -> Result<usize, String> {
    let resp = client
        .send(reqwest::Method::GET, "/api/admin/channels", None)
        .await
        .map_err(|e| format!("list channels: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("list channels: HTTP {}", resp.status()));
    }
    let channels: Vec<mytv::model::channel::Channel> = resp
        .json()
        .await
        .map_err(|e| format!("decode channels: {e}"))?;
    let mut removed = 0;
    for ch in channels.iter().filter(|c| is_e2e_name(&c.name)) {
        let _ = client
            .send(
                reqwest::Method::DELETE,
                &format!("/api/admin/channels/{}", ch.id),
                None,
            )
            .await;
        removed += 1;
    }
    Ok(removed)
}

fn make_token() -> String {
    std::process::id().to_string()
}

fn e2e_name(scenario: &str, token: &str) -> String {
    format!("{E2E_PREFIX}{token}__{scenario}")
}

/// Create → GET → PATCH → DELETE a channel via the JSON API, asserting the
/// contract at each step. Deserializes into the lib's own `Channel` type so a
/// shape change is a compile error. Hard-fail: returns Err on any mismatch.
async fn scenario_crud(client: &ApiClient, token: &str) -> Result<(), String> {
    use reqwest::Method;
    let name = e2e_name("crud", token);

    // CREATE → 201 + Channel echo
    let resp = client
        .send(
            Method::POST,
            "/api/admin/channels",
            Some(serde_json::json!({
                "name": name, "category": "__e2e__", "type": "live", "sort_order": 0
            })),
        )
        .await
        .map_err(|e| format!("create: {e}"))?;
    if resp.status().as_u16() != 201 {
        return Err(format!("create: expected 201, got {}", resp.status()));
    }
    let created: mytv::model::channel::Channel = resp
        .json()
        .await
        .map_err(|e| format!("create decode: {e}"))?;
    if created.name != name {
        return Err(format!("create: name mismatch ({})", created.name));
    }
    let id = created.id;

    // GET → 200 + same name
    let resp = client
        .send(Method::GET, &format!("/api/admin/channels/{id}"), None)
        .await
        .map_err(|e| format!("get: {e}"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("get: expected 200, got {}", resp.status()));
    }
    let got: mytv::model::channel::Channel =
        resp.json().await.map_err(|e| format!("get decode: {e}"))?;
    if got.id != id || got.name != name {
        return Err("get: round-trip mismatch".into());
    }

    // PATCH (full-replace) → 200 + updated category
    let resp = client
        .send(
            Method::PATCH,
            &format!("/api/admin/channels/{id}"),
            Some(serde_json::json!({
                "name": name, "category": "__e2e__edited", "type": "live", "sort_order": 1
            })),
        )
        .await
        .map_err(|e| format!("patch: {e}"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("patch: expected 200, got {}", resp.status()));
    }
    let patched: mytv::model::channel::Channel = resp
        .json()
        .await
        .map_err(|e| format!("patch decode: {e}"))?;
    if patched.category != "__e2e__edited" || patched.sort_order != 1 {
        return Err("patch: fields not updated".into());
    }

    // DELETE → 204, then GET → 404
    let resp = client
        .send(Method::DELETE, &format!("/api/admin/channels/{id}"), None)
        .await
        .map_err(|e| format!("delete: {e}"))?;
    if resp.status().as_u16() != 204 {
        return Err(format!("delete: expected 204, got {}", resp.status()));
    }
    let resp = client
        .send(Method::GET, &format!("/api/admin/channels/{id}"), None)
        .await
        .map_err(|e| format!("get-after-delete: {e}"))?;
    if resp.status().as_u16() != 404 {
        return Err(format!(
            "get-after-delete: expected 404, got {}",
            resp.status()
        ));
    }
    println!("✓ scenario 1: channel CRUD arc");
    Ok(())
}

/// Create a live channel, attach a source with a harmless fake URL, tune, and
/// assert the returned `source_id`/`source_url` match the source we created
/// (Spec 1 observability). `tune` does not probe the stream — a non-YouTube
/// https URL resolves to itself with status Unknown — so this is deterministic
/// and needs no real stream. Hard-fail.
async fn scenario_tune(client: &ApiClient, token: &str) -> Result<(), String> {
    use reqwest::Method;
    let name = e2e_name("tune", token);
    let fake_url = "https://example.invalid/__e2e__.m3u8";

    // Create live channel
    let resp = client
        .send(
            Method::POST,
            "/api/admin/channels",
            Some(serde_json::json!({
                "name": name, "category": "__e2e__", "type": "live", "sort_order": 0
            })),
        )
        .await
        .map_err(|e| format!("create channel: {e}"))?;
    if resp.status().as_u16() != 201 {
        return Err(format!("create channel: got {}", resp.status()));
    }
    let ch: mytv::model::channel::Channel = resp
        .json()
        .await
        .map_err(|e| format!("channel decode: {e}"))?;
    let cid = ch.id;

    // Helper to guarantee channel cleanup before returning an error.
    async fn cleanup(client: &ApiClient, cid: i64) {
        let _ = client
            .send(
                reqwest::Method::DELETE,
                &format!("/api/admin/channels/{cid}"),
                None,
            )
            .await;
    }

    // Attach a source
    let resp = match client
        .send(
            Method::POST,
            &format!("/api/admin/channels/{cid}/sources"),
            Some(serde_json::json!({ "url": fake_url })),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            cleanup(client, cid).await;
            return Err(format!("create source: {e}"));
        }
    };
    if resp.status().as_u16() != 201 {
        let s = resp.status();
        cleanup(client, cid).await;
        return Err(format!("create source: got {s}"));
    }
    let src: mytv::model::source::Source = match resp.json().await {
        Ok(s) => s,
        Err(e) => {
            cleanup(client, cid).await;
            return Err(format!("source decode: {e}"));
        }
    };

    // Tune (public player route) and assert source_id / source_url
    let result: Result<(), String> = async {
        let resp = client
            .send(Method::GET, &format!("/channel/{cid}/tune"), None)
            .await
            .map_err(|e| format!("tune: {e}"))?;
        if resp.status().as_u16() != 200 {
            return Err(format!("tune: expected 200, got {}", resp.status()));
        }
        let body: serde_json::Value = resp.json().await.map_err(|e| format!("tune decode: {e}"))?;
        if body["source_id"].as_i64() != Some(src.id) {
            return Err(format!(
                "tune: source_id {:?} != created {}",
                body["source_id"], src.id
            ));
        }
        if body["source_url"].as_str() != Some(fake_url) {
            return Err(format!(
                "tune: source_url {:?} != {fake_url}",
                body["source_url"]
            ));
        }
        Ok(())
    }
    .await;

    cleanup(client, cid).await;
    result?;
    println!("✓ scenario 2: tune asserts source_id");
    Ok(())
}

/// Drive the compiled `mytvctl` binary against prod and assert exit codes:
/// 0 = success (create→get→delete), 1 = non-2xx (GET missing id → 404),
/// 2 = MYTV_ADMIN_PASSWORD unset. Advisory: returns Err (→ warning) on any
/// mismatch, never panics.
fn scenario_mytvctl(cfg: &Config, token: &str) -> Result<(), String> {
    use std::process::Command;
    let bin = env!("CARGO_BIN_EXE_mytvctl");

    // `set_password = false` removes the env var to exercise the exit-2 path.
    // Returns Err (never panics) on spawn failure so the scenario stays fully
    // non-panicking — a panic here would bypass the orchestrator's end-sweep.
    let run = |set_password: bool, args: &[&str]| -> Result<std::process::Output, String> {
        let mut cmd = Command::new(bin);
        cmd.args(args).env("MYTV_BASE_URL", &cfg.base_url);
        if set_password {
            cmd.env("MYTV_ADMIN_PASSWORD", &cfg.password);
        } else {
            cmd.env_remove("MYTV_ADMIN_PASSWORD");
        }
        cmd.output().map_err(|e| format!("spawn mytvctl: {e}"))
    };

    let name = e2e_name("ctl", token);

    // exit 0: create → parse id from stdout JSON
    let out = run(
        true,
        &[
            "channel",
            "create",
            "--name",
            &name,
            "--category",
            "__e2e__",
            "--type",
            "live",
        ],
    )?;
    if out.status.code() != Some(0) {
        return Err(format!("create exit {:?}, want 0", out.status.code()));
    }
    let created: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("create stdout not JSON: {e}"))?;
    let id = created["id"].as_i64().ok_or("create stdout missing id")?;

    // exit 0: get
    let id_str = id.to_string();
    let out = run(true, &["channel", "get", &id_str])?;
    if out.status.code() != Some(0) {
        return Err(format!("get exit {:?}, want 0", out.status.code()));
    }

    // exit 0: delete (also cleans up this scenario's channel)
    let out = run(true, &["channel", "delete", &id_str])?;
    if out.status.code() != Some(0) {
        return Err(format!("delete exit {:?}, want 0", out.status.code()));
    }

    // exit 1: GET a non-existent id → server 404 → exit 1
    let out = run(true, &["channel", "get", "999999999"])?;
    if out.status.code() != Some(1) {
        return Err(format!("missing-id exit {:?}, want 1", out.status.code()));
    }

    // exit 2: password env var unset
    let out = run(false, &["channel", "list"])?;
    if out.status.code() != Some(2) {
        return Err(format!("no-password exit {:?}, want 2", out.status.code()));
    }

    println!("✓ scenario 3: mytvctl exit codes 0/1/2");
    Ok(())
}

/// Best-effort discovery checks. Advisory throughout — gone/changed streams or
/// yt-dlp hiccups produce sub-warnings, never a hard failure. Accumulates all
/// sub-failures into one Err string (logged as a single warning by the caller).
async fn scenario_discovery(client: &ApiClient) -> Result<(), String> {
    use reqwest::Method;
    let mut problems: Vec<String> = Vec::new();

    // Deterministic: YouTube search without an API key → 503.
    match client
        .send(
            Method::GET,
            "/api/admin/discover/youtube?keyword=test",
            None,
        )
        .await
    {
        Ok(r) if r.status().as_u16() == 503 => {}
        Ok(r) => problems.push(format!("youtube: expected 503, got {}", r.status())),
        Err(e) => problems.push(format!("youtube: {e}")),
    }

    // Pure URL parse (no network/yt-dlp): a valid channel handle → 200.
    match client
        .send(
            Method::POST,
            "/api/admin/discover/channel",
            Some(serde_json::json!({ "url": "https://www.youtube.com/@NASA" })),
        )
        .await
    {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => problems.push(format!("channel-parse: got {}", r.status())),
        Err(e) => problems.push(format!("channel-parse: {e}")),
    }

    // m3u search against live iptv-org: shape only (200 + JSON array, may be empty).
    match client
        .send(
            Method::GET,
            "/api/admin/discover/m3u?country=se&group=News",
            None,
        )
        .await
    {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(v) if v.is_array() => {}
            Ok(_) => problems.push("m3u: response not a JSON array".into()),
            Err(e) => problems.push(format!("m3u decode: {e}")),
        },
        Ok(r) => problems.push(format!("m3u: got {}", r.status())),
        Err(e) => problems.push(format!("m3u: {e}")),
    }

    // Resolve representative real URLs across protocols/types. Best-effort.
    let resolve_urls = [
        (
            "dash-live",
            "https://demo.unified-streaming.com/k8s/live/scte35.isml/.mpd",
        ),
        (
            "dash-vod",
            "https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd",
        ),
        (
            "hls-live",
            "https://stream.mux.com/v69RSHhFelSm4701snP22dYz2jICy4E4FUyk02rW4gxRM.m3u8",
        ),
        (
            "hls-vod",
            "https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8",
        ),
        ("youtube", "https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
    ];
    for (label, url) in resolve_urls {
        match client
            .send(
                Method::POST,
                "/api/admin/discover/resolve",
                Some(serde_json::json!({ "url": url })),
            )
            .await
        {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => problems.push(format!("resolve {label}: got {}", r.status())),
            Err(e) => problems.push(format!("resolve {label}: {e}")),
        }
    }

    if problems.is_empty() {
        println!("✓ scenario 4: discovery");
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

#[tokio::test]
#[ignore = "e2e against prod — run manually"]
async fn e2e_smoke() {
    let Some(cfg) = env_or_skip() else {
        return;
    };
    println!("== e2e smoke against {} ==", cfg.base_url);
    let client = ApiClient::new(&cfg);

    let pre = sweep(&client)
        .await
        .expect("start sweep failed — is prod reachable / creds valid?");
    println!("start sweep removed {pre} stale __e2e__ channel(s)");

    let token = make_token();
    let mut auth: Result<(), String> = Ok(());
    if auth.is_ok() {
        auth = scenario_crud(&client, &token).await;
    }
    if auth.is_ok() {
        auth = scenario_tune(&client, &token).await;
    }

    let mut warnings: Vec<String> = Vec::new();
    if let Err(e) = scenario_mytvctl(&cfg, &token) {
        warnings.push(format!("mytvctl: {e}"));
    }
    if let Err(e) = scenario_discovery(&client).await {
        warnings.push(format!("discovery: {e}"));
    }

    let post = sweep(&client).await.unwrap_or(0);
    println!("== e2e summary ==");
    println!("end sweep removed {post} leftover __e2e__ channel(s)");
    for w in &warnings {
        println!("⚠ WARN {w}");
    }
    println!("{} advisory warning(s)", warnings.len());
    auth.expect("authoritative e2e scenario failed");
}

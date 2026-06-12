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
fn env_or_skip() -> Option<Config> {
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

    let post = sweep(&client).await.unwrap_or(0);
    println!("== e2e summary ==");
    println!("end sweep removed {post} leftover __e2e__ channel(s)");
    auth.expect("authoritative e2e scenario failed");
}

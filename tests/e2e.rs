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
    })
}

#[tokio::test]
#[ignore = "e2e against prod — run manually"]
async fn e2e_smoke() {
    let Some(cfg) = env_or_skip() else {
        return;
    };
    println!("== e2e smoke against {} ==", cfg.base_url);
}

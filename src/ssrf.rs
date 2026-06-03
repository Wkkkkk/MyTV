use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::net::lookup_host;
use tokio::sync::RwLock;

#[derive(Debug)]
pub enum SsrfError {
    BlockedAddress(IpAddr),
    DnsFailure(String),
    UnsupportedScheme,
}

impl std::fmt::Display for SsrfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SsrfError::BlockedAddress(ip) => write!(f, "blocked address: {ip}"),
            SsrfError::DnsFailure(msg) => write!(f, "DNS failure: {msg}"),
            SsrfError::UnsupportedScheme => write!(f, "unsupported scheme"),
        }
    }
}

fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            let first = v6.segments()[0];
            // fc00::/7 — IPv6 unique local (includes Fly fdaa::/16)
            // fe80::/10 — IPv6 link-local
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

pub async fn is_safe_url(url: &str) -> Result<(), SsrfError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| SsrfError::UnsupportedScheme)?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err(SsrfError::UnsupportedScheme),
    }
    let host = parsed.host_str().ok_or(SsrfError::UnsupportedScheme)?;
    // host_str() returns "[::1]" for IPv6 literals — strip brackets to parse
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        if is_blocked(ip) {
            return Err(SsrfError::BlockedAddress(ip));
        }
        return Ok(());
    }
    let lookup_target = format!("{host}:80");
    let addrs = lookup_host(&lookup_target)
        .await
        .map_err(|e| SsrfError::DnsFailure(e.to_string()))?;
    for addr in addrs {
        if is_blocked(addr.ip()) {
            return Err(SsrfError::BlockedAddress(addr.ip()));
        }
    }
    Ok(())
}

pub type SsrfCache = Arc<RwLock<HashMap<String, std::time::Instant>>>;

pub async fn is_safe_url_cached(url: &str, cache: &SsrfCache) -> Result<(), SsrfError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| SsrfError::UnsupportedScheme)?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err(SsrfError::UnsupportedScheme),
    }
    let host = parsed
        .host_str()
        .ok_or(SsrfError::UnsupportedScheme)?
        .to_string();
    {
        let r = cache.read().await;
        if let Some(ts) = r.get(&host) {
            if ts.elapsed() < std::time::Duration::from_secs(60) {
                return Ok(());
            }
        }
    }
    is_safe_url(url).await?;
    cache.write().await.insert(host, std::time::Instant::now());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn blocks_loopback_ipv4() {
        assert!(matches!(
            is_safe_url("http://127.0.0.1/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_rfc1918_10() {
        assert!(matches!(
            is_safe_url("http://10.0.0.1/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_rfc1918_172() {
        assert!(matches!(
            is_safe_url("http://172.16.0.1/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_rfc1918_192() {
        assert!(matches!(
            is_safe_url("http://192.168.1.1/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_link_local_metadata() {
        assert!(matches!(
            is_safe_url("http://169.254.169.254/latest/meta-data/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_ipv6_loopback() {
        assert!(matches!(
            is_safe_url("http://[::1]/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_ipv6_ula() {
        assert!(matches!(
            is_safe_url("http://[fc00::1]/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn blocks_ipv6_link_local() {
        assert!(matches!(
            is_safe_url("http://[fe80::1]/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn allows_routable_ipv4_literal() {
        assert!(is_safe_url("http://1.1.1.1/").await.is_ok());
    }

    #[tokio::test]
    async fn blocks_rfc1918_172_upper_bound() {
        assert!(matches!(
            is_safe_url("http://172.31.255.255/").await,
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[tokio::test]
    async fn cache_hit_returns_ok() {
        let cache: SsrfCache = Arc::new(RwLock::new(HashMap::new()));
        cache
            .write()
            .await
            .insert("1.1.1.1".to_string(), std::time::Instant::now());
        assert!(is_safe_url_cached("http://1.1.1.1/", &cache).await.is_ok());
    }

    #[tokio::test]
    async fn cache_miss_on_blocked_host() {
        let cache: SsrfCache = Arc::new(RwLock::new(HashMap::new()));
        let result = is_safe_url_cached("http://127.0.0.1/", &cache).await;
        assert!(matches!(result, Err(SsrfError::BlockedAddress(_))));
        assert!(cache.read().await.is_empty());
    }

    #[tokio::test]
    async fn cache_expires_and_refreshes() {
        let cache: SsrfCache = Arc::new(RwLock::new(HashMap::new()));
        cache.write().await.insert(
            "1.1.1.1".to_string(),
            std::time::Instant::now() - std::time::Duration::from_secs(61),
        );
        assert!(is_safe_url_cached("http://1.1.1.1/", &cache).await.is_ok());
        let elapsed = cache.read().await.get("1.1.1.1").unwrap().elapsed();
        assert!(elapsed < std::time::Duration::from_secs(1));
    }
}

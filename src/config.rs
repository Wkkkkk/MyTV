pub struct Config {
    pub database_url: String,
    pub admin_password: String,
    pub youtube_api_key: Option<String>,
    pub port: u16,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Config {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:mytv.db".to_string()),
            admin_password: std::env::var("ADMIN_PASSWORD")
                .unwrap_or_else(|_| "admin".to_string()),
            youtube_api_key: std::env::var("YOUTUBE_API_KEY").ok(),
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("PORT");
        std::env::remove_var("YOUTUBE_API_KEY");

        let config = Config::from_env().unwrap();

        assert_eq!(config.database_url, "sqlite:mytv.db");
        assert_eq!(config.port, 3000);
        assert!(config.youtube_api_key.is_none());
    }

    #[test]
    fn test_config_reads_env_vars() {
        std::env::set_var("DATABASE_URL", "sqlite:test.db");
        std::env::set_var("PORT", "8080");
        std::env::set_var("YOUTUBE_API_KEY", "abc123");

        let config = Config::from_env().unwrap();

        assert_eq!(config.database_url, "sqlite:test.db");
        assert_eq!(config.port, 8080);
        assert_eq!(config.youtube_api_key, Some("abc123".to_string()));

        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("PORT");
        std::env::remove_var("YOUTUBE_API_KEY");
    }
}

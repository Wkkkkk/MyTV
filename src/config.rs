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
            admin_password: {
                match std::env::var("ADMIN_PASSWORD") {
                    Ok(p) => p,
                    Err(_) => {
                        eprintln!(
                            "WARNING: ADMIN_PASSWORD not set — using insecure default 'admin'"
                        );
                        "admin".to_string()
                    }
                }
            },
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
    use std::sync::Mutex;

    // Serialize all env-var tests so they don't race each other in the same process.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn save_env_vars() -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        (
            std::env::var("DATABASE_URL").ok(),
            std::env::var("PORT").ok(),
            std::env::var("YOUTUBE_API_KEY").ok(),
            std::env::var("ADMIN_PASSWORD").ok(),
        )
    }

    fn restore_env_vars(
        saved: (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    ) {
        match saved.0 {
            Some(v) => std::env::set_var("DATABASE_URL", v),
            None => std::env::remove_var("DATABASE_URL"),
        }
        match saved.1 {
            Some(v) => std::env::set_var("PORT", v),
            None => std::env::remove_var("PORT"),
        }
        match saved.2 {
            Some(v) => std::env::set_var("YOUTUBE_API_KEY", v),
            None => std::env::remove_var("YOUTUBE_API_KEY"),
        }
        match saved.3 {
            Some(v) => std::env::set_var("ADMIN_PASSWORD", v),
            None => std::env::remove_var("ADMIN_PASSWORD"),
        }
    }

    #[test]
    fn test_config_defaults() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env_vars();

        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("PORT");
        std::env::remove_var("YOUTUBE_API_KEY");
        std::env::remove_var("ADMIN_PASSWORD");

        let config = Config::from_env().unwrap();

        assert_eq!(config.database_url, "sqlite:mytv.db");
        assert_eq!(config.port, 3000);
        assert!(config.youtube_api_key.is_none());

        restore_env_vars(saved);
    }

    #[test]
    fn test_config_reads_env_vars() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let saved = save_env_vars();

        std::env::set_var("DATABASE_URL", "sqlite:test.db");
        std::env::set_var("PORT", "8080");
        std::env::set_var("YOUTUBE_API_KEY", "abc123");
        std::env::set_var("ADMIN_PASSWORD", "test_pass");

        let config = Config::from_env().unwrap();

        assert_eq!(config.database_url, "sqlite:test.db");
        assert_eq!(config.port, 8080);
        assert_eq!(config.youtube_api_key, Some("abc123".to_string()));
        assert_eq!(config.admin_password, "test_pass");

        restore_env_vars(saved);
    }
}

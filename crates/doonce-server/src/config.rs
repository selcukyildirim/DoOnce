use std::path::PathBuf;

use anyhow::{Result, bail};
use doonce_core::{DeploymentMode, region_from_env};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub production: bool,
    pub deployment_mode: DeploymentMode,
    pub region: String,
    pub bind: String,
    pub ui_dir: PathBuf,
    pub artifact_dir: PathBuf,
    pub cors_origins: Vec<String>,
    pub dev_no_auth: bool,
    pub headless: bool,
    pub public_url: String,
    pub cookie_secure: bool,
    pub session_ttl_secs: i64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            production: is_production(),
            deployment_mode: DeploymentMode::from_env(),
            region: region_from_env(),
            bind: "127.0.0.1:8080".into(),
            ui_dir: PathBuf::from("web/dist"),
            artifact_dir: data_dir().join("artifacts"),
            cors_origins: vec![
                "http://localhost:5173".into(),
                "http://127.0.0.1:5173".into(),
            ],
            dev_no_auth: false,
            headless: true,
            public_url: "http://127.0.0.1:8080".into(),
            cookie_secure: false,
            session_ttl_secs: 28_800,
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let mut config = ServerConfig {
            production: is_production(),
            deployment_mode: DeploymentMode::from_env(),
            region: region_from_env(),
            ..ServerConfig::default()
        };
        if let Some(bind) = std::env::var("DOONCE_BIND")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            config.bind = bind;
        }
        if let Some(ui_dir) = std::env::var("DOONCE_UI_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            config.ui_dir = PathBuf::from(ui_dir);
        }
        if let Some(dir) = std::env::var("DOONCE_ARTIFACT_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            config.artifact_dir = PathBuf::from(dir);
        }
        if let Ok(origins) = std::env::var("DOONCE_CORS_ORIGINS") {
            config.cors_origins = origins
                .split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(str::to_string)
                .collect();
        }
        if let Ok(value) = std::env::var("DOONCE_DEV_NO_AUTH") {
            config.dev_no_auth = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            );
        }
        if let Ok(value) = std::env::var("DOONCE_HEADLESS") {
            config.headless = !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no"
            );
        }
        if let Some(value) = std::env::var("DOONCE_PUBLIC_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            config.public_url = value.trim().trim_end_matches('/').to_string();
        }
        if let Ok(value) = std::env::var("DOONCE_COOKIE_SECURE") {
            config.cookie_secure = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        } else {
            config.cookie_secure = config.public_url.starts_with("https://");
        }
        if let Some(value) = std::env::var("DOONCE_SESSION_TTL_SECS")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value > 0)
        {
            config.session_ttl_secs = value;
        }
        config
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_with(master_key_present())
    }

    fn validate_with(&self, master_key_present: bool) -> Result<()> {
        if self.region.trim().is_empty() {
            bail!("DOONCE_REGION must not be empty");
        }
        if self.production {
            if self.dev_no_auth {
                bail!("DOONCE_ENV=production refuses --dev-no-auth");
            }
            if !master_key_present {
                bail!("DOONCE_ENV=production requires DOONCE_MASTER_KEY from KMS/secret manager");
            }
            if !self.public_url.starts_with("https://") {
                bail!("DOONCE_ENV=production requires an https DOONCE_PUBLIC_URL");
            }
            if !self.cookie_secure {
                bail!("DOONCE_ENV=production requires secure cookies");
            }
        }
        if self.deployment_mode.is_air_gapped() {
            let telemetry = std::env::var("DOONCE_TELEMETRY").ok().is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            });
            if telemetry {
                bail!("air-gapped mode requires DOONCE_TELEMETRY=false");
            }
            if self.dev_no_auth {
                bail!("air-gapped mode refuses --dev-no-auth; use API keys or SSO");
            }
        }
        Ok(())
    }
}

fn is_production() -> bool {
    std::env::var("DOONCE_ENV").ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "production" | "prod"
        )
    })
}

fn master_key_present() -> bool {
    std::env::var("DOONCE_MASTER_KEY")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
}

pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("DOONCE_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DoOnce")
}

pub fn database_url() -> String {
    if let Some(url) = std::env::var("DOONCE_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return url;
    }
    format!(
        "sqlite://{}?mode=rwc",
        data_dir().join("doonce.db").display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_requires_tls_master_key_and_no_dev_auth() {
        let mut config = ServerConfig {
            production: true,
            public_url: "http://doonce.example.com".into(),
            cookie_secure: false,
            ..ServerConfig::default()
        };
        assert!(config.validate_with(false).is_err());
        assert!(config.validate_with(true).is_err(), "http is rejected");
        config.public_url = "https://doonce.example.com".into();
        config.cookie_secure = true;
        assert!(
            config.validate_with(false).is_err(),
            "master key is required"
        );
        assert!(config.validate_with(true).is_ok());
        config.dev_no_auth = true;
        assert!(config.validate_with(true).is_err());
    }

    #[test]
    fn air_gapped_rejects_dev_auth() {
        let config = ServerConfig {
            deployment_mode: DeploymentMode::AirGapped,
            dev_no_auth: true,
            ..ServerConfig::default()
        };
        assert!(config.validate_with(true).is_err());
    }
}

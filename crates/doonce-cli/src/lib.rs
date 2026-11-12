//! DoOnce CLI implementation. Kept in a library so integration tests can call
//! the bootstrap and diagnostic paths without spawning the binary.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use doonce_core::{DeploymentMode, Role, now_rfc3339, region_from_env};
use doonce_security::{MasterKey, generate_api_key};
use doonce_store::{Database, Dialect, repo};
use serde_json::json;

pub mod connector;
pub mod mcp;

pub const APP_NAME: &str = "doonce";
pub const DEFAULT_TENANT_SLUG: &str = "default";

pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("DOONCE_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DoOnce")
}

pub fn database_url() -> String {
    if let Some(url) = std::env::var("DOONCE_DATABASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return url;
    }
    let path = data_dir().join("doonce.db");
    format!("sqlite://{}?mode=rwc", path.display())
}

pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

pub async fn connect_and_migrate() -> Result<Database> {
    let url = database_url();
    let db = Database::connect(&url)
        .await
        .with_context(|| format!("could not open database {url}"))?;
    db.migrate().await.context("database migration failed")?;
    Ok(db)
}

pub struct QuickstartReport {
    pub data_dir: PathBuf,
    pub database_url: String,
    pub dialect: Dialect,
    pub tenant_id: String,
    pub tenant_name: String,
    pub tenant_slug: String,
    pub user_id: String,
    pub email: String,
    pub api_key: String,
    pub created_tenant: bool,
    pub created_user: bool,
}

pub async fn quickstart(email: Option<&str>) -> Result<QuickstartReport> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    let master = MasterKey::load_or_create(&dir.join("master.key"))?;
    // Touch the encryptor so a broken key is detected before the first run.
    master.encryptor().encrypt_str("doonce-bootstrap")?;

    let url = database_url();
    let db = connect_and_migrate().await?;
    let (tenant, created_tenant) = match repo::find_tenant_by_slug(&db, DEFAULT_TENANT_SLUG).await?
    {
        Some(tenant) => (tenant, false),
        None => (
            repo::create_tenant_with_region(
                &db,
                "Default",
                DEFAULT_TENANT_SLUG,
                &region_from_env(),
            )
            .await?,
            true,
        ),
    };

    let email = email
        .unwrap_or("admin@localhost")
        .trim()
        .to_ascii_lowercase();
    let (user, created_user) = match repo::find_user_by_email(&db, &tenant.id, &email).await? {
        Some(user) => (user, false),
        None => (
            repo::create_user(&db, &tenant.id, &email, "Administrator", Role::Owner).await?,
            true,
        ),
    };

    let generated = generate_api_key();
    repo::create_api_key(
        &db,
        &tenant.id,
        Some(&user.id),
        "quickstart",
        &generated.prefix,
        &generated.hash,
        &[],
    )
    .await?;

    repo::set_setting(&db, &tenant.id, "server.bind", "127.0.0.1:8080").await?;
    repo::set_setting(&db, &tenant.id, "telemetry.enabled", "false").await?;
    repo::set_setting(&db, &tenant.id, "security.require_mfa", "true").await?;
    repo::append_audit(
        &db,
        &tenant.id,
        Some(&user.id),
        "quickstart.completed",
        "tenant:default",
        &json!({"email": email, "database": url, "dialect": format!("{dialect:?}", dialect = db.dialect())}),
    )
    .await?;

    Ok(QuickstartReport {
        data_dir: dir,
        database_url: url,
        dialect: db.dialect(),
        tenant_id: tenant.id,
        tenant_name: tenant.name,
        tenant_slug: tenant.slug,
        user_id: user.id,
        email,
        api_key: generated.token,
        created_tenant,
        created_user,
    })
}

#[derive(Debug)]
pub struct DoctorCheck {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn healthy(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }
}

pub async fn doctor() -> Result<DoctorReport> {
    let mut checks = Vec::new();
    let dir = data_dir();

    let writable = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let probe = dir.join(".doonce-doctor");
        std::fs::write(&probe, b"ok")?;
        std::fs::remove_file(&probe)
    })();
    checks.push(DoctorCheck {
        name: "data directory",
        ok: writable.is_ok(),
        detail: match &writable {
            Ok(()) => dir.display().to_string(),
            Err(e) => format!("{}: {e}", dir.display()),
        },
    });

    let mode = DeploymentMode::from_env();
    let region = region_from_env();
    checks.push(DoctorCheck {
        name: "deployment mode",
        ok: true,
        detail: format!("{mode} (region {region})"),
    });
    if mode.is_air_gapped() {
        let telemetry = std::env::var("DOONCE_TELEMETRY").ok().is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
        checks.push(DoctorCheck {
            name: "air-gapped telemetry",
            ok: !telemetry,
            detail: if telemetry {
                "DOONCE_TELEMETRY must be false".into()
            } else {
                "disabled".into()
            },
        });
        let provider = std::env::var("DOONCE_LLM_PROVIDER")
            .or_else(|_| std::env::var("DOONCE_AI_PROVIDER"))
            .unwrap_or_default();
        let local = provider.is_empty()
            || matches!(
                provider.trim().to_ascii_lowercase().as_str(),
                "ollama" | "openai-compatible" | "openai_compatible"
            );
        checks.push(DoctorCheck {
            name: "air-gapped LLM",
            ok: local,
            detail: if provider.is_empty() {
                "no AI provider configured".into()
            } else {
                format!("provider={provider} (use Ollama or a loopback compatible endpoint)")
            },
        });
    }

    let master_from_env = std::env::var("DOONCE_MASTER_KEY")
        .ok()
        .is_some_and(|v| !v.trim().is_empty());
    let key_path = dir.join("master.key");
    let master_ok = master_from_env || key_path.exists();
    checks.push(DoctorCheck {
        name: "master key",
        ok: master_ok,
        detail: if master_from_env {
            "DOONCE_MASTER_KEY (environment/KMS)".into()
        } else if key_path.exists() {
            format!(
                "{} (local file: move to KMS/HSM for production)",
                key_path.display()
            )
        } else {
            "missing; run `doonce quickstart`".into()
        },
    });

    let db_result = connect_and_migrate().await;
    match &db_result {
        Ok(db) => {
            let ping = db.ping().await;
            checks.push(DoctorCheck {
                name: "database",
                ok: ping.is_ok(),
                detail: format!("{:?} — {}", db.dialect(), database_url_redacted()),
            });
        }
        Err(e) => checks.push(DoctorCheck {
            name: "database",
            ok: false,
            detail: format!("{e:#}"),
        }),
    }

    if let Ok(db) = db_result {
        match repo::find_tenant_by_slug(&db, DEFAULT_TENANT_SLUG).await {
            Ok(Some(tenant)) => {
                checks.push(DoctorCheck {
                    name: "default tenant",
                    ok: true,
                    detail: format!("{} ({})", tenant.name, tenant.slug),
                });
                let users = repo::count_users(&db, &tenant.id).await.unwrap_or(-1);
                let keys = repo::count_api_keys(&db, &tenant.id).await.unwrap_or(-1);
                let workflows = repo::count_workflows(&db, &tenant.id).await.unwrap_or(-1);
                let tasks = repo::count_tasks(&db, &tenant.id).await.unwrap_or(-1);
                checks.push(DoctorCheck {
                    name: "tenant resources",
                    ok: users >= 0 && keys >= 0,
                    detail: format!(
                        "{users} users, {keys} API keys, {workflows} workflows, {tasks} tasks"
                    ),
                });
                if keys == 0 {
                    checks.push(DoctorCheck {
                        name: "api key",
                        ok: false,
                        detail: "no API key exists; run `doonce quickstart`".into(),
                    });
                }
                if db.dialect() == Dialect::Sqlite
                    && std::env::var("DOONCE_ENV").ok().as_deref() == Some("production")
                {
                    checks.push(DoctorCheck {
                        name: "production database",
                        ok: false,
                        detail: "SQLite is not supported for production HA; use Postgres".into(),
                    });
                }
            }
            Ok(None) => checks.push(DoctorCheck {
                name: "default tenant",
                ok: false,
                detail: "not initialized; run `doonce quickstart`".into(),
            }),
            Err(e) => checks.push(DoctorCheck {
                name: "default tenant",
                ok: false,
                detail: format!("{e}"),
            }),
        }
    }

    Ok(DoctorReport { checks })
}

pub fn database_url_redacted() -> String {
    let url = database_url();
    if let Some((scheme, rest)) = url.split_once("://")
        && let Some((creds, host)) = rest.split_once('@')
    {
        let user = creds.split(':').next().unwrap_or("user");
        return format!("{scheme}://{user}:***@{host}");
    }
    url
}

pub async fn config_get(key: &str) -> Result<Option<String>> {
    let db = connect_and_migrate().await?;
    let tenant = default_tenant(&db).await?;
    Ok(repo::get_setting(&db, &tenant.id, key).await?)
}

pub async fn config_set(key: &str, value: &str) -> Result<()> {
    let db = connect_and_migrate().await?;
    let tenant = default_tenant(&db).await?;
    if looks_sensitive_key(key) {
        bail!(
            "`{key}` looks like a secret; store secrets through the credential/LLM config API, not settings"
        );
    }
    repo::set_setting(&db, &tenant.id, key, value).await?;
    repo::append_audit(
        &db,
        &tenant.id,
        None,
        "setting.updated",
        &format!("setting:{key}"),
        &json!({"key": key}),
    )
    .await?;
    Ok(())
}

pub async fn config_list() -> Result<Vec<(String, String)>> {
    let db = connect_and_migrate().await?;
    let tenant = default_tenant(&db).await?;
    let settings = repo::list_settings(&db, &tenant.id).await?;
    Ok(settings.into_iter().map(|s| (s.key, s.value)).collect())
}

pub async fn default_tenant(db: &Database) -> Result<doonce_core::Tenant> {
    repo::find_tenant_by_slug(db, DEFAULT_TENANT_SLUG)
        .await?
        .ok_or_else(|| anyhow::anyhow!("DoOnce is not initialized; run `doonce quickstart`"))
}

fn looks_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "secret",
        "password",
        "passwd",
        "token",
        "api_key",
        "apikey",
        "private_key",
        "credential",
    ]
    .iter()
    .any(|needle| k.contains(needle))
}

/// RFC 3339 timestamp used by deterministic CLI output in tests.
pub fn timestamp() -> String {
    now_rfc3339()
}

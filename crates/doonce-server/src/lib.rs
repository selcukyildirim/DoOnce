//! DoOnce Enterprise control-plane server.

#![forbid(unsafe_code)]
#![recursion_limit = "256"]

pub mod api;
pub mod auth;
pub mod config;
pub mod error;
pub mod generate;
pub mod llm;
pub mod openapi;
pub mod session;
pub mod state;

use anyhow::{Context, Result};
use axum::Router;
use doonce_security::MasterKey;
use doonce_store::Database;
use state::AppState;

pub fn build_router(state: AppState) -> Router {
    api::router(state)
}

pub async fn serve(config: config::ServerConfig) -> Result<()> {
    config.validate()?;
    let database_url = config::database_url();
    let db = Database::connect(&database_url)
        .await
        .with_context(|| format!("could not open database {database_url}"))?;
    if config.production && db.dialect() != doonce_store::Dialect::Postgres {
        anyhow::bail!("DOONCE_ENV=production requires Postgres; SQLite is single-node only");
    }
    db.migrate().await.context("database migration failed")?;

    let data_dir = config::data_dir();
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("could not create {}", data_dir.display()))?;
    let master = MasterKey::load_or_create(&data_dir.join("master.key"))?;
    let state = AppState::new(db, master.encryptor(), config.clone());
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(&config.bind)
        .await
        .with_context(|| format!("could not bind {}", config.bind))?;
    let address = listener.local_addr()?;
    println!(
        "DoOnce API listening on http://{address} (mode={}, region={})",
        config.deployment_mode, config.region
    );
    println!("OpenAPI: http://{address}/api/v1/openapi.json");
    if config.ui_dir.join("index.html").exists() {
        println!("Web console: http://{address}/");
    } else {
        println!(
            "Web console not built (expected {})",
            config.ui_dir.join("index.html").display()
        );
    }
    axum::serve(listener, app).await?;
    Ok(())
}

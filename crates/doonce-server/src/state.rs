use std::sync::Arc;

use doonce_core::Role;
use doonce_security::Encryptor;
use doonce_store::Database;

use crate::config::ServerConfig;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub encryptor: Encryptor,
    pub config: Arc<ServerConfig>,
}

impl AppState {
    pub fn new(db: Database, encryptor: Encryptor, config: ServerConfig) -> Self {
        AppState {
            db,
            encryptor,
            config: Arc::new(config),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub tenant_id: String,
    pub user_id: Option<String>,
    pub role: Role,
    pub scopes: Vec<String>,
    pub api_key_id: Option<String>,
    pub auth_method: String,
    pub session_id: Option<String>,
    pub mfa_verified: bool,
}

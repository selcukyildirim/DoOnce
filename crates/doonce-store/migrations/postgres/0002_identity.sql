CREATE TABLE IF NOT EXISTS identity_providers (
    id                     TEXT PRIMARY KEY,
    tenant_id              TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name                   TEXT NOT NULL,
    kind                   TEXT NOT NULL DEFAULT 'oidc',
    issuer                 TEXT NOT NULL,
    client_id              TEXT NOT NULL,
    client_secret_encrypted TEXT,
    scopes                 TEXT NOT NULL DEFAULT '["openid","email","profile"]',
    enabled                INTEGER NOT NULL DEFAULT 1,
    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL,
    UNIQUE (tenant_id, name)
);

CREATE TABLE IF NOT EXISTS user_identities (
    id         TEXT PRIMARY KEY,
    tenant_id  TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider   TEXT NOT NULL,
    subject    TEXT NOT NULL,
    email      TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (tenant_id, provider, subject)
);

CREATE TABLE IF NOT EXISTS sessions (
    id           TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash   TEXT NOT NULL UNIQUE,
    csrf_hash    TEXT NOT NULL,
    mfa_verified INTEGER NOT NULL DEFAULT 0,
    user_agent   TEXT,
    ip           TEXT,
    expires_at   TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    last_used_at TEXT,
    revoked_at   TEXT
);
CREATE INDEX IF NOT EXISTS sessions_by_user ON sessions(user_id);

CREATE TABLE IF NOT EXISTS oidc_states (
    id                      TEXT PRIMARY KEY,
    tenant_id               TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    provider_id             TEXT NOT NULL REFERENCES identity_providers(id) ON DELETE CASCADE,
    state                   TEXT NOT NULL UNIQUE,
    nonce                   TEXT NOT NULL,
    pkce_verifier_encrypted TEXT NOT NULL,
    redirect_uri            TEXT NOT NULL,
    expires_at              TEXT NOT NULL,
    consumed_at             TEXT,
    created_at              TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS mfa_totp (
    tenant_id        TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id          TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    secret_encrypted TEXT NOT NULL,
    confirmed        INTEGER NOT NULL DEFAULT 0,
    created_at       TEXT NOT NULL,
    PRIMARY KEY (tenant_id, user_id)
);

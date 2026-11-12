use doonce_core::validation::parse_workflow;
use doonce_core::{CredentialKind, Role};
use doonce_engine::{ArtifactStore, CredentialResolver, WorkflowResolver};
use doonce_security::MasterKey;
use doonce_services::{CredentialService, DbWorkflowResolver, LocalArtifactStore};
use doonce_store::{Database, repo};
use serde_json::json;
use totp_rs::{Algorithm, Secret, TOTP};

async fn database() -> (Database, String) {
    let db = Database::open_in_memory().await.expect("database");
    let tenant = repo::create_tenant(&db, "Test", "test")
        .await
        .expect("tenant");
    (db, tenant.id)
}

#[tokio::test]
async fn resolves_encrypted_credentials_and_totp() {
    let (db, tenant_id) = database().await;
    let encryptor = MasterKey::generate().encryptor();

    let secret = encryptor.encrypt_str("hunter2").unwrap();
    let credential = repo::create_credential(
        &db,
        &tenant_id,
        "Portal",
        CredentialKind::UsernamePassword,
        Some("ada"),
        &secret,
        None,
        &json!({}),
    )
    .await
    .unwrap();

    let service = CredentialService::new(db.clone(), encryptor.clone(), tenant_id.clone());
    let resolved = service.resolve(&credential.id).await.unwrap();
    assert_eq!(resolved.username.as_deref(), Some("ada"));
    assert_eq!(resolved.secret, "hunter2");
    assert!(resolved.totp_code.is_none());

    let encoded = match Secret::generate_secret().to_encoded() {
        Secret::Encoded(value) => value,
        Secret::Raw(_) => unreachable!(),
    };
    let encrypted_totp = encryptor.encrypt_str(&encoded).unwrap();
    let totp_credential = repo::create_credential(
        &db,
        &tenant_id,
        "TOTP",
        CredentialKind::Totp,
        Some("ada"),
        &secret,
        Some(&encrypted_totp),
        &json!({}),
    )
    .await
    .unwrap();
    let resolved = service.resolve(&totp_credential.id).await.unwrap();
    let code = resolved.totp_code.expect("totp code");
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Encoded(encoded).to_bytes().unwrap(),
        Some("DoOnce".to_string()),
        "ada".to_string(),
    )
    .unwrap();
    assert!(totp.check_current(&code).unwrap());
}

#[tokio::test]
async fn local_artifacts_stay_inside_the_upload_directory() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("report.csv");
    std::fs::write(&source, b"a,b\n1,2\n").unwrap();
    let store = LocalArtifactStore::new(dir.path().join("uploads"));
    let result = store
        .put(source.to_string_lossy().as_ref(), "reports/report.csv")
        .await
        .unwrap();
    assert!(dir.path().join("uploads/reports/report.csv").exists());
    assert_eq!(result["bytes"], 8);

    let escaped = store
        .put(source.to_string_lossy().as_ref(), "../escape.csv")
        .await;
    assert!(escaped.is_err());
}

#[tokio::test]
async fn database_workflow_resolver_loads_the_current_definition() {
    let (db, tenant_id) = database().await;
    let definition = parse_workflow(
        r#"{"schemaVersion":"2.0","name":"Nested","blocks":[{"id":"done","type":"terminate","status":"completed"}]}"#,
    )
    .unwrap();
    let workflow = repo::create_workflow(&db, &tenant_id, &definition, None, "test")
        .await
        .unwrap();
    let resolver = DbWorkflowResolver::new(db, tenant_id);
    let loaded = resolver.load(&workflow.slug).await.unwrap();
    assert_eq!(loaded.name, "Nested");
    assert_eq!(loaded.blocks.len(), 1);
}

#[tokio::test]
async fn user_role_is_available_for_service_scoping() {
    let (db, tenant_id) = database().await;
    let user = repo::create_user(&db, &tenant_id, "owner@test", "Owner", Role::Owner)
        .await
        .unwrap();
    assert_eq!(user.role, "owner");
}

#[tokio::test]
async fn resolves_http_external_credentials_and_totp() {
    use doonce_services::providers::{CredentialProviders, HttpProvider};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer).await;
            let body = r#"{"username":"ada","password":"hunter2","totp":"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });

    let (db, tenant_id) = database().await;
    let encryptor = MasterKey::generate().encryptor();
    let credential = doonce_store::repo::create_credential_with_provider(
        &db,
        &tenant_id,
        "HTTP vault",
        CredentialKind::UsernamePassword,
        "http",
        Some("item-1"),
        None,
        "",
        None,
        &json!({}),
    )
    .await
    .unwrap();
    let providers = CredentialProviders {
        http: Some(HttpProvider {
            base_url: format!("http://{address}/credentials"),
            token: Some("test-token".into()),
            allow_private: true,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        }),
        ..CredentialProviders::default()
    };
    let service = CredentialService::with_providers(db, encryptor, tenant_id, providers);
    let resolved = service.resolve(&credential.id).await.unwrap();
    assert_eq!(resolved.username.as_deref(), Some("ada"));
    assert_eq!(resolved.secret, "hunter2");
    assert!(resolved.totp_code.is_some());
    server.abort();
}

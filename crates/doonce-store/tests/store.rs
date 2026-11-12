use doonce_core::validation::parse_workflow;
use doonce_core::{ArtifactKind, CredentialKind, Role, TaskStatus};
use doonce_security::{generate_api_key, verify_api_key};
use doonce_store::{Database, repo};
use serde_json::json;

async fn db() -> Database {
    Database::open_in_memory()
        .await
        .expect("in-memory database")
}

const WORKFLOW: &str = r#"{
  "schemaVersion": "2.0",
  "name": "Daily Report",
  "description": "Open the report page",
  "blocks": [
    {"id":"open","type":"browserAction","action":{"kind":"navigate","url":"https://example.com"},"next":"done"},
    {"id":"done","type":"terminate","status":"completed"}
  ]
}"#;

#[tokio::test]
async fn provisions_identity_and_api_keys() {
    let db = db().await;
    let tenant = repo::create_tenant(&db, "Acme", "acme").await.unwrap();
    let user = repo::create_user(&db, &tenant.id, "admin@acme.test", "Admin", Role::Owner)
        .await
        .unwrap();

    let key = generate_api_key();
    let record = repo::create_api_key(
        &db,
        &tenant.id,
        Some(&user.id),
        "ci",
        &key.prefix,
        &key.hash,
        &["workflows:read".to_string()],
    )
    .await
    .unwrap();

    assert_eq!(record.tenant_id, tenant.id);
    assert!(verify_api_key(&key.token, &record.key_hash));
    let found = repo::find_api_key_by_prefix(&db, &key.prefix)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, record.id);
    assert_eq!(found.scopes, vec!["workflows:read"]);

    repo::mark_api_key_used(&db, &record.id).await.unwrap();
    repo::revoke_api_key(&db, &tenant.id, &record.id)
        .await
        .unwrap();
    assert_eq!(repo::count_api_keys(&db, &tenant.id).await.unwrap(), 1);
    assert_eq!(repo::count_users(&db, &tenant.id).await.unwrap(), 1);
}

#[tokio::test]
async fn stores_workflows_with_versions_and_rejects_invalid_ones() {
    let db = db().await;
    let tenant = repo::create_tenant(&db, "Acme", "acme").await.unwrap();
    let definition = parse_workflow(WORKFLOW).unwrap();
    let record = repo::create_workflow(&db, &tenant.id, &definition, None, "test")
        .await
        .unwrap();
    assert_eq!(record.slug, "daily-report");
    assert_eq!(record.current_version, 1);

    let second = parse_workflow(WORKFLOW).unwrap();
    let version = repo::create_workflow_version(&db, &tenant.id, &record.id, &second, "edit", None)
        .await
        .unwrap();
    assert_eq!(version.version, 2);

    let (current, current_def) = repo::current_workflow_definition(&db, &tenant.id, &record.slug)
        .await
        .unwrap();
    assert_eq!(current.current_version, 2);
    assert_eq!(current_def.blocks.len(), 2);
    assert_eq!(
        repo::list_workflow_versions(&db, &tenant.id, &record.id)
            .await
            .unwrap()
            .len(),
        2
    );

    let invalid = parse_workflow(WORKFLOW);
    // Validation happens before the repository; ensure the repository refuses
    // a definition that was mutated after parsing.
    let mut broken = invalid.unwrap();
    broken.blocks[0].id = "bad id".into();
    assert!(
        repo::create_workflow(&db, &tenant.id, &broken, None, "bad")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn records_tasks_actions_and_artifacts() {
    let db = db().await;
    let tenant = repo::create_tenant(&db, "Acme", "acme").await.unwrap();
    let task = repo::create_task(
        &db,
        &tenant.id,
        None,
        "Find the top post",
        Some("https://news.example.test"),
        None,
        Some(&json!({"type": "object"})),
        &["NOT_FOUND".to_string()],
        Some("test-model"),
        10,
    )
    .await
    .unwrap();
    assert_eq!(task.status, "queued");
    assert_eq!(task.max_steps, 10);

    let running = repo::set_task_status(
        &db,
        &tenant.id,
        &task.id,
        TaskStatus::Running,
        None,
        None,
        Some("2026-01-01T00:00:00Z"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(running.status, "running");

    let action = repo::record_task_action(
        &db,
        &tenant.id,
        &task.id,
        &json!({"kind": "click", "ref": 1}),
        Some("clicked the first result"),
        None,
        "completed",
    )
    .await
    .unwrap();
    assert_eq!(action.seq, 1);

    let artifact = repo::create_artifact(
        &db,
        &tenant.id,
        None,
        Some(&task.id),
        None,
        ArtifactKind::Screenshot,
        "step-1.png",
        "image/png",
        42,
        "tenant/step-1.png",
        "hash",
    )
    .await
    .unwrap();
    let artifacts = repo::list_artifacts_for_task(&db, &tenant.id, &task.id)
        .await
        .unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].id, artifact.id);

    let finished = repo::set_task_status(
        &db,
        &tenant.id,
        &task.id,
        TaskStatus::Completed,
        Some(&json!({"title": "Hello"})),
        None,
        None,
        Some("2026-01-01T00:01:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(finished.status, "completed");
    assert_eq!(finished.output.unwrap()["title"], "Hello");
}

#[tokio::test]
async fn settings_and_audit_chain() {
    let db = db().await;
    let tenant = repo::create_tenant(&db, "Acme", "acme").await.unwrap();

    repo::set_setting(&db, &tenant.id, "server.bind", "127.0.0.1:8080")
        .await
        .unwrap();
    repo::set_setting(&db, &tenant.id, "server.bind", "0.0.0.0:8080")
        .await
        .unwrap();
    assert_eq!(
        repo::get_setting(&db, &tenant.id, "server.bind")
            .await
            .unwrap()
            .as_deref(),
        Some("0.0.0.0:8080")
    );
    assert_eq!(repo::list_settings(&db, &tenant.id).await.unwrap().len(), 1);

    let first = repo::append_audit(
        &db,
        &tenant.id,
        None,
        "tenant.created",
        "tenant:acme",
        &json!({"slug": "acme"}),
    )
    .await
    .unwrap();
    let second = repo::append_audit(
        &db,
        &tenant.id,
        None,
        "setting.updated",
        "setting:server.bind",
        &json!({}),
    )
    .await
    .unwrap();
    assert_eq!(second.prev_hash, first.hash);
    assert_ne!(first.hash, second.hash);
    assert_eq!(
        repo::list_audit(&db, &tenant.id, 10).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn credentials_store_only_ciphertext() {
    let db = db().await;
    let tenant = repo::create_tenant(&db, "Acme", "acme").await.unwrap();
    let credential = repo::create_credential(
        &db,
        &tenant.id,
        "Portal login",
        CredentialKind::UsernamePassword,
        Some("ada"),
        "v1:not-plaintext",
        None,
        &json!({"site": "example.test"}),
    )
    .await
    .unwrap();
    assert_eq!(credential.username.as_deref(), Some("ada"));
    assert_ne!(credential.secret_encrypted, "hunter2");
    let fetched = repo::get_credential(&db, &tenant.id, &credential.id)
        .await
        .unwrap();
    assert_eq!(fetched.id, credential.id);
    assert_eq!(
        repo::list_credentials(&db, &tenant.id).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn credentials_can_reference_external_providers() {
    let db = db().await;
    let tenant = repo::create_tenant(&db, "Acme", "acme").await.unwrap();
    let credential = repo::create_credential_with_provider(
        &db,
        &tenant.id,
        "Bitwarden login",
        CredentialKind::UsernamePassword,
        "bitwarden",
        Some("item-123"),
        Some("ada"),
        "",
        None,
        &json!({}),
    )
    .await
    .unwrap();
    assert_eq!(credential.provider, "bitwarden");
    assert_eq!(credential.external_ref.as_deref(), Some("item-123"));
    assert_eq!(credential.username.as_deref(), Some("ada"));
    let fetched = repo::get_credential(&db, &tenant.id, &credential.id)
        .await
        .unwrap();
    assert_eq!(fetched.provider, "bitwarden");
}

#[tokio::test]
async fn schedule_due_detection_and_worker_queue_leasing() {
    let db = db().await;
    let tenant = repo::create_tenant(&db, "Acme", "acme").await.unwrap();
    let definition = parse_workflow(WORKFLOW).unwrap();
    let workflow = repo::create_workflow(&db, &tenant.id, &definition, None, "test")
        .await
        .unwrap();
    let schedule = repo::create_schedule(&db, &tenant.id, &workflow.id, "* * * * *", "UTC")
        .await
        .unwrap();
    assert!(schedule.next_run_at.is_some());
    let due = repo::due_schedules(&db, "2999-01-01T00:00:00.000Z")
        .await
        .unwrap();
    assert!(due.iter().any(|item| item.id == schedule.id));

    let run = repo::create_workflow_run(
        &db,
        &tenant.id,
        &workflow.id,
        workflow.current_version,
        &json!({}),
        "manual",
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(run.status, "queued");
    let claimed = repo::claim_next_workflow_run(&db, "worker-1", "2000-01-01T00:00:00.000Z")
        .await
        .unwrap()
        .expect("run claimed");
    assert_eq!(claimed.id, run.id);
    assert_eq!(claimed.status, "running");
    let none = repo::claim_next_workflow_run(&db, "worker-2", "2000-01-01T00:00:00.000Z")
        .await
        .unwrap();
    assert!(none.is_none());

    let released = repo::release_stale_workflow_runs(&db, "2999-01-01T00:00:00.000Z")
        .await
        .unwrap();
    assert_eq!(released, 1);
    let claimed = repo::claim_next_workflow_run(&db, "worker-2", "2000-01-01T00:00:00.000Z")
        .await
        .unwrap()
        .expect("run requeued");
    assert_eq!(claimed.id, run.id);
}

#[tokio::test]
async fn approvals_and_webhooks_are_persisted() {
    let db = db().await;
    let tenant = repo::create_tenant(&db, "Acme", "acme").await.unwrap();
    let definition = parse_workflow(WORKFLOW).unwrap();
    let workflow = repo::create_workflow(&db, &tenant.id, &definition, None, "test")
        .await
        .unwrap();
    let run = repo::create_workflow_run(
        &db,
        &tenant.id,
        &workflow.id,
        workflow.current_version,
        &json!({}),
        "manual",
        None,
        None,
    )
    .await
    .unwrap();

    let approval = repo::create_approval(
        &db,
        &tenant.id,
        Some(&run.id),
        None,
        "step-1",
        "Delete account",
        "This looks destructive",
    )
    .await
    .unwrap();
    let pending = repo::list_approvals(&db, &tenant.id, Some("pending"), Some(&run.id), None, 10)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    let resolved =
        repo::resolve_approval(&db, &tenant.id, &approval.id, "approved", Some("user-1"))
            .await
            .unwrap();
    assert_eq!(resolved.status, "approved");

    let webhook = repo::create_webhook(
        &db,
        &tenant.id,
        "https://example.com/hook",
        "encrypted-secret",
        &["run.completed".to_string()],
    )
    .await
    .unwrap();
    let enabled = repo::enabled_webhooks_for_event(&db, &tenant.id, "run.completed")
        .await
        .unwrap();
    assert_eq!(enabled.len(), 1);
    let delivery = repo::record_webhook_delivery(
        &db,
        &tenant.id,
        &webhook.id,
        "run.completed",
        &json!({"runId": run.id}),
        "delivered",
        1,
        Some(200),
        None,
    )
    .await
    .unwrap();
    let deliveries = repo::list_webhook_deliveries(&db, &tenant.id, &webhook.id, 10)
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].id, delivery);
    assert_eq!(deliveries[0].status, "delivered");
}

#[tokio::test]
async fn routes_runs_to_outbound_connectors_and_reports_results() {
    let db = db().await;
    let tenant = repo::create_tenant_with_region(&db, "Acme", "acme", "eu-central")
        .await
        .unwrap();
    assert_eq!(tenant.region, "eu-central");

    let connector_key = generate_api_key();
    let connector = repo::create_connector(
        &db,
        &tenant.id,
        "vpc-connector",
        &connector_key.prefix,
        &connector_key.hash,
        "eu-central",
    )
    .await
    .unwrap();
    let found = repo::find_connector_by_prefix(&db, &connector_key.prefix)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.0.id, connector.id);
    assert!(verify_api_key(&connector_key.token, &found.1));

    let definition = parse_workflow(WORKFLOW).unwrap();
    let workflow = repo::create_workflow(&db, &tenant.id, &definition, None, "test")
        .await
        .unwrap();
    repo::set_setting(&db, &tenant.id, "execution.connector_id", &connector.id)
        .await
        .unwrap();
    let run = repo::create_workflow_run(
        &db,
        &tenant.id,
        &workflow.id,
        workflow.current_version,
        &json!({"date": "2026-11-06"}),
        "api",
        None,
        None,
    )
    .await
    .unwrap();

    assert!(
        repo::claim_next_workflow_run(&db, "local-worker", "9999")
            .await
            .unwrap()
            .is_none(),
        "local workers must not claim connector-routed runs"
    );
    let job = repo::claim_connector_job(&db, &connector, 60)
        .await
        .unwrap()
        .expect("connector job");
    assert_eq!(job.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(job.payload["kind"], "workflow");
    assert_eq!(job.payload["inputs"]["date"], "2026-11-06");
    assert_eq!(
        repo::get_workflow_run(&db, &tenant.id, &run.id)
            .await
            .unwrap()
            .status,
        "running"
    );

    let completed = repo::complete_connector_job(
        &db,
        &connector.id,
        &job.id,
        "completed",
        Some(&json!({"ok": true})),
        None,
    )
    .await
    .unwrap();
    assert_eq!(completed.status, "completed");
    let final_run = repo::get_workflow_run(&db, &tenant.id, &run.id)
        .await
        .unwrap();
    assert_eq!(final_run.status, "completed");
    assert_eq!(final_run.outputs.unwrap()["ok"], true);

    let queued = repo::create_workflow_run(
        &db,
        &tenant.id,
        &workflow.id,
        workflow.current_version,
        &json!({}),
        "api",
        None,
        None,
    )
    .await
    .unwrap();
    let _ = repo::claim_connector_job(&db, &connector, 60)
        .await
        .unwrap()
        .expect("second connector job");
    repo::delete_connector(&db, &tenant.id, &connector.id)
        .await
        .unwrap();
    assert_eq!(
        repo::get_workflow_run(&db, &tenant.id, &queued.id)
            .await
            .unwrap()
            .status,
        "cancelled",
        "deleting a connector must cancel leased work"
    );
    assert!(
        repo::claim_next_workflow_run(&db, "local-worker", "9999")
            .await
            .unwrap()
            .is_none(),
        "a cancelled connector job must never fall back to the local worker"
    );
    assert!(
        repo::create_workflow_run(
            &db,
            &tenant.id,
            &workflow.id,
            workflow.current_version,
            &json!({}),
            "api",
            None,
            None,
        )
        .await
        .is_err(),
        "a deleted connector binding must fail closed"
    );
}

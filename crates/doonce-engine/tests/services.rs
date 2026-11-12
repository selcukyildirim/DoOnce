//! Engine service wiring: AI, email, upload, nested workflows and downloads.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use doonce_browser::{BrowserSession, LaunchOptions, find_chrome};
use doonce_core::WorkflowDefinition;
use doonce_core::validation::parse_workflow;
use doonce_engine::{
    AiExecutor, AiTaskRequest, ApprovalCallback, ApprovalRequest, ArtifactStore, EmailMessage,
    EmailSender, ExecEvent, Observer, RunOptions, RunStatus, Services, WorkflowResolver,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

struct Silent;

impl Observer for Silent {
    fn on_event(&self, _: &ExecEvent) {}
    async fn approve(&self, _: ApprovalRequest) -> bool {
        true
    }
}

struct MockAi;

impl AiExecutor for MockAi {
    fn run_task<'a>(
        &'a self,
        _browser: &'a mut BrowserSession,
        _request: AiTaskRequest<'a>,
        _approvals: &'a dyn ApprovalCallback,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Ok(json!({ "completed": true })) })
    }

    fn extract<'a>(
        &'a self,
        _browser: &'a mut BrowserSession,
        _prompt: &'a str,
        _schema: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Ok(json!({ "title": "ok" })) })
    }

    fn validate<'a>(
        &'a self,
        _browser: &'a mut BrowserSession,
        _prompt: &'a str,
    ) -> BoxFuture<'a, Result<bool, String>> {
        Box::pin(async { Ok(true) })
    }

    fn prompt<'a>(
        &'a self,
        _browser: &'a mut BrowserSession,
        _prompt: &'a str,
        _schema: Option<&'a Value>,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Ok(Value::String("prompt answer".into())) })
    }

    fn text_prompt<'a>(
        &'a self,
        _prompt: &'a str,
        _model: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Ok(Value::String("hello".into())) })
    }

    fn download_files<'a>(
        &'a self,
        _browser: &'a mut BrowserSession,
        _prompt: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Ok(json!({ "files": ["report.csv"] })) })
    }
}

#[derive(Default)]
struct MockEmail {
    sent: Mutex<Vec<EmailMessage>>,
}

impl EmailSender for MockEmail {
    fn send<'a>(&'a self, message: EmailMessage) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            self.sent.lock().expect("email lock").push(message);
            Ok(json!({ "sent": true }))
        })
    }
}

struct MockArtifacts;

impl ArtifactStore for MockArtifacts {
    fn put<'a>(
        &'a self,
        artifact: &'a str,
        destination: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            Ok(json!({ "uploaded": true, "artifact": artifact, "destination": destination }))
        })
    }
}

struct MockWorkflows {
    definition: WorkflowDefinition,
}

impl WorkflowResolver for MockWorkflows {
    fn load<'a>(
        &'a self,
        _workflow_id: &'a str,
    ) -> BoxFuture<'a, Result<WorkflowDefinition, String>> {
        let definition = self.definition.clone();
        Box::pin(async move { Ok(definition) })
    }
}

#[tokio::test]
async fn runs_ai_email_upload_nested_and_download_blocks() {
    let Some(chrome) = find_chrome() else {
        eprintln!("skipping: Chrome not found");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("data.json"), r#"{"ok":true}"#).expect("artifact");

    let workflow = parse_workflow(
        r##"{
          "schemaVersion": "2.0",
          "name": "Service block test",
          "parameters": [],
          "blocks": [
            {"id":"extract","type":"extract","prompt":"Get the title","schema":{"type":"object"},"next":"validate"},
            {"id":"validate","type":"validate","prompt":"Is the page valid?","next":"check"},
            {"id":"check","type":"conditional","condition":{"==":[{"var":"outputs.validate"},true]},"then":"text","else":"fail"},
            {"id":"text","type":"textPrompt","prompt":"Say hello","saveAs":"message","next":"email"},
            {"id":"email","type":"sendEmail","to":["test@example.com"],"subject":"Report","body":"{{message}}","next":"upload"},
            {"id":"upload","type":"upload","artifact":"data.json","destination":"reports/data.json","next":"nested"},
            {"id":"nested","type":"runWorkflow","workflowId":"nested","inputs":{"name":"Ada"},"next":"download"},
            {"id":"download","type":"downloadFiles","prompt":"Download the report","saveTo":"files","next":"done"},
            {"id":"done","type":"terminate","status":"completed"},
            {"id":"fail","type":"terminate","status":"failed","reason":"validate returned false"}
          ]
        }"##,
    )
    .expect("valid workflow");

    let nested = parse_workflow(
        r#"{"schemaVersion":"2.0","name":"Nested","blocks":[{"id":"done","type":"terminate","status":"completed"}]}"#,
    )
    .expect("nested workflow");

    let email = Arc::new(MockEmail::default());
    let services = Services {
        ai: Some(Arc::new(MockAi)),
        credentials: None,
        email: Some(email.clone()),
        artifacts: Some(Arc::new(MockArtifacts)),
        workflows: Some(Arc::new(MockWorkflows { definition: nested })),
        max_depth: 3,
    };

    let mut browser = BrowserSession::launch(
        &LaunchOptions {
            executable: chrome,
            user_data_dir: dir.path().join("chrome"),
            headless: true,
        },
        &dir.path().join("downloads"),
    )
    .await
    .expect("launch Chrome");

    let options = RunOptions {
        element_timeout: Duration::from_secs(10),
        artifact_dir: dir.path().to_path_buf(),
        ..RunOptions::default()
    };
    let outcome = doonce_engine::run_with_services(
        &workflow,
        &HashMap::new(),
        &mut browser,
        &Silent,
        &options,
        &services,
    )
    .await;
    assert_eq!(
        outcome.status,
        RunStatus::Completed,
        "{:?}",
        outcome.failure
    );
    assert_eq!(outcome.outputs["extract"]["title"], "ok");
    assert_eq!(outcome.outputs["validate"], true);
    assert_eq!(outcome.outputs["message"], "hello");
    assert_eq!(outcome.outputs["email"]["sent"], true);
    assert_eq!(outcome.outputs["upload"]["uploaded"], true);
    assert_eq!(outcome.outputs["nested"]["status"], "completed");
    assert_eq!(outcome.outputs["files"]["files"][0], "report.csv");
    {
        let sent = email.sent.lock().expect("email lock");
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].subject, "Report");
        assert_eq!(sent[0].body, "hello");
    }

    browser.close().await;
}

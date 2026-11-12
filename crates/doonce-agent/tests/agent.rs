//! Agent tests with a deterministic fake LLM and a real headless browser.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use doonce_agent::{
    AutoApprove, TaskOptions, TaskSpec, TaskStatus, act, extract, prompt, run_task, validate,
};
use doonce_browser::{BrowserSession, LaunchOptions, find_chrome};
use doonce_llm::{Completion, CompletionRequest, LlmProvider};
use futures_util::future::BoxFuture;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const PAGE: &str = r#"<!doctype html>
<html>
<head><meta charset="utf-8"><title>Agent Test</title></head>
<body>
  <main>
    <h1>Agent test</h1>
    <label for="name">Name</label>
    <input id="name" name="name">
    <label for="country">Country</label>
    <select id="country" name="country">
      <option value="tr">Türkiye</option>
      <option value="us">USA</option>
    </select>
    <button id="save" onclick="document.getElementById('out').textContent =
      document.getElementById('name').value + ':' + document.getElementById('country').value;">Save</button>
    <div id="out"></div>
  </main>
</body>
</html>"#;

struct FakeProvider {
    replies: Mutex<VecDeque<String>>,
    fallback: String,
}

impl FakeProvider {
    fn actions(replies: &[&str]) -> Self {
        FakeProvider {
            replies: Mutex::new(replies.iter().map(|reply| reply.to_string()).collect()),
            fallback: r#"{"name":"Ada","country":"tr"}"#.to_string(),
        }
    }

    fn single(reply: &str) -> Self {
        FakeProvider {
            replies: Mutex::new(VecDeque::new()),
            fallback: reply.to_string(),
        }
    }
}

impl LlmProvider for FakeProvider {
    fn complete<'a>(
        &'a self,
        _request: &'a CompletionRequest,
    ) -> BoxFuture<'a, doonce_llm::Result<Completion>> {
        Box::pin(async move {
            let reply = self
                .replies
                .lock()
                .expect("fake replies lock")
                .pop_front()
                .unwrap_or_else(|| self.fallback.clone());
            Ok(Completion {
                text: reply,
                usage: None,
            })
        })
    }
}

async fn serve() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("local address");
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                PAGE.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.write_all(PAGE.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    (format!("http://{address}/"), handle)
}

async fn browser(dir: &tempfile::TempDir) -> Option<BrowserSession> {
    let chrome = find_chrome()?;
    Some(
        BrowserSession::launch(
            &LaunchOptions {
                executable: chrome,
                user_data_dir: dir.path().join("chrome"),
                headless: true,
            },
            &dir.path().join("downloads"),
        )
        .await
        .expect("launch Chrome"),
    )
}

fn options() -> TaskOptions {
    TaskOptions {
        element_timeout: Duration::from_secs(15),
        max_steps: 8,
        vision: false,
        ..TaskOptions::default()
    }
}

#[tokio::test]
async fn act_fills_and_submits_a_form() {
    let Some(chrome) = find_chrome() else {
        eprintln!("skipping: Chrome not found");
        return;
    };
    let _ = chrome;
    let (url, server) = serve().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(mut browser) = browser(&dir).await else {
        return;
    };
    browser
        .navigate(&url, Duration::from_secs(20))
        .await
        .expect("navigate");

    let provider = FakeProvider::actions(&[
        r#"{"reasoning":"fill name","action":{"type":"fill","target":{"label":"Name"},"value":"Ada"}}"#,
        r#"{"reasoning":"pick country","action":{"type":"select","target":{"label":"Country"},"value":"tr"}}"#,
        r#"{"reasoning":"save","action":{"type":"click","target":{"role":"button","name":"Save"}}}"#,
        r#"{"reasoning":"done","done":true}"#,
    ]);
    let outcome = act(
        &provider,
        "Fill and submit the form",
        &mut browser,
        &AutoApprove,
        &options(),
    )
    .await;
    assert_eq!(
        outcome.status,
        TaskStatus::Completed,
        "{:?}",
        outcome.failure
    );

    let value = browser
        .query("extract", json!({ "target": { "selectors": ["#out"] } }))
        .await
        .expect("extract");
    assert_eq!(value["value"].as_str(), Some("Ada:tr"));
    browser.close().await;
    server.abort();
}

#[tokio::test]
async fn run_task_extracts_with_schema() {
    let Some(_chrome) = find_chrome() else {
        eprintln!("skipping: Chrome not found");
        return;
    };
    let (url, server) = serve().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(mut browser) = browser(&dir).await else {
        return;
    };

    let provider = FakeProvider::actions(&[
        r#"{"reasoning":"fill name","action":{"type":"fill","target":{"label":"Name"},"value":"Ada"}}"#,
        r#"{"reasoning":"pick country","action":{"type":"select","target":{"label":"Country"},"value":"tr"}}"#,
        r#"{"reasoning":"save","action":{"type":"click","target":{"role":"button","name":"Save"}}}"#,
        r#"{"reasoning":"done","done":true}"#,
    ]);
    let spec = TaskSpec {
        prompt: "Fill the form and return the name and country".into(),
        url: Some(url),
        data_schema: Some(json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "country": { "type": "string" }
            },
            "required": ["name", "country"]
        })),
        error_codes: Vec::new(),
        max_steps: 8,
        model: None,
    };
    let outcome = run_task(&provider, &spec, &mut browser, &AutoApprove, &options()).await;
    assert_eq!(
        outcome.status,
        TaskStatus::Completed,
        "{:?}",
        outcome.failure
    );
    assert_eq!(outcome.output.as_ref().unwrap()["name"], "Ada");
    assert_eq!(outcome.output.as_ref().unwrap()["country"], "tr");
    browser.close().await;
    server.abort();
}

#[tokio::test]
async fn extract_validate_and_prompt_use_the_provider() {
    let Some(_chrome) = find_chrome() else {
        eprintln!("skipping: Chrome not found");
        return;
    };
    let (url, server) = serve().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(mut browser) = browser(&dir).await else {
        return;
    };
    browser
        .navigate(&url, Duration::from_secs(20))
        .await
        .expect("navigate");

    let schema = json!({
        "type": "object",
        "properties": { "title": { "type": "string" } },
        "required": ["title"]
    });
    let extractor = FakeProvider::single(r#"{"title":"Agent Test"}"#);
    let extracted = extract(
        &extractor,
        &mut browser,
        "Get the title",
        &schema,
        &options(),
    )
    .await;
    assert_eq!(extracted.unwrap()["title"], "Agent Test");

    let validator = FakeProvider::single(r#"{"valid":true,"reasoning":"ok"}"#);
    assert!(
        validate(&validator, &mut browser, "The page has a title", &options())
            .await
            .unwrap()
    );

    let prompter = FakeProvider::single(r#"{"answer":"hello from the page"}"#);
    let answer = prompt(&prompter, &mut browser, "Say hello", None, &options())
        .await
        .unwrap();
    assert_eq!(answer.as_str(), Some("hello from the page"));

    browser.close().await;
    server.abort();
}

//! End-to-end deterministic replay against a local page.
//!
//! Requires Google Chrome (or Chromium) to be installed. The test skips, rather
//! than fails, on machines without a browser.

use std::collections::HashMap;

use doonce_browser::{BrowserSession, LaunchOptions, find_chrome};
use doonce_core::validation::parse_workflow;
use doonce_engine::{ApprovalRequest, ExecEvent, Observer, RunOptions, RunStatus};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const PAGE: &str = r#"<!doctype html>
<html>
<head><meta charset="utf-8"><title>DoOnce Replay Test</title></head>
<body>
  <main>
    <h1>Contact</h1>
    <label for="name">Name</label>
    <input id="name" name="name" autocomplete="name">
    <label for="country">Country</label>
    <select id="country" name="country">
      <option value="tr">Türkiye</option>
      <option value="us">USA</option>
    </select>
    <label for="terms">Terms</label>
    <input id="terms" name="terms" type="checkbox">
    <button id="save" onclick="document.getElementById('out').textContent =
      document.getElementById('name').value + ':' +
      document.getElementById('country').value + ':' +
      document.getElementById('terms').checked;">Save</button>
    <div id="out"></div>
  </main>
</body>
</html>"#;

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

struct Silent;

impl Observer for Silent {
    fn on_event(&self, _: &ExecEvent) {}
    async fn approve(&self, _: ApprovalRequest) -> bool {
        true
    }
}

#[tokio::test]
async fn replays_a_form_workflow_and_extracts_output() {
    let Some(chrome) = find_chrome() else {
        eprintln!("skipping: Chrome not found");
        return;
    };
    let (base_url, server) = serve().await;
    let workflow = parse_workflow(
        r##"{
          "schemaVersion": "2.0",
          "name": "Replay test",
          "parameters": [
            {"name": "base", "type": "string"},
            {"name": "person", "type": "string", "default": "Ada"}
          ],
          "blocks": [
            {"id":"open","type":"browserAction","action":{"kind":"navigate","url":"{{base}}"},"next":"name"},
            {"id":"name","type":"browserAction","action":{"kind":"fill","target":{"label":"Name"},"value":"{{person}}"},"next":"country"},
            {"id":"country","type":"browserAction","action":{"kind":"select","target":{"label":"Country"},"value":"tr"},"next":"terms"},
            {"id":"terms","type":"browserAction","action":{"kind":"check","target":{"label":"Terms"},"checked":true},"next":"save"},
            {"id":"save","type":"browserAction","action":{"kind":"click","target":{"role":"button","name":"Save"}},"next":"wait"},
            {"id":"wait","type":"wait","condition":{"textContains":"Ada:tr:true"},"next":"extract"},
            {"id":"extract","type":"browserAction","action":{"kind":"extract","target":{"selectors":["#out"]},"var":"result"},"next":"done"},
            {"id":"done","type":"terminate","status":"completed"}
          ]
        }"##,
    )
    .expect("valid workflow");

    let dir = tempfile::tempdir().expect("tempdir");
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

    let params = HashMap::from([
        ("base".to_string(), base_url),
        ("person".to_string(), "Ada".to_string()),
    ]);
    let options = RunOptions {
        element_timeout: std::time::Duration::from_secs(15),
        navigation_timeout: std::time::Duration::from_secs(20),
        ..RunOptions::default()
    };
    let outcome = doonce_engine::run(&workflow, &params, &mut browser, &Silent, &options).await;
    assert_eq!(
        outcome.status,
        RunStatus::Completed,
        "{:?}",
        outcome.failure
    );
    assert_eq!(
        outcome
            .outputs
            .get("result")
            .and_then(|value| value.as_str()),
        Some("Ada:tr:true")
    );

    let screenshot = browser.screenshot().await.expect("screenshot");
    assert!(
        screenshot.starts_with(b"\x89PNG"),
        "screenshot is not a PNG"
    );
    browser.close().await;
    server.abort();
}

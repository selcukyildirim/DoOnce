//! Control-flow, HTTP, code and file-parse block tests.

use std::collections::HashMap;
use std::time::Duration;

use doonce_browser::{BrowserSession, LaunchOptions, find_chrome};
use doonce_core::validation::parse_workflow;
use doonce_engine::{ApprovalRequest, ExecEvent, Observer, RunOptions, RunStatus};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct Silent;

impl Observer for Silent {
    fn on_event(&self, _: &ExecEvent) {}
    async fn approve(&self, _: ApprovalRequest) -> bool {
        true
    }
}

async fn serve_json() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind server");
    let address = listener.local_addr().expect("address");
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer).await;
            let body = r#"{"items":[1,2,3]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    (format!("http://{address}"), handle)
}

#[tokio::test]
async fn runs_http_code_loop_conditional_and_file_parse() {
    let Some(chrome) = find_chrome() else {
        eprintln!("skipping: Chrome not found");
        return;
    };
    let (base, server) = serve_json().await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("data.json"), r#"{"ok":true,"count":3}"#)
        .expect("write artifact");

    let workflow = parse_workflow(
        r##"{
          "schemaVersion": "2.0",
          "name": "Block engine test",
          "parameters": [{"name":"base","type":"string"}],
          "blocks": [
            {"id":"http","type":"httpRequest","method":"GET","url":"{{base}}/data","saveAs":"data","next":"code"},
            {"id":"code","type":"code","language":"javascript",
             "code":"const items = input.data.body.items; globalThis.result = 0; for (let i = 0; i < items.length; i++) { globalThis.result += items[i]; }",
             "saveAs":"sum","next":"loop"},
            {"id":"loop","type":"forLoop","over":"{{data.body.items}}","body":"double","itemName":"item","maxIterations":10,"next":"check"},
            {"id":"double","type":"code","language":"javascript","code":"globalThis.result = input.item * 2;","next":"loop"},
            {"id":"check","type":"conditional","condition":{"in":[6,{"var":"outputs.loop"}]},"then":"file","else":"fail"},
            {"id":"file","type":"fileParse","artifact":"data.json","parser":"json","saveAs":"fileData","next":"done"},
            {"id":"done","type":"terminate","status":"completed"},
            {"id":"fail","type":"terminate","status":"failed","reason":"conditional failed"}
          ]
        }"##,
    )
    .expect("valid workflow");

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

    let params = HashMap::from([("base".to_string(), base)]);
    let options = RunOptions {
        element_timeout: Duration::from_secs(10),
        artifact_dir: dir.path().to_path_buf(),
        allow_private_http: true,
        ..RunOptions::default()
    };
    let outcome = doonce_engine::run(&workflow, &params, &mut browser, &Silent, &options).await;
    assert_eq!(
        outcome.status,
        RunStatus::Completed,
        "{:?}",
        outcome.failure
    );
    assert_eq!(outcome.outputs["sum"], 6);
    assert_eq!(outcome.outputs["loop"], json!([2, 4, 6]));
    assert_eq!(outcome.outputs["fileData"]["ok"], true);
    assert_eq!(outcome.outputs["check"], true);

    browser.close().await;
    server.abort();
}

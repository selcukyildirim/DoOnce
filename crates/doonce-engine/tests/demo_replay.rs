//! Replays a workflow against the bundled demo site in headless Chrome.
//! Requires Google Chrome and network access to localhost.

use std::collections::HashMap;
use std::time::Duration;

use doonce_core::{ResolvedParams, parse_workflow};
use doonce_engine::cdp::{CdpDriver, LaunchOptions, find_chrome};
use doonce_engine::{Observer, RunOptions, RunStatus, run};

struct Silent;

impl Observer for Silent {
    fn on_event(&self, _: &doonce_engine::ExecEvent) {}

    async fn approve(&self, _: doonce_engine::ApprovalRequest) -> bool {
        true
    }
}

#[tokio::test]
async fn replays_contact_form_on_demo_site() {
    let chrome = match find_chrome() {
        Some(p) => p,
        None => {
            eprintln!("skipping: Chrome not found");
            return;
        }
    };

    let json = include_str!("../../../examples/workflows/contact-demo.json");
    let workflow = parse_workflow(json).expect("valid workflow");

    let (listener, addr) = doonce_cli::demo::bind(0).await.expect("demo server");
    tokio::spawn(doonce_cli::demo::serve(listener));

    // Point workflow at our ephemeral port.
    let url = format!("http://{addr}/contact.html?variant=classic");
    let mut workflow = workflow;
    if let doonce_core::Action::Navigate { url: u } = &mut workflow.steps[0].action {
        *u = url;
    }

    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("chrome");
    let downloads = dir.path().join("dl");
    let mut driver = CdpDriver::launch(
        &LaunchOptions {
            executable: chrome,
            user_data_dir: profile,
            headless: true,
        },
        &downloads,
    )
    .await
    .expect("launch chrome");

    let params = ResolvedParams::resolve(&workflow, &HashMap::new()).unwrap();
    let outcome = run(
        &workflow,
        &params,
        &mut driver,
        &Silent,
        &RunOptions {
            element_timeout: Duration::from_secs(15),
            navigation_timeout: Duration::from_secs(20),
            ..RunOptions::default()
        },
    )
    .await;
    driver.close().await;

    assert_eq!(outcome.status, RunStatus::Completed, "{:?}", outcome.failure);
}

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use doonce_core::risk::{approval_reason, harden_imported};
use doonce_core::sanitize::sanitize_workflow;
use doonce_core::{ParamType, Workflow, parse_workflow};
use doonce_engine::cdp::{CdpDriver, LaunchOptions, find_chrome};
use doonce_engine::{ApprovalRequest, ExecEvent, Observer, RunOptions, RunStatus, StepStatus};
use doonce_storage::Storage;

use doonce_cli::{demo, native_host, runner, setup};

#[derive(Parser)]
#[command(name = "doonce", version, about = "Teach once. Run forever. Fix itself.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List saved automations
    List,
    /// Print an automation's steps
    Show { workflow: String },
    /// Run an automation in Chrome
    Run {
        workflow: String,
        /// Parameter value, e.g. --param reportDate=2026-10-15 (repeatable)
        #[arg(short, long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
        /// Run without a visible browser window
        #[arg(long)]
        headless: bool,
        /// Use an already running Chrome (started with --remote-debugging-port), e.g. http://127.0.0.1:9222
        #[arg(long, value_name = "URL")]
        cdp: Option<String>,
        /// Chrome executable to launch
        #[arg(long, value_name = "PATH")]
        chrome: Option<PathBuf>,
        /// Chrome profile directory (logins made there are kept between runs)
        #[arg(long, value_name = "DIR")]
        profile: Option<PathBuf>,
        /// Where downloaded files are saved
        #[arg(long, value_name = "DIR")]
        download_dir: Option<PathBuf>,
        /// Seconds to wait for each element
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Show recent runs of an automation
    History {
        workflow: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Include per-step results
        #[arg(long)]
        steps: bool,
    },
    /// Check a workflow JSON file without importing it
    Validate { file: PathBuf },
    /// Write an automation as shareable JSON (stdout or --output)
    Export {
        workflow: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Import a workflow JSON file
    Import { file: PathBuf },
    /// Delete an automation and its history
    Delete {
        workflow: String,
        #[arg(long)]
        yes: bool,
    },
    /// List saved versions of an automation
    Revisions { workflow: String },
    /// Restore an earlier version (kept as a new version)
    Rollback { workflow: String, version: i64 },
    /// Connect the browser extension to this installation
    Setup {
        /// Extension id to allow (defaults to the bundled extension)
        #[arg(long = "extension-id")]
        extension_ids: Vec<String>,
        /// Install into a specific Chrome profile directory only
        #[arg(long, value_name = "DIR")]
        profile_dir: Option<PathBuf>,
        /// Remove the browser connection
        #[arg(long)]
        uninstall: bool,
    },
    /// Serve the demo site for trying DoOnce safely
    Demo {
        #[arg(long, default_value_t = 4317)]
        port: u16,
    },
    /// Print the JSON Schema of the workflow format
    Schema,
}

fn main() {
    // Chrome starts native messaging hosts with the extension origin as the
    // first argument (and on Windows a --parent-window flag).
    let args: Vec<String> = std::env::args().collect();
    if let Some(origin) = args.iter().skip(1).find(|a| a.starts_with("chrome-extension://")) {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        if let Err(e) = rt.block_on(native_host::run(origin)) {
            eprintln!("doonce host error: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    match rt.block_on(dispatch(cli.command)) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}

fn open_storage() -> Result<Storage> {
    Storage::open_default().context("could not open the DoOnce database")
}

fn read_workflow_file(file: &PathBuf) -> Result<Workflow> {
    let text = std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    parse_workflow(&text).map_err(|issues| {
        anyhow!(
            "{} is not a valid workflow:\n{}",
            file.display(),
            issues.iter().map(|i| format!("  - {i}")).collect::<Vec<_>>().join("\n")
        )
    })
}

async fn dispatch(command: Command) -> Result<i32> {
    match command {
        Command::List => {
            let list = open_storage()?.list_workflows()?;
            if list.is_empty() {
                println!("No automations yet. Record one with the DoOnce Chrome extension, or `doonce import <file>`.");
                return Ok(0);
            }
            println!("{:<28} {:<32} {:>5}  {:<22} STATUS", "ID", "NAME", "STEPS", "LAST RUN");
            for w in list {
                println!(
                    "{:<28} {:<32} {:>5}  {:<22} {}",
                    w.slug,
                    truncate(&w.name, 32),
                    w.step_count,
                    w.last_run_at.as_deref().map(short_time).unwrap_or("never".into()),
                    w.last_status.unwrap_or_else(|| "-".into())
                );
            }
            Ok(0)
        }
        Command::Show { workflow } => {
            let r = open_storage()?.get_workflow(&workflow)?;
            println!("{} ({}, version {})", r.workflow.name, r.slug, r.version);
            for p in &r.workflow.parameters {
                let default = p.default.as_deref().map(|d| format!(" = {d}")).unwrap_or_default();
                println!("  param {} : {:?}{}", p.name, p.kind, default);
            }
            for s in &r.workflow.steps {
                let flag = if approval_reason(s).is_some() { "  [needs approval]" } else { "" };
                println!("  {:<10} {}{}", s.id, s.summary(), flag);
            }
            Ok(0)
        }
        Command::Run { workflow, params, headless, cdp, chrome, profile, download_dir, timeout } => {
            run_workflow(&workflow, &params, headless, cdp, chrome, profile, download_dir, timeout).await
        }
        Command::History { workflow, limit, steps } => {
            let storage = open_storage()?;
            let runs = storage.history(&workflow, limit)?;
            if runs.is_empty() {
                println!("No runs yet.");
            }
            for e in runs {
                println!(
                    "#{:<5} {:<22} v{:<3} {:<10} {:<9} {}",
                    e.id,
                    short_time(&e.started_at),
                    e.workflow_revision,
                    e.status,
                    e.source,
                    e.error.unwrap_or_default()
                );
                if steps {
                    for s in storage.execution_steps(e.id)? {
                        println!(
                            "        {:<10} {:<10} {:>6} ms  {}",
                            s.step_id,
                            s.status,
                            s.duration_ms,
                            s.error.unwrap_or_default()
                        );
                    }
                }
            }
            Ok(0)
        }
        Command::Validate { file } => {
            let w = read_workflow_file(&file)?;
            println!("✓ {} is valid: \"{}\" with {} steps", file.display(), w.name, w.steps.len());
            Ok(0)
        }
        Command::Export { workflow, output } => {
            let mut w = open_storage()?.get_workflow(&workflow)?.workflow;
            sanitize_workflow(&mut w);
            let json = w.to_pretty_json() + "\n";
            match output {
                Some(path) => {
                    std::fs::write(&path, json)?;
                    eprintln!("Exported to {}", path.display());
                }
                None => print!("{json}"),
            }
            Ok(0)
        }
        Command::Import { file } => {
            let mut w = read_workflow_file(&file)?;
            let mut notes = sanitize_workflow(&mut w);
            notes.extend(harden_imported(&mut w));
            let record = open_storage()?.create_workflow(&w, "Imported")?;
            for n in notes {
                println!("  note: {n}");
            }
            println!("✓ Imported \"{}\" as {}", record.workflow.name, record.slug);
            Ok(0)
        }
        Command::Delete { workflow, yes } => {
            let mut storage = open_storage()?;
            let r = storage.get_workflow(&workflow)?;
            if !yes && !confirm(&format!("Delete \"{}\" and its history?", r.workflow.name))? {
                return Ok(1);
            }
            storage.delete_workflow(&r.slug)?;
            println!("Deleted {}", r.slug);
            Ok(0)
        }
        Command::Revisions { workflow } => {
            let storage = open_storage()?;
            let current = storage.get_workflow(&workflow)?.version;
            for r in storage.revisions(&workflow)? {
                let mark = if r.version == current { "*" } else { " " };
                println!("{mark} v{:<4} {:<22} {}", r.version, short_time(&r.created_at), r.reason);
            }
            Ok(0)
        }
        Command::Rollback { workflow, version } => {
            let r = open_storage()?.rollback(&workflow, version)?;
            println!("✓ {} restored from v{version} (now v{})", r.slug, r.version);
            Ok(0)
        }
        Command::Setup { extension_ids, profile_dir, uninstall } => {
            if uninstall {
                for p in setup::uninstall()? {
                    println!("Removed {}", p.display());
                }
                return Ok(0);
            }
            let ids =
                if extension_ids.is_empty() { vec![setup::DEFAULT_EXTENSION_ID.to_string()] } else { extension_ids };
            let report = setup::install(&ids, profile_dir.as_deref())?;
            for (browser, path) in &report.installed {
                println!("✓ {browser}: {}", path.display());
            }
            println!("\nDoOnce is connected. Reload the extension (or restart the browser) if it was already open.");
            Ok(0)
        }
        Command::Demo { port } => {
            let (listener, addr) = demo::bind(port).await.context("could not start the demo server")?;
            println!("Demo site running at http://{addr}/  (Ctrl+C to stop)");
            tokio::select! {
                _ = demo::serve(listener) => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            Ok(0)
        }
        Command::Schema => {
            let schema = schemars::schema_for!(Workflow);
            println!("{}", serde_json::to_string_pretty(&schema)?);
            Ok(0)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_workflow(
    reference: &str,
    raw_params: &[String],
    headless: bool,
    cdp: Option<String>,
    chrome: Option<PathBuf>,
    profile: Option<PathBuf>,
    download_dir: Option<PathBuf>,
    timeout: u64,
) -> Result<i32> {
    let storage = Arc::new(Mutex::new(open_storage()?));
    let record = storage.lock().unwrap().get_workflow(reference)?;

    let mut provided = HashMap::new();
    for p in raw_params {
        let (k, v) = p.split_once('=').ok_or_else(|| anyhow!("--param must look like NAME=VALUE, got \"{p}\""))?;
        provided.insert(k.trim().to_string(), v.to_string());
    }
    for p in &record.workflow.parameters {
        if provided.contains_key(&p.name) {
            continue;
        }
        let env = format!("DOONCE_PARAM_{}", p.name.to_ascii_uppercase());
        if let Ok(v) = std::env::var(&env) {
            provided.insert(p.name.clone(), v);
        } else if p.kind == ParamType::Secret && std::io::stdin().is_terminal() {
            let v = rpassword::prompt_password(format!("{} (hidden): ", p.description.as_deref().unwrap_or(&p.name)))?;
            provided.insert(p.name.clone(), v);
        } else if p.kind == ParamType::Secret {
            bail!("secret parameter \"{}\" is required: set {env} or run interactively", p.name);
        }
    }

    let (params, execution_id) = runner::prepare(&storage, &record, &provided, "cli")?;

    let downloads = download_dir
        .unwrap_or_else(|| dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")).join("DoOnce"));
    let mut driver = match cdp {
        Some(endpoint) => CdpDriver::connect(&endpoint, &downloads).await?,
        None => {
            let executable = chrome
                .or_else(find_chrome)
                .ok_or_else(|| anyhow!("Chrome was not found. Install Google Chrome or pass --chrome <path>."))?;
            let user_data_dir = profile.unwrap_or_else(|| doonce_storage::data_dir().join("chrome-profile"));
            CdpDriver::launch(&LaunchOptions { executable, user_data_dir, headless }, &downloads).await?
        }
    };

    let options = RunOptions { element_timeout: std::time::Duration::from_secs(timeout), ..RunOptions::default() };
    let cancel = options.cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nStopping…");
            cancel.cancel();
        }
    });

    println!("▶ {} (v{})", record.workflow.name, record.version);
    let observer = CliObserver {
        summaries: record.workflow.steps.iter().map(|s| (s.id.clone(), s.summary())).collect(),
        total: record.workflow.steps.len(),
    };
    let outcome = runner::execute(&storage, execution_id, &record, &params, &mut driver, &observer, &options).await;
    driver.close().await;

    match outcome.status {
        RunStatus::Completed => {
            println!("✓ Completed");
            Ok(0)
        }
        RunStatus::Cancelled => {
            println!("■ Cancelled");
            Ok(130)
        }
        RunStatus::Failed => {
            let f = outcome.failure.expect("failure");
            println!("✗ Failed at {}: {}", outcome.failed_step.unwrap_or_default(), f.message);
            Ok(1)
        }
    }
}

struct CliObserver {
    summaries: HashMap<String, String>,
    total: usize,
}

impl Observer for CliObserver {
    fn on_event(&self, event: &ExecEvent) {
        match event {
            ExecEvent::Step { index, step_id, status, error, duration_ms } => {
                let summary = self.summaries.get(step_id).map(String::as_str).unwrap_or("");
                let n = format!("[{}/{}]", index + 1, self.total);
                match status {
                    StepStatus::Completed => println!("  ✓ {n} {summary}  ({duration_ms} ms)"),
                    StepStatus::Failed => println!("  ✗ {n} {summary}\n      {}", error.as_deref().unwrap_or("")),
                    StepStatus::Cancelled => println!("  ■ {n} {summary}  {}", error.as_deref().unwrap_or("")),
                    _ => {}
                }
            }
            ExecEvent::Download { info, .. } => match &info.path {
                Some(p) => println!("      ↓ saved {}", p.display()),
                None => println!("      ↓ downloaded {}", info.filename),
            },
        }
    }

    async fn approve(&self, request: ApprovalRequest) -> bool {
        if !std::io::stdin().is_terminal() {
            eprintln!(
                "  ! \"{}\" needs approval ({}) but no one can answer. Run interactively, or set \"requiresApproval\": false on {} if it is safe.",
                request.summary, request.reason, request.step_id
            );
            return false;
        }
        let question = format!("  ? {} — {} Run this step?", request.summary, request.reason);
        tokio::task::spawn_blocking(move || confirm(&question).unwrap_or(false)).await.unwrap_or(false)
    }
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { s.chars().take(max - 1).collect::<String>() + "…" }
}

/// `2026-10-15T14:03:22.123Z` → `2026-10-15 14:03 UTC`.
fn short_time(ts: &str) -> String {
    match (ts.get(0..10), ts.get(11..16)) {
        (Some(d), Some(t)) => format!("{d} {t} UTC"),
        _ => ts.to_string(),
    }
}

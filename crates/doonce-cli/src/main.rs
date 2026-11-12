use std::collections::{HashMap, HashSet};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use doonce_agent::{
    ApprovalRequest as TaskApprovalRequest, TaskEvent, TaskObserver, TaskOptions, TaskSpec,
    TaskStatus,
};
use doonce_browser::{BrowserSession, LaunchOptions, find_chrome};
use doonce_cli::{
    config_get, config_list, config_set, connect_and_migrate, database_url_redacted, doctor,
    init_tracing, quickstart,
};
use doonce_core::validation::parse_workflow;
use doonce_core::{CredentialKind, ParamType, WorkflowDefinition};
use doonce_engine::{
    ApprovalRequest, CredentialResolver, ExecEvent, Observer, RunOptions, RunStatus, StepStatus,
};
use doonce_llm::{Provider, ProviderConfig};
use doonce_security::MasterKey;
use doonce_services::CredentialService;
use doonce_store::{Database, repo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[derive(Parser)]
#[command(
    name = "doonce",
    version,
    about = "DoOnce Enterprise — browser automation with AI",
    long_about = "DoOnce runs natural-language browser tasks and deterministic workflows. \
                  This binary bootstraps the local control plane, drives Chrome and replays workflows.",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize the database, admin identity and first API key
    Quickstart {
        /// Admin email address (defaults to admin@localhost)
        #[arg(long)]
        email: Option<String>,
    },
    /// Read or write non-secret runtime settings
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Database maintenance
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
    /// Inspect or drive a browser session
    Browser {
        #[command(subcommand)]
        command: BrowserCommand,
    },
    /// Replay a deterministic workflow file
    Replay {
        file: PathBuf,
        /// Parameter value, e.g. --param reportDate=2026-11-06 (repeatable)
        #[arg(short, long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
        /// Run without a visible browser window
        #[arg(long)]
        headless: bool,
        /// Connect to an already running Chrome (e.g. http://127.0.0.1:9222)
        #[arg(long, value_name = "URL")]
        cdp: Option<String>,
        /// Chrome executable to launch
        #[arg(long, value_name = "PATH")]
        chrome: Option<PathBuf>,
        /// Chrome profile directory (logins persist between runs)
        #[arg(long, value_name = "DIR")]
        profile: Option<PathBuf>,
        /// Where downloaded files are saved
        #[arg(long, value_name = "DIR")]
        download_dir: Option<PathBuf>,
        /// Seconds to wait for each element
        #[arg(long, default_value_t = 10)]
        timeout: u64,
        /// Directory that fileParse blocks may read from
        #[arg(long, value_name = "DIR")]
        artifact_dir: Option<PathBuf>,
        /// Allow httpRequest blocks to reach private/loopback addresses
        #[arg(long)]
        allow_private_http: bool,
    },
    /// Validate or inspect a workflow file without running it
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },
    /// Run a natural-language browser task
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Run an AI page command (act/extract/validate/prompt)
    Page {
        #[command(subcommand)]
        command: PageCommand,
    },
    /// Inspect the configured LLM provider
    Llm {
        #[command(subcommand)]
        command: LlmCommand,
    },
    /// Manage stored and external credentials
    Credential {
        #[command(subcommand)]
        command: CredentialCommand,
    },
    /// Run the background worker (queued runs, schedules, webhooks)
    Worker {
        #[arg(long)]
        concurrency: Option<usize>,
        #[arg(long)]
        poll_secs: Option<u64>,
        #[arg(long)]
        lease_secs: Option<i64>,
        /// Approve risky steps automatically (trusted automation only)
        #[arg(long)]
        auto_approve: bool,
    },
    /// Run the outbound-only hybrid data-plane connector
    Connector {
        /// Control-plane base URL (default: DOONCE_CONTROL_URL)
        #[arg(long, value_name = "URL")]
        control_url: Option<String>,
        /// Connector token (default: DOONCE_CONNECTOR_TOKEN)
        #[arg(long, env = "DOONCE_CONNECTOR_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// Local data-plane API URL (default: DOONCE_CONNECTOR_LOCAL_URL)
        #[arg(long, value_name = "URL")]
        local_url: Option<String>,
        /// Local data-plane API key (default: DOONCE_CONNECTOR_LOCAL_API_KEY)
        #[arg(long, env = "DOONCE_CONNECTOR_LOCAL_API_KEY", hide_env_values = true)]
        local_api_key: Option<String>,
        #[arg(long)]
        poll_secs: Option<u64>,
        #[arg(long)]
        lease_secs: Option<i64>,
        /// Approve local approval gates automatically
        #[arg(long)]
        auto_approve: bool,
    },
    /// Start a Model Context Protocol (MCP) server on stdio
    Mcp {
        /// Control-plane base URL (default: DOONCE_API_URL or http://127.0.0.1:8080)
        #[arg(long, value_name = "URL")]
        url: Option<String>,
        /// API key (default: DOONCE_API_KEY)
        #[arg(long, env = "DOONCE_API_KEY", hide_env_values = true)]
        api_key: Option<String>,
        /// Hide tools that create, run or cancel work
        #[arg(long)]
        read_only: bool,
    },
    /// Serve the REST API and (if built) the web console
    Serve {
        #[arg(long, value_name = "ADDR")]
        bind: Option<String>,
        #[arg(long, value_name = "DIR")]
        ui_dir: Option<PathBuf>,
        /// DANGER: skip API key auth and act as the default tenant owner
        #[arg(long)]
        dev_no_auth: bool,
    },
    /// Check the local installation and print actionable warnings
    Doctor,
    /// Print version and build information
    Version,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print one setting
    Get { key: String },
    /// Set one non-secret setting
    Set { key: String, value: String },
    /// List all settings
    List,
}

#[derive(Subcommand)]
enum DbCommand {
    /// Apply all pending migrations
    Migrate,
}

#[derive(Subcommand)]
enum BrowserCommand {
    /// Launch or connect to Chrome and print the current page
    Check {
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        headless: bool,
        #[arg(long, value_name = "URL")]
        cdp: Option<String>,
        #[arg(long, value_name = "PATH")]
        chrome: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        profile: Option<PathBuf>,
    },
    /// Print the page agent's observed interactive elements as JSON
    Observe {
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        headless: bool,
        #[arg(long, value_name = "URL")]
        cdp: Option<String>,
        #[arg(long, value_name = "PATH")]
        chrome: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        profile: Option<PathBuf>,
    },
    /// Serve a local live view of a browser session (1 fps screenshots)
    Stream {
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value_t = 8099)]
        port: u16,
        #[arg(long)]
        headless: bool,
        #[arg(long, value_name = "URL")]
        cdp: Option<String>,
        #[arg(long, value_name = "PATH")]
        chrome: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        profile: Option<PathBuf>,
    },
    /// Capture a PNG screenshot of a page
    Screenshot {
        url: String,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        headless: bool,
        #[arg(long, value_name = "URL")]
        cdp: Option<String>,
        #[arg(long, value_name = "PATH")]
        chrome: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        profile: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum WorkflowCommand {
    /// Validate a workflow JSON file
    Validate { file: PathBuf },
    /// Print a workflow's parameters and blocks
    Show { file: PathBuf },
}

#[derive(Args, Debug)]
struct BrowserArgs {
    #[arg(long)]
    headless: bool,
    #[arg(long, value_name = "URL")]
    cdp: Option<String>,
    #[arg(long, value_name = "PATH")]
    chrome: Option<PathBuf>,
    #[arg(long, value_name = "DIR")]
    profile: Option<PathBuf>,
    #[arg(long, value_name = "DIR")]
    download_dir: Option<PathBuf>,
    /// Seconds to wait for each element
    #[arg(long, default_value_t = 15)]
    timeout: u64,
}

#[derive(Args, Debug)]
struct LlmArgs {
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    /// Disable screenshot/vision input even if the model supports it
    #[arg(long)]
    no_vision: bool,
}

#[derive(Subcommand)]
enum TaskCommand {
    /// Run a task from a natural-language prompt
    Run {
        prompt: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long, value_name = "FILE")]
        schema: Option<PathBuf>,
        #[arg(long, value_name = "FILE")]
        error_codes: Option<PathBuf>,
        #[arg(long, default_value_t = 30)]
        max_steps: u32,
        #[command(flatten)]
        browser: BrowserArgs,
        #[command(flatten)]
        llm: LlmArgs,
    },
}

#[derive(Subcommand)]
enum PageCommand {
    /// Perform a natural-language instruction on the current page
    Act {
        instruction: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value_t = 15)]
        max_steps: u32,
        #[command(flatten)]
        browser: BrowserArgs,
        #[command(flatten)]
        llm: LlmArgs,
    },
    /// Extract structured data using a JSON Schema
    Extract {
        prompt: String,
        #[arg(long, value_name = "FILE")]
        schema: PathBuf,
        #[arg(long)]
        url: Option<String>,
        #[command(flatten)]
        browser: BrowserArgs,
        #[command(flatten)]
        llm: LlmArgs,
    },
    /// Validate the current page against a requirement
    Validate {
        prompt: String,
        #[arg(long)]
        url: Option<String>,
        #[command(flatten)]
        browser: BrowserArgs,
        #[command(flatten)]
        llm: LlmArgs,
    },
    /// Send a free-form prompt to the model with the page as context
    Prompt {
        prompt: String,
        #[arg(long, value_name = "FILE")]
        schema: Option<PathBuf>,
        #[arg(long)]
        url: Option<String>,
        #[command(flatten)]
        browser: BrowserArgs,
        #[command(flatten)]
        llm: LlmArgs,
    },
}

#[derive(Subcommand)]
enum CredentialCommand {
    /// List credential metadata (never secrets)
    List,
    /// Add a local or external credential
    Add {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "username_password")]
        kind: String,
        #[arg(long, default_value = "local")]
        provider: String,
        #[arg(long = "external-ref")]
        external_ref: Option<String>,
        #[arg(long)]
        username: Option<String>,
        #[arg(long)]
        secret: Option<String>,
        #[arg(long = "totp-secret")]
        totp_secret: Option<String>,
    },
    /// Delete a credential
    Remove {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Resolve a credential without printing the secret
    Test { id: String },
}

#[derive(Subcommand)]
enum LlmCommand {
    /// Validate provider configuration without making a paid request
    Check {
        #[command(flatten)]
        llm: LlmArgs,
    },
}

#[tokio::main]
async fn main() {
    init_tracing();
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Quickstart { email } => {
            let report = quickstart(email.as_deref()).await?;
            println!("DoOnce Enterprise is ready.\n");
            println!("  Data directory : {}", report.data_dir.display());
            println!(
                "  Database       : {} ({:?})",
                report.database_url, report.dialect
            );
            println!(
                "  Tenant         : {} ({})",
                report.tenant_name, report.tenant_slug
            );
            println!("  Admin          : {}", report.email);
            println!("  API key        : {}", report.api_key);
            if report.created_tenant {
                println!("\n  Created the default tenant.");
            }
            if report.created_user {
                println!("  Created the administrator identity.");
            }
            println!(
                "\nThe API key is shown once. Store it in a secret manager and never commit it."
            );
            println!("Next: `doonce doctor`");
        }
        Command::Config { command } => match command {
            ConfigCommand::Get { key } => match config_get(&key).await? {
                Some(value) => println!("{value}"),
                None => {
                    eprintln!("setting `{key}` is not set");
                    std::process::exit(1);
                }
            },
            ConfigCommand::Set { key, value } => {
                config_set(&key, &value).await?;
                println!("✓ {key} = {value}");
            }
            ConfigCommand::List => {
                let settings = config_list().await?;
                if settings.is_empty() {
                    println!("No settings yet. Run `doonce quickstart`.");
                }
                for (key, value) in settings {
                    if looks_sensitive(&key) {
                        println!("{key} = ••••••");
                    } else {
                        println!("{key} = {value}");
                    }
                }
            }
        },
        Command::Db { command } => match command {
            DbCommand::Migrate => {
                let db = connect_and_migrate().await?;
                println!(
                    "✓ migrations applied ({:?}) — {}",
                    db.dialect(),
                    database_url_redacted()
                );
            }
        },
        Command::Browser { command } => match command {
            BrowserCommand::Check {
                url,
                headless,
                cdp,
                chrome,
                profile,
            } => {
                let mut browser = open_browser(cdp, headless, chrome, profile, None).await?;
                if let Some(url) = url {
                    browser
                        .navigate(&url, Duration::from_secs(30))
                        .await
                        .with_context(|| format!("could not open {url}"))?;
                }
                let info = browser.ping().await?;
                println!("URL   : {}", info["url"].as_str().unwrap_or(""));
                println!("Title : {}", info["title"].as_str().unwrap_or(""));
                println!("Ready : {}", info["readyState"].as_str().unwrap_or(""));
                browser.close().await;
            }
            BrowserCommand::Observe {
                url,
                headless,
                cdp,
                chrome,
                profile,
            } => {
                let mut browser = open_browser(cdp, headless, chrome, profile, None).await?;
                if let Some(url) = url {
                    browser
                        .navigate(&url, Duration::from_secs(30))
                        .await
                        .with_context(|| format!("could not open {url}"))?;
                }
                let page = browser.query("observe", serde_json::json!({})).await?;
                println!("{}", serde_json::to_string_pretty(&page)?);
                browser.close().await;
            }
            BrowserCommand::Stream {
                url,
                port,
                headless,
                cdp,
                chrome,
                profile,
            } => {
                stream_browser(url, port, headless, cdp, chrome, profile).await?;
            }
            BrowserCommand::Screenshot {
                url,
                output,
                headless,
                cdp,
                chrome,
                profile,
            } => {
                let mut browser = open_browser(cdp, headless, chrome, profile, None).await?;
                browser
                    .navigate(&url, Duration::from_secs(30))
                    .await
                    .with_context(|| format!("could not open {url}"))?;
                let bytes = browser.screenshot().await?;
                std::fs::write(&output, bytes)
                    .with_context(|| format!("could not write {}", output.display()))?;
                println!("Saved {}", output.display());
                browser.close().await;
            }
        },
        Command::Replay {
            file,
            params,
            headless,
            cdp,
            chrome,
            profile,
            download_dir,
            timeout,
            artifact_dir,
            allow_private_http,
        } => {
            replay(
                &file,
                &params,
                headless,
                cdp,
                chrome,
                profile,
                download_dir,
                timeout,
                artifact_dir,
                allow_private_http,
            )
            .await?;
        }
        Command::Workflow { command } => match command {
            WorkflowCommand::Validate { file } => {
                let workflow = read_workflow(&file)?;
                println!(
                    "✓ {} is valid: \"{}\" with {} blocks",
                    file.display(),
                    workflow.name,
                    workflow.blocks.len()
                );
            }
            WorkflowCommand::Show { file } => {
                let workflow = read_workflow(&file)?;
                println!("{} (schema {})", workflow.name, workflow.schema_version);
                if !workflow.description.is_empty() {
                    println!("{}", workflow.description);
                }
                for parameter in &workflow.parameters {
                    let default = parameter
                        .default
                        .as_deref()
                        .map(|value| format!(" = {value}"))
                        .unwrap_or_default();
                    println!("  param {} : {:?}{default}", parameter.name, parameter.kind);
                }
                for block in &workflow.blocks {
                    let next = block
                        .next
                        .as_deref()
                        .map(|next| format!(" → {next}"))
                        .unwrap_or_default();
                    println!(
                        "  {:<14} {:<14} {}{next}",
                        block.id,
                        block.type_name(),
                        block.summary()
                    );
                }
            }
        },
        Command::Task { command } => match command {
            TaskCommand::Run {
                prompt,
                url,
                schema,
                error_codes,
                max_steps,
                browser,
                llm,
            } => {
                let schema = read_optional_json(schema.as_ref())?;
                let error_codes = read_optional_string_array(error_codes.as_ref())?;
                let provider = provider_from_args(&llm)?;
                let spec = TaskSpec {
                    prompt,
                    url,
                    data_schema: schema,
                    error_codes,
                    max_steps,
                    model: llm.model.clone(),
                };
                let options = task_options(&browser, &llm, max_steps);
                let cancel = options.cancel.clone();
                tokio::spawn(async move {
                    if tokio::signal::ctrl_c().await.is_ok() {
                        cancel.cancel();
                    }
                });
                let mut session = open_browser(
                    browser.cdp,
                    browser.headless,
                    browser.chrome,
                    browser.profile,
                    browser.download_dir,
                )
                .await?;
                let outcome = doonce_agent::run_task(
                    &provider,
                    &spec,
                    &mut session,
                    &AgentObserver,
                    &options,
                )
                .await;
                session.close().await;
                exit_task_outcome(outcome);
            }
        },
        Command::Page { command } => match command {
            PageCommand::Act {
                instruction,
                url,
                max_steps,
                browser,
                llm,
            } => {
                let provider = provider_from_args(&llm)?;
                let options = task_options(&browser, &llm, max_steps);
                let mut session = open_browser(
                    browser.cdp,
                    browser.headless,
                    browser.chrome,
                    browser.profile,
                    browser.download_dir,
                )
                .await?;
                if let Some(url) = url {
                    session
                        .navigate(&url, Duration::from_secs(30))
                        .await
                        .with_context(|| format!("could not open {url}"))?;
                }
                let outcome = doonce_agent::act(
                    &provider,
                    &instruction,
                    &mut session,
                    &AgentObserver,
                    &options,
                )
                .await;
                session.close().await;
                exit_task_outcome(outcome);
            }
            PageCommand::Extract {
                prompt,
                schema,
                url,
                browser,
                llm,
            } => {
                let provider = provider_from_args(&llm)?;
                let schema = read_json(&schema)?;
                let options = task_options(&browser, &llm, 1);
                let mut session = open_browser(
                    browser.cdp,
                    browser.headless,
                    browser.chrome,
                    browser.profile,
                    browser.download_dir,
                )
                .await?;
                if let Some(url) = url {
                    session
                        .navigate(&url, Duration::from_secs(30))
                        .await
                        .with_context(|| format!("could not open {url}"))?;
                }
                let result =
                    doonce_agent::extract(&provider, &mut session, &prompt, &schema, &options)
                        .await;
                session.close().await;
                match result {
                    Ok(value) => {
                        println!("{}", serde_json::to_string_pretty(&value)?);
                        std::process::exit(0);
                    }
                    Err(error) => {
                        eprintln!("error: {error:#}");
                        std::process::exit(1);
                    }
                }
            }
            PageCommand::Validate {
                prompt,
                url,
                browser,
                llm,
            } => {
                let provider = provider_from_args(&llm)?;
                let options = task_options(&browser, &llm, 1);
                let mut session = open_browser(
                    browser.cdp,
                    browser.headless,
                    browser.chrome,
                    browser.profile,
                    browser.download_dir,
                )
                .await?;
                if let Some(url) = url {
                    session
                        .navigate(&url, Duration::from_secs(30))
                        .await
                        .with_context(|| format!("could not open {url}"))?;
                }
                let result =
                    doonce_agent::validate(&provider, &mut session, &prompt, &options).await;
                session.close().await;
                match result {
                    Ok(valid) => {
                        println!("valid: {valid}");
                        std::process::exit(if valid { 0 } else { 1 });
                    }
                    Err(error) => {
                        eprintln!("error: {error:#}");
                        std::process::exit(1);
                    }
                }
            }
            PageCommand::Prompt {
                prompt,
                schema,
                url,
                browser,
                llm,
            } => {
                let provider = provider_from_args(&llm)?;
                let schema = read_optional_json(schema.as_ref())?;
                let options = task_options(&browser, &llm, 1);
                let mut session = open_browser(
                    browser.cdp,
                    browser.headless,
                    browser.chrome,
                    browser.profile,
                    browser.download_dir,
                )
                .await?;
                if let Some(url) = url {
                    session
                        .navigate(&url, Duration::from_secs(30))
                        .await
                        .with_context(|| format!("could not open {url}"))?;
                }
                let result = doonce_agent::prompt(
                    &provider,
                    &mut session,
                    &prompt,
                    schema.as_ref(),
                    &options,
                )
                .await;
                session.close().await;
                match result {
                    Ok(value) => {
                        if let Some(text) = value.as_str() {
                            println!("{text}");
                        } else {
                            println!("{}", serde_json::to_string_pretty(&value)?);
                        }
                        std::process::exit(0);
                    }
                    Err(error) => {
                        eprintln!("error: {error:#}");
                        std::process::exit(1);
                    }
                }
            }
        },
        Command::Credential { command } => match command {
            CredentialCommand::List => {
                let db = connect_and_migrate().await?;
                let tenant = doonce_cli::default_tenant(&db).await?;
                let credentials = repo::list_credentials(&db, &tenant.id).await?;
                if credentials.is_empty() {
                    println!("No credentials yet.");
                }
                for credential in credentials {
                    println!(
                        "{:<28} {:<20} {:<12} {:<14} {}",
                        credential.id,
                        credential.name,
                        credential.kind,
                        credential.provider,
                        credential.external_ref.as_deref().unwrap_or("-"),
                    );
                }
            }
            CredentialCommand::Add {
                name,
                kind,
                provider,
                external_ref,
                username,
                secret,
                totp_secret,
            } => {
                let kind = CredentialKind::parse(&kind)
                    .ok_or_else(|| anyhow!("unknown credential kind \"{kind}\""))?;
                if !matches!(
                    provider.as_str(),
                    "local" | "bitwarden" | "1password" | "lastpass" | "http"
                ) {
                    bail!("provider must be local, bitwarden, 1password, lastpass or http");
                }
                let external_ref = external_ref
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let secret = match secret {
                    Some(secret) => Some(secret),
                    None if provider == "local" && std::io::stdin().is_terminal() => {
                        Some(rpassword::prompt_password("Secret (hidden): ")?)
                    }
                    None => None,
                };
                if provider == "local" && secret.as_deref().unwrap_or_default().is_empty() {
                    bail!("local credentials require --secret");
                }
                if provider != "local" && external_ref.is_none() {
                    bail!("external credentials require --external-ref");
                }
                let db = connect_and_migrate().await?;
                let tenant = doonce_cli::default_tenant(&db).await?;
                let master = MasterKey::load_or_create(&doonce_cli::data_dir().join("master.key"))?;
                let encryptor = master.encryptor();
                let secret_encrypted = match secret {
                    Some(secret) => encryptor.encrypt_str(&secret)?,
                    None => String::new(),
                };
                let totp_encrypted = match totp_secret {
                    Some(secret) => Some(encryptor.encrypt_str(&secret)?),
                    None => None,
                };
                let credential = repo::create_credential_with_provider(
                    &db,
                    &tenant.id,
                    &name,
                    kind,
                    &provider,
                    external_ref,
                    username.as_deref(),
                    &secret_encrypted,
                    totp_encrypted.as_deref(),
                    &serde_json::json!({}),
                )
                .await?;
                println!(
                    "✓ {} ({}) id={}",
                    credential.name, credential.provider, credential.id
                );
            }
            CredentialCommand::Remove { id, yes } => {
                let db = connect_and_migrate().await?;
                let tenant = doonce_cli::default_tenant(&db).await?;
                if !yes && !confirm(&format!("Delete credential {id}?"))? {
                    return Ok(());
                }
                repo::delete_credential(&db, &tenant.id, &id).await?;
                println!("Deleted {id}");
            }
            CredentialCommand::Test { id } => {
                let db = connect_and_migrate().await?;
                let tenant = doonce_cli::default_tenant(&db).await?;
                let master = MasterKey::load_or_create(&doonce_cli::data_dir().join("master.key"))?;
                let service = CredentialService::new(db, master.encryptor(), tenant.id);
                let resolved = service.resolve(&id).await.map_err(anyhow::Error::msg)?;
                println!("username : {}", resolved.username.as_deref().unwrap_or("-"));
                println!(
                    "secret   : •••••• ({} chars)",
                    resolved.secret.chars().count()
                );
                println!("totp     : {}", resolved.totp_code.is_some());
            }
        },
        Command::Llm { command } => match command {
            LlmCommand::Check { llm } => {
                let config = ProviderConfig::from_env_with(
                    llm.provider.as_deref(),
                    llm.model.as_deref(),
                    llm.base_url.as_deref(),
                )?;
                println!("provider : {}", config.kind.as_str());
                println!("model    : {}", config.model);
                println!(
                    "base_url : {}",
                    config.base_url.as_deref().unwrap_or("(default)")
                );
                println!("vision   : {}", config.vision && !llm.no_vision);
                println!(
                    "api key  : {}",
                    if config.api_key.is_some() {
                        "configured"
                    } else {
                        "not required / absent"
                    }
                );
            }
        },
        Command::Worker {
            concurrency,
            poll_secs,
            lease_secs,
            auto_approve,
        } => {
            let mut config = doonce_worker::WorkerConfig::from_env();
            if let Some(concurrency) = concurrency {
                config.concurrency = concurrency.clamp(1, 32);
            }
            if let Some(poll_secs) = poll_secs {
                config.poll_interval = Duration::from_secs(poll_secs.max(1));
            }
            if let Some(lease_secs) = lease_secs {
                config.lease_secs = lease_secs.max(30);
            }
            if auto_approve {
                config.auto_approve = true;
            }
            doonce_worker::run(config).await?;
        }
        Command::Connector {
            control_url,
            token,
            local_url,
            local_api_key,
            poll_secs,
            lease_secs,
            auto_approve,
        } => {
            let mut config = doonce_cli::connector::ConnectorConfig::from_env()?;
            if let Some(control_url) = control_url {
                config.control_url = doonce_cli::connector::api_base(&control_url);
            }
            if let Some(token) = token {
                config.token = token;
            }
            if let Some(local_url) = local_url {
                config.local_url = doonce_cli::connector::api_base(&local_url);
            }
            if let Some(local_api_key) = local_api_key {
                config.local_api_key = Some(local_api_key);
            }
            if let Some(poll_secs) = poll_secs {
                config.poll_interval = Duration::from_secs(poll_secs.max(1));
            }
            if let Some(lease_secs) = lease_secs {
                config.lease_secs = lease_secs.max(30);
            }
            if auto_approve {
                config.auto_approve = true;
            }
            doonce_cli::connector::serve(config).await?;
        }
        Command::Mcp {
            url,
            api_key,
            read_only,
        } => {
            let base = url
                .or_else(|| std::env::var("DOONCE_API_URL").ok())
                .unwrap_or_else(|| "http://127.0.0.1:8080".into());
            let trimmed = base.trim_end_matches('/');
            let base_url = if trimmed.ends_with("/api/v1") {
                trimmed.to_string()
            } else {
                format!("{trimmed}/api/v1")
            };
            doonce_cli::mcp::serve_stdio(doonce_cli::mcp::McpConfig {
                base_url,
                api_key,
                read_only,
            })
            .await?;
        }
        Command::Serve {
            bind,
            ui_dir,
            dev_no_auth,
        } => {
            let mut config = doonce_server::config::ServerConfig::from_env();
            if let Some(bind) = bind {
                config.bind = bind;
            }
            if let Some(ui_dir) = ui_dir {
                config.ui_dir = ui_dir;
            }
            if dev_no_auth {
                config.dev_no_auth = true;
            }
            doonce_server::serve(config).await?;
        }
        Command::Doctor => {
            let report = doctor().await?;
            println!("DoOnce doctor\n");
            for check in &report.checks {
                let mark = if check.ok { "✓" } else { "✗" };
                println!("  {mark} {:<20} {}", check.name, check.detail);
            }
            if !report.healthy() {
                eprintln!("\nSome checks failed. Fix them before running the service.");
                std::process::exit(1);
            }
            println!("\nAll checks passed.");
        }
        Command::Version => {
            println!("doonce {}", env!("CARGO_PKG_VERSION"));
            println!("license: Apache-2.0");
            println!("schema: workflow v2");
        }
    }
    Ok(())
}

async fn open_browser(
    cdp: Option<String>,
    headless: bool,
    chrome: Option<PathBuf>,
    profile: Option<PathBuf>,
    download_dir: Option<PathBuf>,
) -> Result<BrowserSession> {
    let downloads = download_dir.unwrap_or_else(|| {
        dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("DoOnce")
    });
    match cdp {
        Some(endpoint) => BrowserSession::connect(&endpoint, &downloads)
            .await
            .with_context(|| format!("could not connect to Chrome at {endpoint}")),
        None => {
            let executable = chrome.or_else(find_chrome).ok_or_else(|| {
                anyhow!("Chrome was not found. Install Google Chrome or pass --chrome <path>.")
            })?;
            let user_data_dir =
                profile.unwrap_or_else(|| doonce_cli::data_dir().join("chrome-profile"));
            BrowserSession::launch(
                &LaunchOptions {
                    executable,
                    user_data_dir,
                    headless,
                },
                &downloads,
            )
            .await
            .context("could not launch Chrome")
        }
    }
}

async fn stream_browser(
    url: Option<String>,
    port: u16,
    headless: bool,
    cdp: Option<String>,
    chrome: Option<PathBuf>,
    profile: Option<PathBuf>,
) -> Result<()> {
    let mut session = open_browser(cdp, headless, chrome, profile, None).await?;
    if let Some(url) = url {
        session
            .navigate(&url, Duration::from_secs(30))
            .await
            .with_context(|| format!("could not open {url}"))?;
    }
    let browser = Arc::new(Mutex::new(session));
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("could not bind live view on 127.0.0.1:{port}"))?;
    let address = listener.local_addr()?;
    println!("DoOnce live view: http://{address}/  (Ctrl+C to stop)");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((mut stream, _)) = accepted else { continue };
                let browser = browser.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 8192];
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..n]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");
                    if path.starts_with("/frame") {
                        let result = {
                            let mut session = browser.lock().await;
                            session.screenshot().await
                        };
                        match result {
                            Ok(bytes) => {
                                let header = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                                    bytes.len()
                                );
                                let _ = stream.write_all(header.as_bytes()).await;
                                let _ = stream.write_all(&bytes).await;
                            }
                            Err(error) => {
                                let body = format!("screenshot failed: {error}");
                                let header = format!(
                                    "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    body.len()
                                );
                                let _ = stream.write_all(header.as_bytes()).await;
                                let _ = stream.write_all(body.as_bytes()).await;
                            }
                        }
                    } else {
                        let body = r#"<!doctype html><html><head><meta charset="utf-8"><title>DoOnce live view</title>
<style>body{margin:0;background:#0b0d10;color:#e6edf3;font-family:system-ui}header{padding:.75rem 1rem;border-bottom:1px solid #26303d;font-size:.9rem}img{display:block;max-width:100%;margin:0 auto}</style></head>
<body><header>DoOnce live view · 1 fps</header><img id="live" src="/frame.png" alt="browser live view">
<script>setInterval(() => { document.getElementById('live').src = '/frame.png?t=' + Date.now(); }, 1000);</script></body></html>"#;
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(body.as_bytes()).await;
                    }
                    let _ = stream.shutdown().await;
                });
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    if let Ok(session) = Arc::try_unwrap(browser) {
        session.into_inner().close().await;
    }
    Ok(())
}

fn read_workflow(file: &PathBuf) -> Result<WorkflowDefinition> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    parse_workflow(&text).map_err(|issues| {
        anyhow!(
            "{} is not a valid workflow:\n{}",
            file.display(),
            issues
                .iter()
                .map(|issue| format!("  - {issue}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

fn resolve_params(
    workflow: &WorkflowDefinition,
    raw: &[String],
) -> Result<HashMap<String, String>> {
    let mut provided = HashMap::new();
    for item in raw {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| anyhow!("--param must look like NAME=VALUE, got \"{item}\""))?;
        provided.insert(key.trim().to_string(), value.to_string());
    }
    let known: HashSet<&str> = workflow
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect();
    for key in provided.keys() {
        if !known.contains(key.as_str()) {
            bail!("unknown parameter \"{key}\"");
        }
    }
    for parameter in &workflow.parameters {
        if provided.contains_key(&parameter.name) {
            continue;
        }
        let env_name = format!("DOONCE_PARAM_{}", parameter.name.to_ascii_uppercase());
        if let Ok(value) = std::env::var(&env_name) {
            provided.insert(parameter.name.clone(), value);
            continue;
        }
        if let Some(default) = &parameter.default {
            provided.insert(parameter.name.clone(), default.clone());
            continue;
        }
        if parameter.kind == ParamType::Secret && std::io::stdin().is_terminal() {
            let prompt = parameter.description.as_deref().unwrap_or(&parameter.name);
            let value = rpassword::prompt_password(format!("{prompt} (hidden): "))?;
            provided.insert(parameter.name.clone(), value);
            continue;
        }
        bail!(
            "missing value for parameter \"{}\": pass --param {}=... or set {env_name}",
            parameter.name,
            parameter.name
        );
    }
    Ok(provided)
}

#[allow(clippy::too_many_arguments)]
async fn replay(
    file: &PathBuf,
    raw_params: &[String],
    headless: bool,
    cdp: Option<String>,
    chrome: Option<PathBuf>,
    profile: Option<PathBuf>,
    download_dir: Option<PathBuf>,
    timeout: u64,
    artifact_dir: Option<PathBuf>,
    allow_private_http: bool,
) -> Result<()> {
    let workflow = read_workflow(file)?;
    let params = resolve_params(&workflow, raw_params)?;
    let mut browser = open_browser(cdp, headless, chrome, profile, download_dir).await?;
    let artifact_dir = artifact_dir
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let options = RunOptions {
        element_timeout: Duration::from_secs(timeout),
        artifact_dir,
        allow_private_http,
        ..RunOptions::default()
    };
    let cancel = options.cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nStopping…");
            cancel.cancel();
        }
    });

    println!("▶ {} — {} blocks", workflow.name, workflow.blocks.len());
    let services = build_services(&options).await;
    let observer = CliObserver;
    let outcome = doonce_engine::run_with_services(
        &workflow,
        &params,
        &mut browser,
        &observer,
        &options,
        &services,
    )
    .await;
    browser.close().await;

    match outcome.status {
        RunStatus::Completed => {
            println!("✓ Completed");
            if !outcome.outputs.is_empty() {
                println!(
                    "  outputs: {}",
                    serde_json::to_string_pretty(&outcome.outputs).unwrap_or_default()
                );
            }
            Ok(())
        }
        RunStatus::Cancelled => {
            println!("■ Cancelled");
            std::process::exit(130);
        }
        RunStatus::Failed => {
            let failure = outcome.failure.expect("failed run has a failure");
            println!(
                "✗ Failed at {}: {}",
                outcome.failed_block.as_deref().unwrap_or("unknown"),
                failure.message
            );
            if std::env::var_os("DOONCE_DEBUG").is_some()
                && let Some(diagnostics) = &failure.diagnostics
            {
                println!(
                    "  diagnostics: {}",
                    serde_json::to_string_pretty(diagnostics)
                        .unwrap_or_else(|_| diagnostics.to_string())
                );
            }
            std::process::exit(1);
        }
    }
}

struct CliObserver;

impl Observer for CliObserver {
    fn on_event(&self, event: &ExecEvent) {
        match event {
            ExecEvent::Step {
                index,
                block_id,
                status,
                error,
                duration_ms,
            } => match status {
                StepStatus::Completed => println!("  ✓ [{index}] {block_id}  ({duration_ms} ms)"),
                StepStatus::Failed => println!(
                    "  ✗ [{index}] {block_id}\n      {}",
                    error.as_deref().unwrap_or("")
                ),
                StepStatus::Cancelled => println!(
                    "  ■ [{index}] {block_id}  {}",
                    error.as_deref().unwrap_or("")
                ),
                StepStatus::AwaitingApproval => println!("  ? [{index}] {block_id} needs approval"),
                _ => {}
            },
            ExecEvent::Download { info, .. } => match &info.path {
                Some(path) => println!("      ↓ saved {}", path.display()),
                None => println!("      ↓ downloaded {}", info.filename),
            },
        }
    }

    async fn approve(&self, request: ApprovalRequest) -> bool {
        if !std::io::stdin().is_terminal() {
            eprintln!(
                "  ! \"{}\" needs approval ({}) but no one can answer. Run interactively.",
                request.summary, request.reason
            );
            return false;
        }
        let question = format!(
            "  ? {} — {} Run this block?",
            request.summary, request.reason
        );
        tokio::task::spawn_blocking(move || confirm(&question).unwrap_or(false))
            .await
            .unwrap_or(false)
    }
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn looks_sensitive(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "secret",
        "password",
        "token",
        "api_key",
        "apikey",
        "private_key",
        "credential",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn read_json(path: &PathBuf) -> Result<serde_json::Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} is not valid JSON", path.display()))
}

fn read_optional_json(path: Option<&PathBuf>) -> Result<Option<serde_json::Value>> {
    match path {
        Some(path) => Ok(Some(read_json(path)?)),
        None => Ok(None),
    }
}

fn read_optional_string_array(path: Option<&PathBuf>) -> Result<Vec<String>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let value = read_json(path)?;
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .ok_or_else(|| anyhow!("{} must contain a JSON array of strings", path.display()))
}

fn provider_from_args(args: &LlmArgs) -> Result<Provider> {
    let mut config = ProviderConfig::from_env_with(
        args.provider.as_deref(),
        args.model.as_deref(),
        args.base_url.as_deref(),
    )?;
    if args.no_vision {
        config.vision = false;
    }
    Ok(Provider::new(config)?)
}

fn task_options(browser: &BrowserArgs, llm: &LlmArgs, max_steps: u32) -> TaskOptions {
    TaskOptions {
        element_timeout: Duration::from_secs(browser.timeout),
        navigation_timeout: Duration::from_secs(30),
        max_steps,
        max_duration: Duration::from_secs(600),
        vision: !llm.no_vision,
        ..TaskOptions::default()
    }
}

fn exit_task_outcome(outcome: doonce_agent::TaskOutcome) -> ! {
    match outcome.status {
        TaskStatus::Completed => {
            if let Some(output) = outcome.output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&output).unwrap_or_default()
                );
            } else {
                println!("✓ Completed");
            }
            std::process::exit(0);
        }
        TaskStatus::Cancelled => {
            eprintln!("■ Cancelled");
            std::process::exit(130);
        }
        TaskStatus::Failed => {
            let message = outcome
                .failure
                .map(|failure| failure.message)
                .unwrap_or_else(|| "task failed".into());
            eprintln!("✗ Failed: {message}");
            std::process::exit(1);
        }
    }
}

async fn build_services(options: &RunOptions) -> doonce_engine::Services {
    let upload_root = options.artifact_dir.join("uploads");
    let Some((db, tenant_id)) = default_database_and_tenant().await else {
        return doonce_services::local_services(upload_root);
    };
    let Ok(master) = MasterKey::load_or_create(&doonce_cli::data_dir().join("master.key")) else {
        return doonce_services::local_services(upload_root);
    };
    let task_options = doonce_agent::TaskOptions {
        element_timeout: options.element_timeout,
        navigation_timeout: options.navigation_timeout,
        vision: true,
        ..doonce_agent::TaskOptions::default()
    };
    doonce_services::services_from_env(db, master.encryptor(), tenant_id, task_options, upload_root)
}

async fn default_database_and_tenant() -> Option<(Database, String)> {
    let db = connect_and_migrate().await.ok()?;
    let tenant = doonce_cli::default_tenant(&db).await.ok()?;
    Some((db, tenant.id))
}

struct AgentObserver;

impl TaskObserver for AgentObserver {
    fn on_event(&self, event: &TaskEvent) {
        if let TaskEvent::Step {
            index,
            summary,
            status,
            ..
        } = event
        {
            println!("  [{index}] {status:<9} {summary}");
        }
    }

    async fn approve(&self, request: TaskApprovalRequest) -> bool {
        if !std::io::stdin().is_terminal() {
            eprintln!(
                "  ! \"{}\" needs approval ({}) but no one can answer. Run interactively.",
                request.summary, request.reason
            );
            return false;
        }
        let question = format!("  ? {} — {} Allow?", request.summary, request.reason);
        tokio::task::spawn_blocking(move || confirm(&question).unwrap_or(false))
            .await
            .unwrap_or(false)
    }
}

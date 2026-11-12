//! Versioned workflow definition and block schema (v2).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Only this schema version is accepted by this build.
pub const SCHEMA_VERSION: &str = "2.0";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDefinition {
    pub schema_version: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,
    /// Optional natural-language workflow prompt. When present it runs as an
    /// implicit first browserTask block, so a workflow can be as simple as a
    /// prompt plus a URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    pub blocks: Vec<Block>,
}

impl WorkflowDefinition {
    pub fn to_pretty_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("workflow serializes")
    }

    pub fn block(&self, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub fn block_ids(&self) -> Vec<&str> {
        self.blocks.iter().map(|b| b.id.as_str()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: ParamType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    #[default]
    String,
    Date,
    Number,
    /// Never stored, logged or exported. Must be supplied on every run.
    Secret,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub id: String,
    #[serde(flatten)]
    pub kind: BlockKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Retry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_on_failure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<bool>,
}

impl Block {
    pub fn type_name(&self) -> &'static str {
        self.kind.type_name()
    }

    /// A one-line description safe for logs. Never includes secret values.
    pub fn summary(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        match &self.kind {
            BlockKind::BrowserTask { prompt, url, .. } => match url {
                Some(url) => format!(
                    "Browser task: {} ({})",
                    truncate(prompt, 70),
                    truncate(url, 60)
                ),
                None => format!("Browser task: {}", truncate(prompt, 90)),
            },
            BlockKind::BrowserAction { action } => format!("Browser action: {}", action.summary()),
            BlockKind::Extract { prompt, list, .. } => {
                format!(
                    "Extract {}: {}",
                    if *list { "list" } else { "data" },
                    truncate(prompt, 80)
                )
            }
            BlockKind::Validate { prompt, .. } => format!("Validate: {}", truncate(prompt, 90)),
            BlockKind::ForLoop { over, body, .. } => format!("Loop over {over} → {body}"),
            BlockKind::Conditional { then, .. } => format!("Condition → {then}"),
            BlockKind::HttpRequest { method, url, .. } => format!("{method} {}", truncate(url, 80)),
            BlockKind::Code { language, .. } => format!("Code ({language:?})"),
            BlockKind::FileParse { parser, .. } => format!("Parse file ({parser:?})"),
            BlockKind::SendEmail { subject, .. } => {
                format!("Send email: {}", truncate(subject, 80))
            }
            BlockKind::TextPrompt { prompt, .. } => {
                format!("Text prompt: {}", truncate(prompt, 80))
            }
            BlockKind::Upload { destination, .. } => {
                format!("Upload to {}", truncate(destination, 80))
            }
            BlockKind::DownloadFiles { prompt, .. } => {
                format!("Download files: {}", truncate(prompt, 80))
            }
            BlockKind::Login { credential_id, .. } => {
                format!("Login with credential {credential_id}")
            }
            BlockKind::HumanApproval { message } => format!("Approval: {}", truncate(message, 80)),
            BlockKind::RunWorkflow { workflow_id, .. } => format!("Run workflow {workflow_id}"),
            BlockKind::Wait { .. } => "Wait for condition".to_string(),
            BlockKind::Terminate { status, .. } => format!("Terminate ({status:?})"),
            BlockKind::Goto { target } => format!("Go to {target}"),
        }
    }

    /// All explicit control-flow targets declared by this block.
    pub fn control_targets(&self) -> Vec<&str> {
        let mut out = Vec::new();
        if let Some(next) = &self.next {
            out.push(next.as_str());
        }
        if let Some(err) = &self.on_error {
            out.push(err.as_str());
        }
        match &self.kind {
            BlockKind::ForLoop { body, .. } => out.push(body.as_str()),
            BlockKind::Conditional {
                then, else_branch, ..
            } => {
                out.push(then.as_str());
                if let Some(e) = else_branch {
                    out.push(e.as_str());
                }
            }
            BlockKind::Goto { target } => out.push(target.as_str()),
            _ => {}
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BlockKind {
    BrowserTask {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        navigation_goal: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data_schema: Option<Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        error_codes: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_steps: Option<u32>,
    },
    BrowserAction {
        action: BrowserAction,
    },
    Extract {
        prompt: String,
        schema: Value,
        #[serde(default)]
        list: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pagination: Option<Value>,
    },
    Validate {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
    },
    ForLoop {
        over: String,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_iterations: Option<u32>,
    },
    Conditional {
        condition: Value,
        then: String,
        #[serde(rename = "else", default, skip_serializing_if = "Option::is_none")]
        else_branch: Option<String>,
    },
    HttpRequest {
        method: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        headers: Option<BTreeMap<String, String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_as: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<HttpAuth>,
    },
    Code {
        language: CodeLanguage,
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_as: Option<String>,
    },
    FileParse {
        artifact: String,
        parser: FileParser,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_as: Option<String>,
    },
    SendEmail {
        to: Vec<String>,
        subject: String,
        body: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<String>,
    },
    TextPrompt {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_as: Option<String>,
    },
    Upload {
        artifact: String,
        destination: String,
    },
    DownloadFiles {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_to: Option<String>,
    },
    Login {
        credential_id: String,
        #[serde(default)]
        totp: bool,
    },
    HumanApproval {
        message: String,
    },
    RunWorkflow {
        workflow_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inputs: Option<Value>,
    },
    Wait {
        condition: Condition,
    },
    Terminate {
        status: TerminateStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Goto {
        target: String,
    },
}

impl BlockKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            BlockKind::BrowserTask { .. } => "browserTask",
            BlockKind::BrowserAction { .. } => "browserAction",
            BlockKind::Extract { .. } => "extract",
            BlockKind::Validate { .. } => "validate",
            BlockKind::ForLoop { .. } => "forLoop",
            BlockKind::Conditional { .. } => "conditional",
            BlockKind::HttpRequest { .. } => "httpRequest",
            BlockKind::Code { .. } => "code",
            BlockKind::FileParse { .. } => "fileParse",
            BlockKind::SendEmail { .. } => "sendEmail",
            BlockKind::TextPrompt { .. } => "textPrompt",
            BlockKind::Upload { .. } => "upload",
            BlockKind::DownloadFiles { .. } => "downloadFiles",
            BlockKind::Login { .. } => "login",
            BlockKind::HumanApproval { .. } => "humanApproval",
            BlockKind::RunWorkflow { .. } => "runWorkflow",
            BlockKind::Wait { .. } => "wait",
            BlockKind::Terminate { .. } => "terminate",
            BlockKind::Goto { .. } => "goto",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BrowserAction {
    Navigate {
        url: String,
    },
    Click {
        target: Target,
    },
    Hover {
        target: Target,
    },
    Fill {
        target: Target,
        value: String,
    },
    Select {
        target: Target,
        value: String,
    },
    Check {
        target: Target,
        checked: bool,
    },
    Press {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        key: String,
    },
    Download {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
    },
    Upload {
        target: Target,
        files: Vec<String>,
    },
    SwitchTab {
        url_contains: String,
    },
    Wait {
        condition: Condition,
    },
    Assert {
        condition: Condition,
    },
    Extract {
        target: Target,
        var: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribute: Option<String>,
    },
}

impl BrowserAction {
    pub fn summary(&self) -> String {
        match self {
            BrowserAction::Navigate { url } => format!("open {}", truncate(url, 80)),
            BrowserAction::Click { target } => format!("click {}", target.describe()),
            BrowserAction::Hover { target } => format!("hover {}", target.describe()),
            BrowserAction::Fill { target, .. } => format!("fill {}", target.describe()),
            BrowserAction::Select { target, .. } => format!("select in {}", target.describe()),
            BrowserAction::Check { target, checked } => {
                format!(
                    "{} {}",
                    if *checked { "check" } else { "uncheck" },
                    target.describe()
                )
            }
            BrowserAction::Press { target, key } => match target {
                Some(t) => format!("press {key} in {}", t.describe()),
                None => format!("press {key}"),
            },
            BrowserAction::Download { target } => match target {
                Some(t) => format!("download via {}", t.describe()),
                None => "wait for download".to_string(),
            },
            BrowserAction::Upload { target, files } => {
                format!("upload {} file(s) via {}", files.len(), target.describe())
            }
            BrowserAction::SwitchTab { url_contains } => format!("switch to tab {url_contains}"),
            BrowserAction::Wait { .. } => "wait".to_string(),
            BrowserAction::Assert { .. } => "assert".to_string(),
            BrowserAction::Extract { target, var, .. } => {
                format!("extract {var} from {}", target.describe())
            }
        }
    }

    pub fn target(&self) -> Option<&Target> {
        match self {
            BrowserAction::Click { target }
            | BrowserAction::Hover { target }
            | BrowserAction::Fill { target, .. }
            | BrowserAction::Select { target, .. }
            | BrowserAction::Check { target, .. }
            | BrowserAction::Upload { target, .. }
            | BrowserAction::Extract { target, .. } => Some(target),
            BrowserAction::Press { target, .. } | BrowserAction::Download { target } => {
                target.as_ref()
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HttpAuth {
    Basic { username: String, password: String },
    Bearer { token: String },
    Header { name: String, value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeLanguage {
    Javascript,
    Python,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileParser {
    Csv,
    Xlsx,
    Json,
    Xml,
    Pdf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminateStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selectors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl Target {
    pub fn describe(&self) -> String {
        let what = self
            .name
            .as_deref()
            .or(self.label.as_deref())
            .or(self.text.as_deref())
            .filter(|s| !s.is_empty());
        let kind = self
            .role
            .as_deref()
            .or(self.tag.as_deref())
            .unwrap_or("element");
        match what {
            Some(name) => format!("{kind} \"{}\"", truncate(name, 60)),
            None => match self.selectors.first() {
                Some(sel) => format!("{kind} {}", truncate(sel, 60)),
                None => kind.to_string(),
            },
        }
    }

    pub fn has_identity(&self) -> bool {
        !self.selectors.is_empty()
            || !self.attributes.is_empty()
            || self.name.as_deref().is_some_and(|s| !s.trim().is_empty())
            || self.label.as_deref().is_some_and(|s| !s.trim().is_empty())
            || self.text.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Target>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<Target>,
}

impl Condition {
    pub fn is_empty(&self) -> bool {
        self.url_contains.is_none()
            && self.text_contains.is_none()
            && self.visible.is_none()
            && self.hidden.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Retry {
    /// Total attempts including the first one (1-10).
    pub attempts: u32,
    #[serde(default)]
    pub delay_ms: u64,
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

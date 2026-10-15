use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The only workflow schema version this build understands.
pub const SCHEMA_VERSION: &str = "1.0";

/// A replayable browser automation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Workflow {
    /// Must be "1.0".
    pub schema_version: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,
    pub steps: Vec<Step>,
}

/// A value supplied at run time and referenced as `{{name}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: ParamType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    #[default]
    String,
    /// `YYYY-MM-DD`.
    Date,
    Number,
    /// Never stored, logged or exported. Must be provided on every run.
    Secret,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    #[serde(flatten)]
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Maximum time to wait for the step's element or condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Retry>,
    /// `true` always asks the user before running the step, `false` never
    /// does. When omitted DoOnce decides from the step's risk heuristics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<bool>,
}

/// Field names accepted on every step object, regardless of its type.
pub const COMMON_STEP_FIELDS: &[&str] =
    &["id", "type", "description", "timeoutMs", "retry", "requiresApproval"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    Navigate {
        url: String,
    },
    Click {
        target: Target,
    },
    Fill {
        target: Target,
        value: String,
    },
    /// Selects an `<option>` by value or visible label.
    Select {
        target: Target,
        value: String,
    },
    /// Sets a checkbox or radio button.
    Check {
        target: Target,
        checked: bool,
    },
    /// Presses a key or shortcut such as `Enter` or `Control+K`.
    Press {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        key: String,
    },
    Wait {
        condition: Condition,
    },
    Assert {
        condition: Condition,
    },
    /// Clicks `target` (if given) and waits for the resulting file download.
    Download {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
    },
    /// Continues in another open tab whose URL contains `urlContains`.
    #[serde(rename_all = "camelCase")]
    SwitchTab {
        url_contains: String,
    },
}

impl Action {
    pub fn kind(&self) -> &'static str {
        match self {
            Action::Navigate { .. } => "navigate",
            Action::Click { .. } => "click",
            Action::Fill { .. } => "fill",
            Action::Select { .. } => "select",
            Action::Check { .. } => "check",
            Action::Press { .. } => "press",
            Action::Wait { .. } => "wait",
            Action::Assert { .. } => "assert",
            Action::Download { .. } => "download",
            Action::SwitchTab { .. } => "switchTab",
        }
    }

    /// Fields allowed in addition to [`COMMON_STEP_FIELDS`].
    pub fn fields_for(kind: &str) -> Option<&'static [&'static str]> {
        Some(match kind {
            "navigate" => &["url"],
            "click" => &["target"],
            "fill" | "select" => &["target", "value"],
            "check" => &["target", "checked"],
            "press" => &["target", "key"],
            "wait" | "assert" => &["condition"],
            "download" => &["target"],
            "switchTab" => &["urlContains"],
            _ => return None,
        })
    }

    pub fn target(&self) -> Option<&Target> {
        match self {
            Action::Click { target }
            | Action::Fill { target, .. }
            | Action::Select { target, .. }
            | Action::Check { target, .. } => Some(target),
            Action::Press { target, .. } | Action::Download { target } => target.as_ref(),
            _ => None,
        }
    }

    pub fn target_mut(&mut self) -> Option<&mut Target> {
        match self {
            Action::Click { target }
            | Action::Fill { target, .. }
            | Action::Select { target, .. }
            | Action::Check { target, .. } => Some(target),
            Action::Press { target, .. } | Action::Download { target } => target.as_mut(),
            _ => None,
        }
    }

    /// Whether running the action twice could have a different effect than
    /// running it once (e.g. submitting a form). Such actions are never
    /// re-dispatched automatically.
    pub fn is_idempotent(&self) -> bool {
        matches!(
            self,
            Action::Navigate { .. }
                | Action::Fill { .. }
                | Action::Select { .. }
                | Action::Check { .. }
                | Action::Wait { .. }
                | Action::Assert { .. }
                | Action::SwitchTab { .. }
        )
    }
}

/// Describes an element by several independent signals so it can still be
/// found after the page changes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    /// ARIA role, explicit or implicit (e.g. "button", "textbox").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Accessible name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Visible text, trimmed and truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Text of the associated `<label>` for form fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Stable attributes such as id, name, type, placeholder, data-testid.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
    /// Candidate CSS selectors, most specific first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selectors: Vec<String>,
    /// Nearby heading or landmark text used to tell similar elements apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl Target {
    /// A short human-readable description, safe for logs.
    pub fn describe(&self) -> String {
        let what = self
            .name
            .as_deref()
            .or(self.label.as_deref())
            .or(self.text.as_deref())
            .filter(|s| !s.is_empty());
        let kind = self.role.as_deref().or(self.tag.as_deref()).unwrap_or("element");
        match what {
            Some(name) => format!("{kind} \"{}\"", truncate(name, 60)),
            None => match self.selectors.first() {
                Some(sel) => format!("{kind} {}", truncate(sel, 60)),
                None => kind.to_string(),
            },
        }
    }

    /// Heuristic identity check used when merging recorded events.
    pub fn same_element(&self, other: &Target) -> bool {
        if let (Some(a), Some(b)) = (self.selectors.first(), other.selectors.first()) {
            if a == b {
                return true;
            }
        }
        if let (Some(a), Some(b)) = (self.attributes.get("id"), other.attributes.get("id")) {
            return a == b;
        }
        self.role.is_some()
            && self.name.is_some()
            && self.role == other.role
            && self.name == other.name
            && self.tag == other.tag
            && self.context == other.context
    }
}

/// All provided fields must hold at the same time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Retry {
    /// Total attempts including the first one (1-10).
    pub attempts: u32,
    #[serde(default)]
    pub delay_ms: u64,
}

impl Step {
    /// A one-line description of the step, safe for logs (never includes
    /// substituted parameter values).
    pub fn summary(&self) -> String {
        if let Some(d) = &self.description {
            return d.clone();
        }
        match &self.action {
            Action::Navigate { url } => format!("Open {}", truncate(url, 80)),
            Action::Click { target } => format!("Click {}", target.describe()),
            Action::Fill { target, .. } => format!("Fill {}", target.describe()),
            Action::Select { target, value } => {
                format!("Select \"{}\" in {}", truncate(value, 40), target.describe())
            }
            Action::Check { target, checked } => {
                format!("{} {}", if *checked { "Check" } else { "Uncheck" }, target.describe())
            }
            Action::Press { key, target } => match target {
                Some(t) => format!("Press {key} in {}", t.describe()),
                None => format!("Press {key}"),
            },
            Action::Wait { .. } => "Wait for page".into(),
            Action::Assert { .. } => "Check result".into(),
            Action::Download { target } => match target {
                Some(t) => format!("Download via {}", t.describe()),
                None => "Wait for download".into(),
            },
            Action::SwitchTab { url_contains } => {
                format!("Switch to tab {}", truncate(url_contains, 60))
            }
        }
    }
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

impl Workflow {
    pub fn to_pretty_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("workflow serializes")
    }
}

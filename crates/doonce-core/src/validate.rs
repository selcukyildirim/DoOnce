//! Strict validation for workflows, including ones imported from others.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;

use crate::params::check_type;
use crate::template;
use crate::workflow::*;

pub const MAX_WORKFLOW_BYTES: usize = 2 * 1024 * 1024;
const MAX_STEPS: usize = 1000;
const MAX_STRING: usize = 4000;
const MAX_SELECTORS: usize = 12;
const MAX_TIMEOUT_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    /// JSON-pointer-like location, e.g. `steps[2].target`.
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> Issue {
    Issue { path: path.into(), message: message.into() }
}

/// Parses and fully validates workflow JSON. Unknown fields are rejected so
/// that typos (e.g. `requireApproval`) never silently change behavior.
pub fn parse_workflow(json: &str) -> Result<Workflow, Vec<Issue>> {
    if json.len() > MAX_WORKFLOW_BYTES {
        return Err(vec![issue("", "workflow file is larger than 2 MB")]);
    }
    let value: Value = serde_json::from_str(json).map_err(|e| vec![issue("", format!("invalid JSON: {e}"))])?;
    parse_workflow_value(value)
}

pub fn parse_workflow_value(value: Value) -> Result<Workflow, Vec<Issue>> {
    let mut issues = Vec::new();
    if let Some(steps) = value.get("steps").and_then(Value::as_array) {
        for (i, step) in steps.iter().enumerate() {
            check_step_fields(i, step, &mut issues);
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    let workflow: Workflow = serde_json::from_value(value).map_err(|e| vec![issue("", e.to_string())])?;
    let issues = validate(&workflow);
    if issues.is_empty() { Ok(workflow) } else { Err(issues) }
}

fn check_step_fields(i: usize, step: &Value, issues: &mut Vec<Issue>) {
    let path = format!("steps[{i}]");
    let Some(obj) = step.as_object() else {
        issues.push(issue(path, "step must be an object"));
        return;
    };
    let Some(kind) = obj.get("type").and_then(Value::as_str) else {
        issues.push(issue(path, "missing step \"type\""));
        return;
    };
    let Some(allowed) = Action::fields_for(kind) else {
        issues.push(issue(path, format!("unknown step type \"{kind}\"")));
        return;
    };
    for key in obj.keys() {
        if !COMMON_STEP_FIELDS.contains(&key.as_str()) && !allowed.contains(&key.as_str()) {
            issues.push(issue(&path, format!("unknown field \"{key}\" for {kind} step")));
        }
    }
}

/// Semantic checks on an already-deserialized workflow.
pub fn validate(w: &Workflow) -> Vec<Issue> {
    let mut issues = Vec::new();
    if w.schema_version != SCHEMA_VERSION {
        issues.push(issue(
            "schemaVersion",
            format!("unsupported schema version \"{}\" (expected \"{SCHEMA_VERSION}\")", w.schema_version),
        ));
    }
    if w.name.trim().is_empty() {
        issues.push(issue("name", "name must not be empty"));
    } else if w.name.chars().count() > 120 {
        issues.push(issue("name", "name must be at most 120 characters"));
    }
    check_len("description", &w.description, &mut issues);

    let mut param_names = HashSet::new();
    let mut secrets = HashSet::new();
    for (i, p) in w.parameters.iter().enumerate() {
        let path = format!("parameters[{i}]");
        if !template::is_valid_name(&p.name) {
            issues.push(issue(&path, format!("invalid parameter name \"{}\"", p.name)));
        }
        if !param_names.insert(p.name.as_str()) {
            issues.push(issue(&path, format!("duplicate parameter \"{}\"", p.name)));
        }
        if p.kind == ParamType::Secret {
            secrets.insert(p.name.as_str());
            if p.default.is_some() {
                issues.push(issue(&path, "secret parameters cannot have a default value"));
            }
        } else if let Some(d) = &p.default {
            if let Err(e) = check_type(&p.name, p.kind, d) {
                issues.push(issue(&path, format!("default value: {e}")));
            }
        }
    }

    if w.steps.is_empty() {
        issues.push(issue("steps", "workflow has no steps"));
    }
    if w.steps.len() > MAX_STEPS {
        issues.push(issue("steps", format!("workflow has more than {MAX_STEPS} steps")));
    }

    let mut ids = HashSet::new();
    let ctx = Ctx { params: &param_names, secrets: &secrets };
    for (i, step) in w.steps.iter().enumerate() {
        let path = format!("steps[{i}]");
        if step.id.is_empty()
            || step.id.len() > 64
            || !step.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            issues.push(issue(&path, "step id must be 1-64 characters of letters, digits, '-' or '_'"));
        }
        if !ids.insert(step.id.as_str()) {
            issues.push(issue(&path, format!("duplicate step id \"{}\"", step.id)));
        }
        if let Some(t) = step.timeout_ms {
            if t == 0 || t > MAX_TIMEOUT_MS {
                issues.push(issue(&path, "timeoutMs must be between 1 and 600000"));
            }
        }
        if let Some(r) = &step.retry {
            if !(1..=10).contains(&r.attempts) || r.delay_ms > 60_000 {
                issues.push(issue(&path, "retry.attempts must be 1-10 and retry.delayMs at most 60000"));
            }
        }
        if let Some(d) = &step.description {
            check_len(&path, d, &mut issues);
        }
        check_action(&path, &step.action, &ctx, &mut issues);
    }
    issues
}

struct Ctx<'a> {
    params: &'a HashSet<&'a str>,
    secrets: &'a HashSet<&'a str>,
}

impl Ctx<'_> {
    /// Checks `{{refs}}` in `value`. Secrets are only allowed where
    /// `allow_secret` is true (form values), never in URLs or assertions.
    fn check_template(&self, path: &str, value: &str, allow_secret: bool, issues: &mut Vec<Issue>) {
        match template::references(value) {
            Err(e) => issues.push(issue(path, e.to_string())),
            Ok(refs) => {
                for r in refs {
                    if !self.params.contains(r.as_str()) {
                        issues.push(issue(path, format!("references undefined parameter \"{r}\"")));
                    } else if !allow_secret && self.secrets.contains(r.as_str()) {
                        issues.push(issue(path, format!("secret parameter \"{r}\" can only be used in fill values")));
                    }
                }
            }
        }
    }
}

fn check_action(path: &str, action: &Action, ctx: &Ctx, issues: &mut Vec<Issue>) {
    match action {
        Action::Navigate { url } => {
            check_len(path, url, issues);
            ctx.check_template(&format!("{path}.url"), url, false, issues);
            let literal = url.split("{{").next().unwrap_or_default();
            let ok = if url.contains("{{") {
                literal.starts_with("https://") || literal.starts_with("http://")
            } else {
                is_allowed_url(url)
            };
            if !ok {
                issues.push(issue(format!("{path}.url"), "only http(s) URLs are allowed"));
            }
        }
        Action::Click { target } => check_target(&format!("{path}.target"), target, issues),
        Action::Fill { target, value } | Action::Select { target, value } => {
            check_target(&format!("{path}.target"), target, issues);
            check_len(path, value, issues);
            ctx.check_template(&format!("{path}.value"), value, matches!(action, Action::Fill { .. }), issues);
        }
        Action::Check { target, .. } => check_target(&format!("{path}.target"), target, issues),
        Action::Press { target, key } => {
            if let Some(t) = target {
                check_target(&format!("{path}.target"), t, issues);
            }
            if key.is_empty() || key.len() > 40 {
                issues.push(issue(format!("{path}.key"), "key must be 1-40 characters"));
            }
        }
        Action::Wait { condition } | Action::Assert { condition } => {
            let cpath = format!("{path}.condition");
            if condition.is_empty() {
                issues.push(issue(&cpath, "condition must define at least one check"));
            }
            for s in [&condition.url_contains, &condition.text_contains].into_iter().flatten() {
                check_len(&cpath, s, issues);
                ctx.check_template(&cpath, s, false, issues);
            }
            for t in [&condition.visible, &condition.hidden].into_iter().flatten() {
                check_target(&cpath, t, issues);
            }
        }
        Action::Download { target } => {
            if let Some(t) = target {
                check_target(&format!("{path}.target"), t, issues);
            }
        }
        Action::SwitchTab { url_contains } => {
            if url_contains.trim().is_empty() {
                issues.push(issue(format!("{path}.urlContains"), "must not be empty"));
            }
            check_len(path, url_contains, issues);
            ctx.check_template(&format!("{path}.urlContains"), url_contains, false, issues);
        }
    }
}

fn check_target(path: &str, t: &Target, issues: &mut Vec<Issue>) {
    let has_identity = !t.selectors.is_empty()
        || !t.attributes.is_empty()
        || t.name.as_deref().is_some_and(|s| !s.is_empty())
        || t.label.as_deref().is_some_and(|s| !s.is_empty())
        || t.text.as_deref().is_some_and(|s| !s.is_empty());
    if !has_identity {
        issues.push(issue(path, "target needs at least a selector, attribute, name, label or text"));
    }
    if t.selectors.len() > MAX_SELECTORS {
        issues.push(issue(path, format!("at most {MAX_SELECTORS} selectors are allowed")));
    }
    let strings = [&t.role, &t.name, &t.tag, &t.text, &t.label, &t.context]
        .into_iter()
        .flatten()
        .chain(t.selectors.iter())
        .chain(t.attributes.keys())
        .chain(t.attributes.values());
    for s in strings {
        check_len(path, s, issues);
    }
}

fn check_len(path: &str, s: &str, issues: &mut Vec<Issue>) {
    if s.len() > MAX_STRING {
        issues.push(issue(path, format!("text longer than {MAX_STRING} bytes")));
    }
}

/// Only plain web pages may be opened by a workflow.
pub fn is_allowed_url(s: &str) -> bool {
    url::Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r##"{
        "schemaVersion": "1.0",
        "name": "Daily Report",
        "parameters": [{"name": "reportDate", "type": "date", "default": "2026-10-15"},
                       {"name": "password", "type": "secret"}],
        "steps": [
            {"id": "step-1", "type": "navigate", "url": "https://example.com/reports?d={{reportDate}}"},
            {"id": "step-2", "type": "fill", "target": {"label": "Password"}, "value": "{{password}}"},
            {"id": "step-3", "type": "click", "target": {"role": "button", "name": "Generate Report"},
             "requiresApproval": false},
            {"id": "step-4", "type": "download", "target": {"selectors": ["#export"]}, "timeoutMs": 30000},
            {"id": "step-5", "type": "assert", "condition": {"textContains": "Done"}}
        ]
    }"##;

    #[test]
    fn accepts_valid_workflow() {
        let w = parse_workflow(VALID).unwrap();
        assert_eq!(w.steps.len(), 5);
        assert_eq!(w.steps[2].requires_approval, Some(false));
        // Round trip keeps the document identical.
        let again = parse_workflow(&w.to_pretty_json()).unwrap();
        assert_eq!(w, again);
    }

    fn errors(json: &str) -> String {
        parse_workflow(json).unwrap_err().iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn rejects_unknown_fields_and_types() {
        let typo = VALID.replace("\"requiresApproval\"", "\"requireApproval\"");
        assert!(errors(&typo).contains("unknown field \"requireApproval\""));
        let bad_type = VALID.replace("\"download\"", "\"exfiltrate\"");
        assert!(errors(&bad_type).contains("unknown step type"));
        let extra_root = VALID.replacen("\"name\"", "\"cookies\": [], \"name\"", 1);
        assert!(errors(&extra_root).contains("unknown field `cookies`"));
    }

    #[test]
    fn rejects_dangerous_urls_and_secret_misuse() {
        let js = VALID.replace("https://example.com/reports?d={{reportDate}}", "javascript:alert(1)");
        assert!(errors(&js).contains("only http(s) URLs"));
        let file = VALID.replace("https://example.com/reports?d={{reportDate}}", "file:///etc/passwd");
        assert!(errors(&file).contains("only http(s) URLs"));
        let leak = VALID.replace("?d={{reportDate}}", "?p={{password}}");
        assert!(errors(&leak).contains("secret parameter \"password\" can only be used in fill values"));
        let default_secret = VALID.replace("\"type\": \"secret\"", "\"type\": \"secret\", \"default\": \"x\"");
        assert!(errors(&default_secret).contains("cannot have a default"));
    }

    #[test]
    fn rejects_structural_problems() {
        let dup = VALID.replace("\"step-2\"", "\"step-1\"");
        assert!(errors(&dup).contains("duplicate step id"));
        let undefined = VALID.replace("{{reportDate}}", "{{nope}}");
        assert!(errors(&undefined).contains("undefined parameter \"nope\""));
        let empty_target = VALID.replace(r#"{"label": "Password"}"#, "{}");
        assert!(errors(&empty_target).contains("target needs at least"));
        let version = VALID.replace("\"1.0\"", "\"9.9\"");
        assert!(errors(&version).contains("unsupported schema version"));
        assert!(errors("{not json").contains("invalid JSON"));
        let retry = VALID.replace("\"timeoutMs\": 30000", "\"retry\": {\"attempts\": 99}");
        assert!(errors(&retry).contains("retry.attempts"));
    }
}

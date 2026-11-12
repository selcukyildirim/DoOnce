//! Strict validation for persisted, imported and AI-generated workflows.
//!
//! Every document that enters the database or the engine must pass these
//! checks. The rules are intentionally conservative: unknown fields are
//! rejected by the block deserializer where possible, control-flow targets must
//! exist, secrets may only appear in explicitly allowed sinks, and outbound
//! HTTP blocks are protected against the obvious SSRF cases.

use std::collections::HashSet;
use std::fmt;

use serde::Serialize;
use url::Url;

use crate::template;
use crate::workflow::*;

pub const MAX_WORKFLOW_BYTES: usize = 2 * 1024 * 1024;
const MAX_BLOCKS: usize = 1000;
const MAX_STRING: usize = 4000;
const MAX_CODE: usize = 100_000;
const MAX_SELECTORS: usize = 12;
const MAX_TIMEOUT_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> Issue {
    Issue {
        path: path.into(),
        message: message.into(),
    }
}

pub fn parse_workflow(json: &str) -> Result<WorkflowDefinition, Vec<Issue>> {
    if json.len() > MAX_WORKFLOW_BYTES {
        return Err(vec![issue("", "workflow file is larger than 2 MB")]);
    }
    let workflow: WorkflowDefinition =
        serde_json::from_str(json).map_err(|e| vec![issue("", format!("invalid JSON: {e}"))])?;
    let issues = validate_workflow(&workflow);
    if issues.is_empty() {
        Ok(workflow)
    } else {
        Err(issues)
    }
}

pub fn validate_workflow(w: &WorkflowDefinition) -> Vec<Issue> {
    let mut issues = Vec::new();

    if w.schema_version != SCHEMA_VERSION {
        issues.push(issue(
            "schemaVersion",
            format!(
                "unsupported schema version \"{}\" (expected \"{SCHEMA_VERSION}\")",
                w.schema_version
            ),
        ));
    }
    if w.name.trim().is_empty() {
        issues.push(issue("name", "name must not be empty"));
    } else if w.name.chars().count() > 120 {
        issues.push(issue("name", "name must be at most 120 characters"));
    }
    check_len("description", &w.description, &mut issues);

    let mut params = HashSet::new();
    let mut secrets = HashSet::new();
    for (i, p) in w.parameters.iter().enumerate() {
        let path = format!("parameters[{i}]");
        if !crate::is_valid_name(&p.name) {
            issues.push(issue(
                &path,
                format!("invalid parameter name \"{}\"", p.name),
            ));
        }
        if !params.insert(p.name.clone()) {
            issues.push(issue(&path, format!("duplicate parameter \"{}\"", p.name)));
        }
        match p.kind {
            ParamType::Secret => {
                secrets.insert(p.name.clone());
                if p.default.is_some() {
                    issues.push(issue(
                        &path,
                        "secret parameters cannot have a default value",
                    ));
                }
            }
            ParamType::Date => {
                if let Some(d) = &p.default
                    && !is_iso_date(d)
                {
                    issues.push(issue(&path, "default must be a date in YYYY-MM-DD format"));
                }
            }
            ParamType::Number => {
                if let Some(d) = &p.default
                    && !d.trim().parse::<f64>().is_ok_and(f64::is_finite)
                {
                    issues.push(issue(&path, "default must be a finite number"));
                }
            }
            ParamType::String => {}
        }
    }

    let has_prompt = w
        .prompt
        .as_deref()
        .is_some_and(|prompt| !prompt.trim().is_empty());
    if let Some(prompt) = &w.prompt {
        if prompt.trim().is_empty() {
            issues.push(issue("prompt", "prompt must not be empty"));
        } else {
            check_len("prompt", prompt, &mut issues);
        }
    }
    if let Some(url) = &w.url {
        check_len("url", url, &mut issues);
        if !is_allowed_url_or_template(url) {
            issues.push(issue("url", "only http(s) URLs are allowed"));
        }
    }
    if let Some(schema) = &w.data_schema
        && !schema.is_object()
        && !schema.is_boolean()
    {
        issues.push(issue(
            "dataSchema",
            "data schema must be a JSON object or boolean",
        ));
    }
    if let Some(steps) = w.max_steps
        && !(1..=100).contains(&steps)
    {
        issues.push(issue("maxSteps", "maxSteps must be between 1 and 100"));
    }
    if w.blocks.is_empty() && !has_prompt {
        issues.push(issue("blocks", "workflow has no blocks or prompt"));
    }
    if w.blocks.len() > MAX_BLOCKS {
        issues.push(issue(
            "blocks",
            format!("workflow has more than {MAX_BLOCKS} blocks"),
        ));
    }

    let mut ids = HashSet::new();
    for (i, block) in w.blocks.iter().enumerate() {
        let path = format!("blocks[{i}]");
        if !crate::is_valid_id(&block.id) {
            issues.push(issue(
                &path,
                "block id must be 1-64 characters of letters, digits, '-' or '_'",
            ));
        }
        if !ids.insert(block.id.clone()) {
            issues.push(issue(&path, format!("duplicate block id \"{}\"", block.id)));
        }
        if let Some(t) = block.timeout_ms
            && (t == 0 || t > MAX_TIMEOUT_MS)
        {
            issues.push(issue(&path, "timeoutMs must be between 1 and 600000"));
        }
        if let Some(r) = &block.retry
            && (!(1..=10).contains(&r.attempts) || r.delay_ms > 60_000)
        {
            issues.push(issue(
                &path,
                "retry.attempts must be 1-10 and retry.delayMs at most 60000",
            ));
        }
        if let Some(label) = &block.label {
            check_len(&format!("{path}.label"), label, &mut issues);
        }
        if let Some(description) = &block.description {
            check_len(&format!("{path}.description"), description, &mut issues);
        }
    }

    // Control-flow targets must exist.
    for (i, block) in w.blocks.iter().enumerate() {
        for target in block.control_targets() {
            if !ids.contains(target) {
                issues.push(issue(
                    format!("blocks[{i}]"),
                    format!("control-flow target \"{target}\" does not exist"),
                ));
            }
        }
    }

    let mut vars: HashSet<String> = HashSet::new();
    for block in &w.blocks {
        match &block.kind {
            BlockKind::HttpRequest {
                save_as: Some(name),
                ..
            }
            | BlockKind::Code {
                save_as: Some(name),
                ..
            }
            | BlockKind::FileParse {
                save_as: Some(name),
                ..
            }
            | BlockKind::TextPrompt {
                save_as: Some(name),
                ..
            } => {
                vars.insert(name.clone());
            }
            BlockKind::BrowserAction {
                action: BrowserAction::Extract { var, .. },
            } => {
                vars.insert(var.clone());
            }
            BlockKind::ForLoop {
                item_name: Some(name),
                ..
            } => {
                vars.insert(name.clone());
            }
            _ => {}
        }
    }

    let ctx = Ctx {
        params: &params,
        secrets: &secrets,
        blocks: &ids,
        vars: &vars,
    };
    for (i, block) in w.blocks.iter().enumerate() {
        check_block(&format!("blocks[{i}]"), block, &ctx, &mut issues);
    }

    issues
}

struct Ctx<'a> {
    params: &'a HashSet<String>,
    secrets: &'a HashSet<String>,
    blocks: &'a HashSet<String>,
    vars: &'a HashSet<String>,
}

impl Ctx<'_> {
    fn check_templates(
        &self,
        path: &str,
        value: &str,
        allow_secret: bool,
        issues: &mut Vec<Issue>,
    ) {
        match template::references(value) {
            Err(e) => issues.push(issue(path, e.to_string())),
            Ok(refs) => {
                for root in refs {
                    let is_secret = self.secrets.contains(&root) || root == "secrets";
                    let known = self.params.contains(&root)
                        || self.blocks.contains(&root)
                        || self.vars.contains(&root)
                        || matches!(root.as_str(), "inputs" | "secrets" | "env");
                    if !known {
                        issues.push(issue(
                            path,
                            format!("references undefined parameter, block or variable \"{root}\""),
                        ));
                    } else if is_secret && !allow_secret {
                        issues.push(issue(
                            path,
                            format!("secret \"{root}\" is not allowed in this field"),
                        ));
                    }
                }
            }
        }
    }

    fn check_templates_json(
        &self,
        path: &str,
        value: &serde_json::Value,
        allow_secret: bool,
        issues: &mut Vec<Issue>,
    ) {
        match value {
            serde_json::Value::String(s) => self.check_templates(path, s, allow_secret, issues),
            serde_json::Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    self.check_templates_json(&format!("{path}[{i}]"), v, allow_secret, issues);
                }
            }
            serde_json::Value::Object(o) => {
                for (k, v) in o {
                    self.check_templates_json(&format!("{path}.{k}"), v, allow_secret, issues);
                }
            }
            _ => {}
        }
    }
}

fn check_block(path: &str, block: &Block, ctx: &Ctx, issues: &mut Vec<Issue>) {
    match &block.kind {
        BlockKind::BrowserTask {
            prompt,
            url,
            navigation_goal,
            data_schema,
            error_codes,
            max_steps,
        } => {
            check_len(path, prompt, issues);
            ctx.check_templates(&format!("{path}.prompt"), prompt, false, issues);
            if let Some(u) = url {
                check_len(&format!("{path}.url"), u, issues);
                ctx.check_templates(&format!("{path}.url"), u, false, issues);
                if !is_allowed_url_or_template(u) {
                    issues.push(issue(
                        format!("{path}.url"),
                        "only http(s) URLs are allowed",
                    ));
                }
            }
            if let Some(g) = navigation_goal {
                check_len(&format!("{path}.navigationGoal"), g, issues);
                ctx.check_templates(&format!("{path}.navigationGoal"), g, false, issues);
            }
            if let Some(schema) = data_schema {
                if !schema.is_object() && !schema.is_boolean() {
                    issues.push(issue(
                        format!("{path}.dataSchema"),
                        "data schema must be a JSON object or boolean",
                    ));
                }
                ctx.check_templates_json(&format!("{path}.dataSchema"), schema, false, issues);
            }
            if let Some(steps) = max_steps
                && !(1..=100).contains(steps)
            {
                issues.push(issue(
                    format!("{path}.maxSteps"),
                    "maxSteps must be between 1 and 100",
                ));
            }
            for (i, code) in error_codes.iter().enumerate() {
                check_len(&format!("{path}.errorCodes[{i}]"), code, issues);
            }
        }
        BlockKind::BrowserAction { action } => check_action(path, action, ctx, issues),
        BlockKind::Extract {
            prompt,
            schema,
            list: _,
            pagination,
        } => {
            check_len(path, prompt, issues);
            ctx.check_templates(&format!("{path}.prompt"), prompt, false, issues);
            if !schema.is_object() && !schema.is_boolean() {
                issues.push(issue(
                    format!("{path}.schema"),
                    "schema must be a JSON object or boolean",
                ));
            }
            if let Some(p) = pagination
                && !p.is_object()
            {
                issues.push(issue(
                    format!("{path}.pagination"),
                    "pagination must be an object",
                ));
            }
        }
        BlockKind::Validate { prompt, schema } => {
            check_len(path, prompt, issues);
            ctx.check_templates(&format!("{path}.prompt"), prompt, false, issues);
            if let Some(s) = schema
                && !s.is_object()
                && !s.is_boolean()
            {
                issues.push(issue(
                    format!("{path}.schema"),
                    "schema must be a JSON object or boolean",
                ));
            }
        }
        BlockKind::ForLoop {
            over,
            body,
            item_name,
            max_iterations,
        } => {
            check_len(&format!("{path}.over"), over, issues);
            ctx.check_templates(&format!("{path}.over"), over, false, issues);
            if body.trim().is_empty() {
                issues.push(issue(format!("{path}.body"), "body must reference a block"));
            }
            if let Some(name) = item_name
                && !crate::is_valid_name(name)
            {
                issues.push(issue(format!("{path}.itemName"), "invalid variable name"));
            }
            if let Some(max) = max_iterations
                && !(1..=1000).contains(max)
            {
                issues.push(issue(
                    format!("{path}.maxIterations"),
                    "maxIterations must be between 1 and 1000",
                ));
            }
        }
        BlockKind::Conditional {
            condition,
            then,
            else_branch,
        } => {
            if !condition.is_object() {
                issues.push(issue(
                    format!("{path}.condition"),
                    "condition must be a JSON object",
                ));
            }
            if then.trim().is_empty() {
                issues.push(issue(format!("{path}.then"), "then must reference a block"));
            }
            if let Some(e) = else_branch
                && e.trim().is_empty()
            {
                issues.push(issue(
                    format!("{path}.else"),
                    "else must reference a block when present",
                ));
            }
        }
        BlockKind::HttpRequest {
            method,
            url,
            headers,
            body,
            save_as,
            auth,
        } => {
            let method = method.to_ascii_uppercase();
            if !matches!(
                method.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
            ) {
                issues.push(issue(format!("{path}.method"), "unsupported HTTP method"));
            }
            check_len(&format!("{path}.url"), url, issues);
            ctx.check_templates(&format!("{path}.url"), url, false, issues);
            if !is_safe_outbound_url_or_template(url) {
                issues.push(issue(
                    format!("{path}.url"),
                    "URL must be public http(s); private and link-local hosts are blocked",
                ));
            }
            if let Some(h) = headers {
                for (k, v) in h {
                    check_len(&format!("{path}.headers.{k}"), v, issues);
                    ctx.check_templates(&format!("{path}.headers.{k}"), v, true, issues);
                }
            }
            if let Some(b) = body {
                let size = serde_json::to_string(b).map(|s| s.len()).unwrap_or(0);
                if size > MAX_WORKFLOW_BYTES {
                    issues.push(issue(format!("{path}.body"), "body is too large"));
                }
                ctx.check_templates_json(&format!("{path}.body"), b, true, issues);
            }
            if let Some(a) = auth {
                check_http_auth(path, a, ctx, issues);
            }
            if let Some(v) = save_as
                && !crate::is_valid_name(v)
            {
                issues.push(issue(format!("{path}.saveAs"), "invalid variable name"));
            }
        }
        BlockKind::Code { code, save_as, .. } => {
            if code.len() > MAX_CODE {
                issues.push(issue(format!("{path}.code"), "code is too large"));
            }
            if let Some(v) = save_as
                && !crate::is_valid_name(v)
            {
                issues.push(issue(format!("{path}.saveAs"), "invalid variable name"));
            }
        }
        BlockKind::FileParse {
            artifact, save_as, ..
        } => {
            check_len(&format!("{path}.artifact"), artifact, issues);
            if let Some(v) = save_as
                && !crate::is_valid_name(v)
            {
                issues.push(issue(format!("{path}.saveAs"), "invalid variable name"));
            }
        }
        BlockKind::SendEmail {
            to,
            subject,
            body,
            attachments,
        } => {
            if to.is_empty() {
                issues.push(issue(
                    format!("{path}.to"),
                    "at least one recipient is required",
                ));
            }
            for (i, address) in to.iter().enumerate() {
                if !looks_like_email(address) {
                    issues.push(issue(format!("{path}.to[{i}]"), "invalid email address"));
                }
            }
            check_len(&format!("{path}.subject"), subject, issues);
            check_len(&format!("{path}.body"), body, issues);
            ctx.check_templates(&format!("{path}.subject"), subject, false, issues);
            ctx.check_templates(&format!("{path}.body"), body, true, issues);
            for (i, a) in attachments.iter().enumerate() {
                check_len(&format!("{path}.attachments[{i}]"), a, issues);
            }
        }
        BlockKind::TextPrompt {
            prompt, save_as, ..
        } => {
            check_len(&format!("{path}.prompt"), prompt, issues);
            ctx.check_templates(&format!("{path}.prompt"), prompt, false, issues);
            if let Some(v) = save_as
                && !crate::is_valid_name(v)
            {
                issues.push(issue(format!("{path}.saveAs"), "invalid variable name"));
            }
        }
        BlockKind::Upload {
            artifact,
            destination,
        } => {
            check_len(&format!("{path}.artifact"), artifact, issues);
            check_len(&format!("{path}.destination"), destination, issues);
        }
        BlockKind::DownloadFiles { prompt, save_to } => {
            check_len(&format!("{path}.prompt"), prompt, issues);
            ctx.check_templates(&format!("{path}.prompt"), prompt, false, issues);
            if let Some(s) = save_to {
                check_len(&format!("{path}.saveTo"), s, issues);
            }
        }
        BlockKind::Login { credential_id, .. } => {
            if credential_id.trim().is_empty() {
                issues.push(issue(
                    format!("{path}.credentialId"),
                    "credentialId must not be empty",
                ));
            }
        }
        BlockKind::HumanApproval { message } => {
            check_len(&format!("{path}.message"), message, issues);
        }
        BlockKind::RunWorkflow {
            workflow_id,
            inputs,
        } => {
            if workflow_id.trim().is_empty() {
                issues.push(issue(
                    format!("{path}.workflowId"),
                    "workflowId must not be empty",
                ));
            }
            if let Some(i) = inputs
                && !i.is_object()
            {
                issues.push(issue(format!("{path}.inputs"), "inputs must be an object"));
            }
        }
        BlockKind::Wait { condition } => check_condition(path, condition, ctx, issues),
        BlockKind::Terminate { reason, .. } => {
            if let Some(r) = reason {
                check_len(&format!("{path}.reason"), r, issues);
            }
        }
        BlockKind::Goto { target } => {
            if target.trim().is_empty() {
                issues.push(issue(format!("{path}.target"), "target must not be empty"));
            }
        }
    }
}

fn check_http_auth(path: &str, auth: &HttpAuth, ctx: &Ctx, issues: &mut Vec<Issue>) {
    match auth {
        HttpAuth::Basic { username, password } => {
            check_len(&format!("{path}.auth.username"), username, issues);
            check_len(&format!("{path}.auth.password"), password, issues);
            ctx.check_templates(&format!("{path}.auth.username"), username, true, issues);
            ctx.check_templates(&format!("{path}.auth.password"), password, true, issues);
        }
        HttpAuth::Bearer { token } => {
            check_len(&format!("{path}.auth.token"), token, issues);
            ctx.check_templates(&format!("{path}.auth.token"), token, true, issues);
        }
        HttpAuth::Header { name, value } => {
            check_len(&format!("{path}.auth.header.name"), name, issues);
            check_len(&format!("{path}.auth.header.value"), value, issues);
            ctx.check_templates(&format!("{path}.auth.header.value"), value, true, issues);
        }
    }
}

fn check_action(path: &str, action: &BrowserAction, ctx: &Ctx, issues: &mut Vec<Issue>) {
    match action {
        BrowserAction::Navigate { url } => {
            check_len(&format!("{path}.url"), url, issues);
            ctx.check_templates(&format!("{path}.url"), url, false, issues);
            if !is_allowed_url_or_template(url) {
                issues.push(issue(
                    format!("{path}.url"),
                    "only http(s) URLs are allowed",
                ));
            }
        }
        BrowserAction::Click { target } | BrowserAction::Hover { target } => {
            check_target(&format!("{path}.target"), target, issues)
        }
        BrowserAction::Fill { target, value } => {
            check_target(&format!("{path}.target"), target, issues);
            check_len(&format!("{path}.value"), value, issues);
            ctx.check_templates(&format!("{path}.value"), value, true, issues);
        }
        BrowserAction::Select { target, value } => {
            check_target(&format!("{path}.target"), target, issues);
            check_len(&format!("{path}.value"), value, issues);
            ctx.check_templates(&format!("{path}.value"), value, false, issues);
        }
        BrowserAction::Check { target, .. } => {
            check_target(&format!("{path}.target"), target, issues)
        }
        BrowserAction::Press { target, key } => {
            if let Some(t) = target {
                check_target(&format!("{path}.target"), t, issues);
            }
            if key.is_empty() || key.len() > 40 {
                issues.push(issue(format!("{path}.key"), "key must be 1-40 characters"));
            }
        }
        BrowserAction::Download { target } => {
            if let Some(t) = target {
                check_target(&format!("{path}.target"), t, issues);
            }
        }
        BrowserAction::Upload { target, files } => {
            check_target(&format!("{path}.target"), target, issues);
            if files.is_empty() {
                issues.push(issue(
                    format!("{path}.files"),
                    "at least one file is required",
                ));
            }
            for (i, f) in files.iter().enumerate() {
                check_len(&format!("{path}.files[{i}]"), f, issues);
                if f.contains("..") {
                    issues.push(issue(
                        format!("{path}.files[{i}]"),
                        "path traversal is not allowed",
                    ));
                }
            }
        }
        BrowserAction::SwitchTab { url_contains } => {
            if url_contains.trim().is_empty() {
                issues.push(issue(format!("{path}.urlContains"), "must not be empty"));
            }
            check_len(&format!("{path}.urlContains"), url_contains, issues);
            ctx.check_templates(&format!("{path}.urlContains"), url_contains, false, issues);
        }
        BrowserAction::Wait { condition } | BrowserAction::Assert { condition } => {
            check_condition(path, condition, ctx, issues)
        }
        BrowserAction::Extract {
            target,
            var,
            attribute,
        } => {
            check_target(&format!("{path}.target"), target, issues);
            if !crate::is_valid_name(var) {
                issues.push(issue(
                    format!("{path}.var"),
                    format!("invalid variable name \"{var}\""),
                ));
            }
            if let Some(a) = attribute {
                check_len(&format!("{path}.attribute"), a, issues);
            }
        }
    }
}

fn check_condition(path: &str, condition: &Condition, ctx: &Ctx, issues: &mut Vec<Issue>) {
    if condition.is_empty() {
        issues.push(issue(path, "condition must define at least one check"));
    }
    for (name, value) in [
        ("urlContains", &condition.url_contains),
        ("textContains", &condition.text_contains),
    ] {
        if let Some(s) = value {
            check_len(&format!("{path}.{name}"), s, issues);
            ctx.check_templates(&format!("{path}.{name}"), s, false, issues);
        }
    }
    if let Some(t) = &condition.visible {
        check_target(&format!("{path}.visible"), t, issues);
    }
    if let Some(t) = &condition.hidden {
        check_target(&format!("{path}.hidden"), t, issues);
    }
}

fn check_target(path: &str, target: &Target, issues: &mut Vec<Issue>) {
    if !target.has_identity() {
        issues.push(issue(
            path,
            "target needs at least a selector, attribute, name, label or text",
        ));
    }
    if target.selectors.len() > MAX_SELECTORS {
        issues.push(issue(
            path,
            format!("at most {MAX_SELECTORS} selectors are allowed"),
        ));
    }
    for (i, s) in target.selectors.iter().enumerate() {
        if s.trim().is_empty() {
            issues.push(issue(
                format!("{path}.selectors[{i}]"),
                "selector must not be empty",
            ));
        }
        check_len(&format!("{path}.selectors[{i}]"), s, issues);
    }
    for (k, v) in &target.attributes {
        check_len(&format!("{path}.attributes.{k}"), v, issues);
    }
    for s in [
        &target.role,
        &target.name,
        &target.tag,
        &target.text,
        &target.label,
        &target.context,
    ]
    .into_iter()
    .flatten()
    {
        check_len(path, s, issues);
    }
}

fn check_len(path: &str, s: &str, issues: &mut Vec<Issue>) {
    if s.len() > MAX_STRING {
        issues.push(issue(path, format!("text longer than {MAX_STRING} bytes")));
    }
}

fn looks_like_email(s: &str) -> bool {
    let mut parts = s.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !s.contains(' ')
}

fn is_iso_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (
        parts[0].parse::<u32>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
    ) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

/// Browser navigation may contain templates (for example `{{base}}/path`).
/// The literal prefix must still be http(s).
pub fn is_allowed_url_or_template(s: &str) -> bool {
    if !s.contains("{{") {
        return is_allowed_url(s);
    }
    let literal = s.split("{{").next().unwrap_or_default();
    // A template at the very start (for example `{{base}}`) is validated again
    // after rendering by the engine.
    literal.is_empty() || literal.starts_with("https://") || literal.starts_with("http://")
}

/// Browser navigation allows localhost so the bundled demo site and local apps
/// can be automated.
pub fn is_allowed_url(s: &str) -> bool {
    Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
}

/// Outbound HTTP URLs may contain templates when the literal prefix is
/// http(s). The engine re-checks the fully rendered URL at runtime.
pub fn is_safe_outbound_url_or_template(s: &str) -> bool {
    if !s.contains("{{") {
        return is_safe_outbound_url(s);
    }
    let literal = s.split("{{").next().unwrap_or_default();
    literal.is_empty() || literal.starts_with("https://") || literal.starts_with("http://")
}

/// Outbound HTTP blocks must not reach private, loopback, link-local or
/// cloud-metadata endpoints. Runtime code must additionally resolve DNS and
/// re-check the resolved addresses to mitigate rebinding.
pub fn is_safe_outbound_url(s: &str) -> bool {
    let Ok(u) = Url::parse(s) else { return false };
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = u.host_str() else {
        return false;
    };
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if matches!(host.as_str(), "localhost" | "0.0.0.0" | "::1" | "::") {
        return false;
    }
    if host.ends_with(".local") || host.ends_with(".internal") || host.ends_with(".localhost") {
        return false;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let blocked = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_multicast()
                    || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
            }
            std::net::IpAddr::V6(v6) => {
                let first = v6.segments()[0];
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || (first & 0xfe00) == 0xfc00
                    || (first & 0xffc0) == 0xfe80
            }
        };
        return !blocked;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
        "schemaVersion": "2.0",
        "name": "Daily Report",
        "parameters": [{"name":"reportDate","type":"date","default":"2026-10-15"},
                       {"name":"password","type":"secret"}],
        "blocks": [
            {"id":"open","type":"browserAction","action":{"kind":"navigate","url":"https://example.com/reports?d={{reportDate}}"},"next":"login"},
            {"id":"login","type":"browserAction","action":{"kind":"fill","target":{"label":"Password"},"value":"{{password}}"},"next":"generate"},
            {"id":"generate","type":"browserAction","action":{"kind":"click","target":{"role":"button","name":"Generate Report"}},"next":"done"},
            {"id":"done","type":"terminate","status":"completed"}
        ]
    }"#;

    #[test]
    fn accepts_valid_workflow() {
        let w = parse_workflow(VALID).unwrap();
        assert_eq!(w.blocks.len(), 4);
        assert_eq!(w.block("generate").unwrap().type_name(), "browserAction");
        let again = parse_workflow(&w.to_pretty_json()).unwrap();
        assert_eq!(w, again);
    }

    #[test]
    fn rejects_dangling_control_flow_and_unknown_refs() {
        let dangling = VALID.replace("\"next\":\"done\"", "\"next\":\"missing\"");
        assert!(
            parse_workflow(&dangling)
                .unwrap_err()
                .iter()
                .any(|i| i.message.contains("does not exist"))
        );
        let unknown = VALID.replace("{{reportDate}}", "{{nope}}");
        assert!(
            parse_workflow(&unknown)
                .unwrap_err()
                .iter()
                .any(|i| i.message.contains("undefined"))
        );
    }

    #[test]
    fn rejects_secret_leaks_and_unsafe_urls() {
        let leak = VALID.replace("?d={{reportDate}}", "?p={{password}}");
        assert!(
            parse_workflow(&leak)
                .unwrap_err()
                .iter()
                .any(|i| i.message.contains("not allowed"))
        );
        let js = VALID.replace(
            "https://example.com/reports?d={{reportDate}}",
            "javascript:alert(1)",
        );
        assert!(
            parse_workflow(&js)
                .unwrap_err()
                .iter()
                .any(|i| i.message.contains("only http(s)"))
        );
        let ssrf = r#"{"schemaVersion":"2.0","name":"x","blocks":[{"id":"a","type":"httpRequest","method":"GET","url":"http://127.0.0.1:8080/admin"}]}"#;
        assert!(
            parse_workflow(ssrf)
                .unwrap_err()
                .iter()
                .any(|i| i.message.contains("private"))
        );
    }

    #[test]
    fn rejects_structure_problems() {
        let dup = VALID.replace("\"id\":\"login\"", "\"id\":\"open\"");
        assert!(
            parse_workflow(&dup)
                .unwrap_err()
                .iter()
                .any(|i| i.message.contains("duplicate block id"))
        );
        let bad_version = VALID.replace("\"2.0\"", "\"1.0\"");
        assert!(
            parse_workflow(&bad_version)
                .unwrap_err()
                .iter()
                .any(|i| i.message.contains("unsupported"))
        );
        let target = VALID.replace(r#"{"role":"button","name":"Generate Report"}"#, "{}");
        assert!(
            parse_workflow(&target)
                .unwrap_err()
                .iter()
                .any(|i| i.message.contains("target needs"))
        );
        assert!(
            parse_workflow("{not json").unwrap_err()[0]
                .message
                .contains("invalid JSON")
        );
    }

    #[test]
    fn outbound_url_guard() {
        assert!(is_safe_outbound_url("https://example.com/x"));
        assert!(!is_safe_outbound_url("http://localhost/x"));
        assert!(!is_safe_outbound_url("http://10.0.0.1/x"));
        assert!(!is_safe_outbound_url(
            "http://169.254.169.254/latest/meta-data"
        ));
        assert!(!is_safe_outbound_url("file:///etc/passwd"));
    }
}

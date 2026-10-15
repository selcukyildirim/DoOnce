//! Turns the raw event stream captured by the browser extension into a
//! clean, replayable workflow.

use serde::{Deserialize, Serialize};

use crate::sanitize::{sanitize_url, sanitize_workflow};
use crate::workflow::*;

/// One event captured while recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedEvent {
    /// Milliseconds since the Unix epoch.
    pub ts: f64,
    #[serde(default)]
    pub tab_id: Option<i64>,
    /// Page URL where the event happened.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EventKind {
    /// Recording started on a page.
    Start {
        #[serde(default)]
        title: Option<String>,
    },
    /// A top-level navigation. `transition` is Chrome's transition type.
    Navigate {
        #[serde(default)]
        transition: Option<String>,
    },
    Click {
        target: Target,
    },
    /// Final value of a text field. `value` is `None` for masked fields.
    Fill {
        target: Target,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        sensitive: bool,
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
        #[serde(default)]
        target: Option<Target>,
        key: String,
    },
    Download {
        #[serde(default)]
        filename: Option<String>,
    },
}

/// Navigation transitions the user initiated directly (rather than as a
/// consequence of clicking something).
const USER_TRANSITIONS: &[&str] = &["typed", "auto_bookmark", "generated", "keyword", "start_page", "reload"];

/// Clicks this close to a download are treated as what triggered it.
const DOWNLOAD_TRIGGER_WINDOW_MS: f64 = 5_000.0;

pub struct Compiled {
    pub workflow: Workflow,
    pub warnings: Vec<String>,
}

struct Draft {
    step: Action,
    ts: f64,
}

pub fn compile(name: Option<&str>, events: &[RecordedEvent]) -> Compiled {
    let mut drafts: Vec<Draft> = Vec::new();
    let mut parameters: Vec<Parameter> = Vec::new();
    let mut warnings = Vec::new();
    let mut title = None;
    let mut current_tab: Option<i64> = None;

    for (i, ev) in events.iter().enumerate() {
        // Tab changes: a new tab whose first event is a typed navigation is
        // just "go there"; otherwise it was opened by the page (e.g. a
        // target=_blank link) and replay has to follow it.
        if let (Some(tab), Some(prev)) = (ev.tab_id, current_tab) {
            if tab != prev && !matches!(ev.kind, EventKind::Navigate { .. } | EventKind::Start { .. }) {
                if let Some(url) = ev.url.as_deref().and_then(tab_hint) {
                    drafts.push(Draft { step: Action::SwitchTab { url_contains: url }, ts: ev.ts });
                }
            }
        }
        if ev.tab_id.is_some() {
            current_tab = ev.tab_id;
        }

        match &ev.kind {
            EventKind::Start { title: t } => {
                title = t.clone().filter(|t| !t.trim().is_empty());
                if let Some(url) = ev.url.as_deref().filter(|u| is_web_url(u)) {
                    push_navigate(&mut drafts, url, ev.ts);
                }
            }
            EventKind::Navigate { transition } => {
                let user_initiated = transition.as_deref().is_none_or(|t| USER_TRANSITIONS.contains(&t));
                if let Some(url) = ev.url.as_deref().filter(|u| is_web_url(u)) {
                    if user_initiated || drafts.is_empty() {
                        push_navigate(&mut drafts, url, ev.ts);
                    }
                }
            }
            EventKind::Click { target } => {
                // Ignore duplicate clicks from double-clicking.
                if let Some(last) = drafts.last() {
                    if let Action::Click { target: t } = &last.step {
                        if t.same_element(target) && ev.ts - last.ts < 500.0 {
                            continue;
                        }
                    }
                }
                drafts.push(Draft { step: Action::Click { target: target.clone() }, ts: ev.ts });
            }
            EventKind::Fill { target, value, sensitive } => {
                let secret = *sensitive || value.is_none();
                // A later fill of the same field replaces the earlier one and
                // the click that focused the field is redundant.
                let mut reuse_placeholder = None;
                if let Some(last) = drafts.last() {
                    match &last.step {
                        Action::Fill { target: t, value: old } if t.same_element(target) => {
                            if is_placeholder(old) {
                                reuse_placeholder = Some(old.clone());
                            }
                            drafts.pop();
                        }
                        Action::Click { target: t } if t.same_element(target) => {
                            drafts.pop();
                        }
                        _ => {}
                    }
                }
                let value = match (secret, reuse_placeholder) {
                    // Once a field was seen as sensitive it stays masked.
                    (_, Some(placeholder)) => placeholder,
                    (true, None) => {
                        let name = secret_param_name(target, &parameters);
                        parameters.push(Parameter {
                            name: name.clone(),
                            kind: ParamType::Secret,
                            default: None,
                            description: Some(format!("Value for {}", target.describe())),
                        });
                        format!("{{{{{name}}}}}")
                    }
                    (false, None) => escape_braces(value.as_deref().unwrap_or_default()),
                };
                drafts.push(Draft { step: Action::Fill { target: target.clone(), value }, ts: ev.ts });
            }
            EventKind::Select { target, value } => {
                pop_if_same(&mut drafts, target, |a| matches!(a, Action::Click { .. } | Action::Select { .. }));
                drafts.push(Draft {
                    step: Action::Select { target: target.clone(), value: escape_braces(value) },
                    ts: ev.ts,
                });
            }
            EventKind::Check { target, checked } => {
                pop_if_same(&mut drafts, target, |a| matches!(a, Action::Click { .. } | Action::Check { .. }));
                drafts.push(Draft { step: Action::Check { target: target.clone(), checked: *checked }, ts: ev.ts });
            }
            EventKind::Press { target, key } => {
                drafts.push(Draft { step: Action::Press { target: target.clone(), key: key.clone() }, ts: ev.ts });
            }
            EventKind::Download { .. } => {
                let trigger = drafts.iter().rposition(|d| {
                    matches!(d.step, Action::Click { .. }) && ev.ts - d.ts <= DOWNLOAD_TRIGGER_WINDOW_MS
                });
                match trigger {
                    Some(idx) if idx + 1 >= drafts.len().saturating_sub(1) => {
                        if let Action::Click { target } = &drafts[idx].step {
                            drafts[idx].step = Action::Download { target: Some(target.clone()) };
                        }
                    }
                    _ => {
                        warnings.push(format!(
                            "event {i}: download was not triggered by a recorded click; replay will only wait for it"
                        ));
                        drafts.push(Draft { step: Action::Download { target: None }, ts: ev.ts });
                    }
                }
            }
        }
    }

    let steps = drafts
        .into_iter()
        .enumerate()
        .map(|(i, d)| Step {
            id: format!("step-{}", i + 1),
            action: d.step,
            description: None,
            timeout_ms: None,
            retry: None,
            requires_approval: None,
        })
        .collect();

    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or(title)
        .unwrap_or_else(|| "Recorded automation".into());

    let mut workflow = Workflow {
        schema_version: SCHEMA_VERSION.into(),
        name: crate::workflow::truncate(&name, 120),
        description: String::new(),
        parameters,
        steps,
    };
    warnings.extend(sanitize_workflow(&mut workflow));
    Compiled { workflow, warnings }
}

fn push_navigate(drafts: &mut Vec<Draft>, url: &str, ts: f64) {
    let (url, _) = sanitize_url(url);
    let url = escape_braces(&url);
    if let Some(Draft { step: Action::Navigate { url: last }, .. }) = drafts.last() {
        if *last == url {
            return;
        }
    }
    // Two navigations in a row: only the final destination matters.
    if matches!(drafts.last(), Some(Draft { step: Action::Navigate { .. }, .. })) {
        drafts.pop();
    }
    drafts.push(Draft { step: Action::Navigate { url }, ts });
}

fn pop_if_same(drafts: &mut Vec<Draft>, target: &Target, kind: impl Fn(&Action) -> bool) {
    if let Some(last) = drafts.last() {
        if kind(&last.step) && last.step.target().is_some_and(|t| t.same_element(target)) {
            drafts.pop();
        }
    }
}

fn is_web_url(u: &str) -> bool {
    u.starts_with("http://") || u.starts_with("https://")
}

/// Host and path of a URL, without query or fragment.
fn tab_hint(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?;
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{host}{port}{}", u.path()))
}

fn is_placeholder(value: &str) -> bool {
    value.starts_with("{{") && value.ends_with("}}")
}

/// Recorded literal text must not be mistaken for a parameter reference.
fn escape_braces(s: &str) -> String {
    s.replace("{{", "{ {")
}

fn secret_param_name(target: &Target, existing: &[Parameter]) -> String {
    let source = target
        .attributes
        .get("autocomplete")
        .filter(|a| a.contains("password"))
        .map(|_| "password".to_string())
        .or_else(|| target.label.clone())
        .or_else(|| target.attributes.get("name").cloned())
        .or_else(|| target.attributes.get("id").cloned())
        .or_else(|| target.name.clone())
        .unwrap_or_else(|| "secret".into());
    let mut base = camel_case(&source);
    if base.is_empty() || !base.chars().next().unwrap().is_ascii_alphabetic() {
        base = "secret".into();
    }
    base.truncate(48);
    let mut name = base.clone();
    let mut n = 2;
    while existing.iter().any(|p| p.name == name) {
        name = format!("{base}{n}");
        n += 1;
    }
    name
}

fn camel_case(s: &str) -> String {
    let mut out = String::new();
    for (i, word) in s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).enumerate() {
        let lower = word.to_ascii_lowercase();
        if i == 0 {
            out.push_str(&lower);
        } else {
            let mut chars = lower.chars();
            if let Some(f) = chars.next() {
                out.push(f.to_ascii_uppercase());
                out.extend(chars);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate;

    fn target(id: &str, role: &str, name: &str) -> Target {
        Target {
            role: Some(role.into()),
            name: Some(name.into()),
            selectors: vec![format!("#{id}")],
            attributes: [("id".to_string(), id.to_string())].into(),
            ..Default::default()
        }
    }

    fn ev(ts: f64, kind: EventKind) -> RecordedEvent {
        RecordedEvent { ts, tab_id: Some(1), url: Some("http://localhost:4317/contact".into()), kind }
    }

    #[test]
    fn compiles_a_form_session() {
        let name = target("name", "textbox", "Full name");
        let pw = Target { label: Some("Password".into()), ..target("pw", "textbox", "Password") };
        let events = vec![
            ev(0.0, EventKind::Start { title: Some("Contact".into()) }),
            ev(10.0, EventKind::Click { target: name.clone() }),
            ev(20.0, EventKind::Fill { target: name.clone(), value: Some("Ada".into()), sensitive: false }),
            ev(30.0, EventKind::Fill { target: name.clone(), value: Some("Ada Lovelace".into()), sensitive: false }),
            ev(40.0, EventKind::Click { target: pw.clone() }),
            ev(50.0, EventKind::Fill { target: pw.clone(), value: None, sensitive: true }),
            ev(55.0, EventKind::Fill { target: pw.clone(), value: None, sensitive: true }),
            ev(60.0, EventKind::Select { target: target("country", "combobox", "Country"), value: "tr".into() }),
            ev(70.0, EventKind::Click { target: target("terms", "checkbox", "Terms") }),
            ev(71.0, EventKind::Check { target: target("terms", "checkbox", "Terms"), checked: true }),
            ev(80.0, EventKind::Click { target: target("save", "button", "Save") }),
            ev(85.0, EventKind::Click { target: target("save", "button", "Save") }),
            ev(90.0, EventKind::Navigate { transition: Some("form_submit".into()) }),
        ];
        let Compiled { workflow, .. } = compile(None, &events);
        assert_eq!(workflow.name, "Contact");
        let kinds: Vec<&str> = workflow.steps.iter().map(|s| s.action.kind()).collect();
        assert_eq!(kinds, ["navigate", "fill", "fill", "select", "check", "click"]);
        assert_eq!(workflow.steps[1].action, Action::Fill { target: name, value: "Ada Lovelace".into() });
        assert_eq!(workflow.parameters.len(), 1);
        assert_eq!(workflow.parameters[0].name, "password");
        assert_eq!(workflow.parameters[0].kind, ParamType::Secret);
        assert!(matches!(&workflow.steps[2].action, Action::Fill { value, .. } if value == "{{password}}"));
        assert!(validate(&workflow).is_empty(), "{:?}", validate(&workflow));
    }

    #[test]
    fn download_click_becomes_download_step() {
        let events = vec![
            ev(0.0, EventKind::Start { title: None }),
            ev(10.0, EventKind::Click { target: target("export", "button", "Export CSV") }),
            ev(900.0, EventKind::Download { filename: Some("report.csv".into()) }),
        ];
        let w = compile(Some("Report"), &events).workflow;
        assert_eq!(w.steps.len(), 2);
        assert!(matches!(&w.steps[1].action, Action::Download { target: Some(t) } if t.name.as_deref() == Some("Export CSV")));
    }

    #[test]
    fn typed_navigation_is_kept_and_tokens_are_stripped() {
        let mut nav = ev(5.0, EventKind::Navigate { transition: Some("typed".into()) });
        nav.url = Some("https://example.com/app?session=abc&tab=2".into());
        let events = vec![ev(0.0, EventKind::Start { title: None }), nav];
        let w = compile(None, &events).workflow;
        assert_eq!(w.steps.len(), 1, "consecutive navigations collapse");
        assert_eq!(w.steps[0].action, Action::Navigate { url: "https://example.com/app?tab=2".into() });
    }

    #[test]
    fn new_tab_from_click_emits_switch_tab() {
        let mut in_new_tab = ev(20.0, EventKind::Click { target: target("x", "button", "Go") });
        in_new_tab.tab_id = Some(2);
        in_new_tab.url = Some("http://localhost:4317/details?id=5".into());
        let events = vec![
            ev(0.0, EventKind::Start { title: None }),
            ev(10.0, EventKind::Click { target: target("open", "link", "Open details") }),
            in_new_tab,
        ];
        let w = compile(None, &events).workflow;
        assert_eq!(w.steps[2].action, Action::SwitchTab { url_contains: "localhost:4317/details".into() });
    }

    #[test]
    fn literal_braces_are_not_parameters() {
        let t = target("q", "textbox", "Query");
        let events = vec![
            ev(0.0, EventKind::Start { title: None }),
            ev(10.0, EventKind::Fill { target: t, value: Some("{{evil}}".into()), sensitive: false }),
        ];
        let w = compile(None, &events).workflow;
        assert!(validate(&w).is_empty());
    }
}

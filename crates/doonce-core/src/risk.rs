//! Detects steps that may have real-world consequences (payments,
//! deletions, messages, permission changes) so they are never run without
//! the user's explicit approval.

use crate::workflow::{Action, Step, Target, Workflow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskCategory {
    Payment,
    Transfer,
    Deletion,
    Message,
    Permissions,
    Security,
    Irreversible,
}

impl RiskCategory {
    pub fn label(self) -> &'static str {
        match self {
            RiskCategory::Payment => "payment or purchase",
            RiskCategory::Transfer => "money transfer",
            RiskCategory::Deletion => "deleting data",
            RiskCategory::Message => "sending a message",
            RiskCategory::Permissions => "changing permissions",
            RiskCategory::Security => "changing security settings",
            RiskCategory::Irreversible => "an action that may not be undoable",
        }
    }
}

/// Single words (matched as whole tokens) and multi-word phrases.
const RULES: &[(RiskCategory, &[&str])] = &[
    (RiskCategory::Payment, &[
        "pay", "payment", "purchase", "buy", "checkout", "place order", "order now", "subscribe",
        "donate", "upgrade", "öde", "ödeme", "satın", "satın al", "abone ol",
    ]),
    (RiskCategory::Transfer, &["transfer", "wire", "send money", "withdraw", "havale", "eft", "para gönder"]),
    (RiskCategory::Deletion, &[
        "delete", "remove", "destroy", "erase", "purge", "trash", "discard", "sil", "kaldır", "temizle",
    ]),
    (RiskCategory::Message, &[
        "send", "email", "mail", "post", "publish", "reply", "tweet", "message", "submit message",
        "gönder", "yayınla", "yanıtla",
    ]),
    (RiskCategory::Permissions, &[
        "grant", "revoke", "permission", "permissions", "make admin", "change role", "invite",
        "yetki", "izin ver",
    ]),
    (RiskCategory::Security, &[
        "change password", "reset password", "disable 2fa", "disable two-factor", "security settings",
        "deactivate", "şifre değiştir", "güvenlik",
    ]),
    (RiskCategory::Irreversible, &[
        "confirm", "permanently", "irreversible", "close account", "terminate", "cancel subscription",
        "onayla", "kalıcı",
    ]),
];

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn target_text(t: &Target) -> String {
    let mut parts: Vec<&str> = [&t.name, &t.text, &t.label].into_iter().flatten().map(String::as_str).collect();
    for key in ["aria-label", "title", "id", "name", "data-testid", "data-action"] {
        if let Some(v) = t.attributes.get(key) {
            parts.push(v);
        }
    }
    // `delete-account` style identifiers become separate tokens.
    parts.join(" ").replace(['-', '_'], " ")
}

/// Classifies the text describing an element.
pub fn classify_text(text: &str) -> Option<RiskCategory> {
    let toks = tokens(text);
    let joined = format!(" {} ", toks.join(" "));
    for (category, words) in RULES {
        for w in *words {
            let hit = if w.contains(' ') {
                joined.contains(&format!(" {w} "))
            } else {
                toks.iter().any(|t| t == w)
            };
            if hit {
                return Some(*category);
            }
        }
    }
    None
}

/// The heuristic risk of a step, ignoring any explicit `requiresApproval`.
pub fn classify(step: &Step) -> Option<RiskCategory> {
    match &step.action {
        Action::Click { target } | Action::Download { target: Some(target) } => {
            classify_text(&target_text(target))
        }
        Action::Press { target: Some(target), key } if key.eq_ignore_ascii_case("enter") => {
            classify_text(&target_text(target))
        }
        _ => None,
    }
}

/// Why a step must be approved before it runs, if it must.
pub fn approval_reason(step: &Step) -> Option<String> {
    match step.requires_approval {
        Some(true) => Some("This step is marked as requiring approval.".into()),
        Some(false) => None,
        None => classify(step).map(|c| format!("This step looks like {}.", c.label())),
    }
}

/// Imported workflows cannot opt risky steps out of approval: an explicit
/// `requiresApproval: false` on a risky-looking step is reset.
pub fn harden_imported(w: &mut Workflow) -> Vec<String> {
    let mut notes = Vec::new();
    for step in &mut w.steps {
        if step.requires_approval == Some(false) {
            if let Some(c) = classify(step) {
                step.requires_approval = None;
                notes.push(format!("{}: approval re-enabled because it looks like {}", step.id, c.label()));
            }
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(name: &str) -> Step {
        Step {
            id: "s".into(),
            action: Action::Click {
                target: Target { role: Some("button".into()), name: Some(name.into()), ..Default::default() },
            },
            description: None,
            timeout_ms: None,
            retry: None,
            requires_approval: None,
        }
    }

    #[test]
    fn classifies_risky_buttons() {
        assert_eq!(classify(&click("Delete account")), Some(RiskCategory::Deletion));
        assert_eq!(classify(&click("Pay now")), Some(RiskCategory::Payment));
        assert_eq!(classify(&click("Send email")), Some(RiskCategory::Message));
        assert_eq!(classify(&click("Hesabı Sil")), Some(RiskCategory::Deletion));
        assert_eq!(classify(&click("Generate Report")), None);
        assert_eq!(classify(&click("Save")), None);
        // Whole-word matching: "display" must not match "pay".
        assert_eq!(classify(&click("Display options")), None);
    }

    #[test]
    fn identifiers_count_too() {
        let mut s = click("");
        if let Action::Click { target } = &mut s.action {
            target.name = None;
            target.attributes.insert("id".into(), "delete-account".into());
        }
        assert_eq!(classify(&s), Some(RiskCategory::Deletion));
    }

    #[test]
    fn explicit_flags_and_import_hardening() {
        let mut s = click("Delete account");
        s.requires_approval = Some(false);
        assert!(approval_reason(&s).is_none());
        let mut w = Workflow {
            schema_version: "1.0".into(),
            name: "x".into(),
            description: String::new(),
            parameters: vec![],
            steps: vec![s, { let mut o = click("Open"); o.requires_approval = Some(true); o }],
        };
        let notes = harden_imported(&mut w);
        assert_eq!(notes.len(), 1);
        assert!(approval_reason(&w.steps[0]).is_some());
        assert!(approval_reason(&w.steps[1]).is_some());
    }
}

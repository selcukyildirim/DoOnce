//! Risk classification for actions that may have real-world consequences.
//!
//! The classifier is deliberately conservative. It is a *signal* used to
//! require explicit approval; it is not an authorization decision on its own.

use crate::workflow::{Block, BlockKind, BrowserAction, Target};

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

const RULES: &[(RiskCategory, &[&str])] = &[
    (
        RiskCategory::Payment,
        &[
            "pay",
            "payment",
            "purchase",
            "buy",
            "checkout",
            "place order",
            "order now",
            "subscribe",
            "donate",
            "upgrade",
            "öde",
            "ödeme",
            "satın",
            "satın al",
            "abone ol",
        ],
    ),
    (
        RiskCategory::Transfer,
        &[
            "transfer",
            "wire",
            "send money",
            "withdraw",
            "havale",
            "eft",
            "para gönder",
        ],
    ),
    (
        RiskCategory::Deletion,
        &[
            "delete", "remove", "destroy", "erase", "purge", "trash", "discard", "sil", "kaldır",
            "temizle",
        ],
    ),
    (
        RiskCategory::Message,
        &[
            "send",
            "email",
            "mail",
            "post",
            "publish",
            "reply",
            "tweet",
            "message",
            "submit message",
            "gönder",
            "yayınla",
            "yanıtla",
        ],
    ),
    (
        RiskCategory::Permissions,
        &[
            "grant",
            "revoke",
            "permission",
            "permissions",
            "make admin",
            "change role",
            "invite",
            "yetki",
            "izin ver",
        ],
    ),
    (
        RiskCategory::Security,
        &[
            "change password",
            "reset password",
            "disable 2fa",
            "disable two-factor",
            "security settings",
            "deactivate",
            "şifre değiştir",
            "güvenlik",
        ],
    ),
    (
        RiskCategory::Irreversible,
        &[
            "confirm",
            "permanently",
            "irreversible",
            "close account",
            "terminate",
            "cancel subscription",
            "onayla",
            "kalıcı",
        ],
    ),
];

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn target_text(t: &Target) -> String {
    let mut parts: Vec<&str> = [&t.name, &t.text, &t.label]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    for key in [
        "aria-label",
        "title",
        "id",
        "name",
        "data-testid",
        "data-action",
    ] {
        if let Some(v) = t.attributes.get(key) {
            parts.push(v);
        }
    }
    parts.join(" ").replace(['-', '_'], " ")
}

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

pub fn classify_block(block: &Block) -> Option<RiskCategory> {
    match &block.kind {
        BlockKind::BrowserTask { prompt, .. } => classify_text(prompt),
        BlockKind::BrowserAction { action } => match action {
            BrowserAction::Click { target }
            | BrowserAction::Download {
                target: Some(target),
            } => classify_text(&target_text(target)),
            BrowserAction::Press {
                target: Some(target),
                key,
            } if key.eq_ignore_ascii_case("enter") => classify_text(&target_text(target)),
            _ => None,
        },
        BlockKind::SendEmail { .. } => Some(RiskCategory::Message),
        BlockKind::HttpRequest { method, .. } if method.eq_ignore_ascii_case("DELETE") => {
            Some(RiskCategory::Deletion)
        }
        _ => None,
    }
}

/// Why a block must be approved before it runs, if it must.
pub fn approval_reason(block: &Block) -> Option<String> {
    match block.requires_approval {
        Some(true) => Some("This block is marked as requiring approval.".into()),
        Some(false) => None,
        None => classify_block(block).map(|c| format!("This block looks like {}.", c.label())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{Block, BlockKind, BrowserAction, Target, TerminateStatus};

    fn action(name: &str) -> Block {
        Block {
            id: "b".into(),
            kind: BlockKind::BrowserAction {
                action: BrowserAction::Click {
                    target: Target {
                        role: Some("button".into()),
                        name: Some(name.into()),
                        ..Default::default()
                    },
                },
            },
            label: None,
            description: None,
            next: None,
            on_error: None,
            retry: None,
            timeout_ms: None,
            continue_on_failure: None,
            requires_approval: None,
        }
    }

    #[test]
    fn classifies_risky_actions() {
        assert_eq!(
            classify_block(&action("Delete account")),
            Some(RiskCategory::Deletion)
        );
        assert_eq!(
            classify_block(&action("Pay now")),
            Some(RiskCategory::Payment)
        );
        assert_eq!(
            classify_block(&action("Send email")),
            Some(RiskCategory::Message)
        );
        assert_eq!(
            classify_block(&action("Hesabı Sil")),
            Some(RiskCategory::Deletion)
        );
        assert_eq!(classify_block(&action("Generate report")), None);
        assert_eq!(classify_block(&action("Display options")), None);
    }

    #[test]
    fn explicit_flags_win() {
        let mut b = action("Delete account");
        b.requires_approval = Some(false);
        assert!(approval_reason(&b).is_none());
        b.requires_approval = Some(true);
        assert!(approval_reason(&b).is_some());
    }

    #[test]
    fn non_browser_blocks() {
        let mut b = action("x");
        b.kind = BlockKind::Terminate {
            status: TerminateStatus::Completed,
            reason: None,
        };
        assert!(classify_block(&b).is_none());
    }
}

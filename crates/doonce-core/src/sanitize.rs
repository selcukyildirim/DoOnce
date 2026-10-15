//! Removes credentials and session data from workflows before they are
//! stored or shared.

use crate::workflow::{Action, Target, Workflow};

/// Query parameters that commonly carry credentials or session state.
const SENSITIVE_QUERY_KEYS: &[&str] = &[
    "token", "access_token", "id_token", "refresh_token", "auth", "authorization", "session",
    "sessionid", "session_id", "sid", "jsessionid", "phpsessid", "key", "apikey", "api_key",
    "password", "passwd", "pwd", "secret", "code", "state", "sig", "signature", "jwt", "otp",
    "nonce", "ticket", "x-amz-signature", "x-amz-credential", "x-amz-security-token",
];

const ALLOWED_ATTRIBUTES: &[&str] = &[
    "id", "name", "type", "placeholder", "aria-label", "title", "alt", "href", "for", "role",
    "autocomplete", "class",
];

const MAX_ATTRIBUTE_VALUE: usize = 300;

/// Returns true when a query key looks like it carries a credential.
pub fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    SENSITIVE_QUERY_KEYS.contains(&k.as_str()) || k.ends_with("_token") || k.ends_with("token")
}

/// Strips user info and credential-like query parameters from a URL. URLs
/// that contain `{{templates}}` or do not parse are returned unchanged.
pub fn sanitize_url(input: &str) -> (String, bool) {
    if input.contains("{{") {
        return (input.to_string(), false);
    }
    let Ok(mut url) = url::Url::parse(input) else {
        return (input.to_string(), false);
    };
    let mut changed = false;
    if !url.username().is_empty() || url.password().is_some() {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        changed = true;
    }
    if url.query().is_some() {
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let kept: Vec<&(String, String)> = pairs.iter().filter(|(k, _)| !is_sensitive_key(k)).collect();
        if kept.len() != pairs.len() {
            changed = true;
            if kept.is_empty() {
                url.set_query(None);
            } else {
                let mut q = url.query_pairs_mut();
                q.clear();
                for (k, v) in kept {
                    q.append_pair(k, v);
                }
            }
        }
    }
    if let Some(fragment) = url.fragment() {
        // OAuth implicit flows put tokens in the fragment.
        if fragment.split('&').any(|kv| is_sensitive_key(kv.split('=').next().unwrap_or(""))) {
            url.set_fragment(None);
            changed = true;
        }
    }
    (url.to_string(), changed)
}

fn sanitize_target(t: &mut Target, warnings: &mut Vec<String>, step_id: &str) {
    let before = t.attributes.len();
    t.attributes.retain(|k, v| {
        (ALLOWED_ATTRIBUTES.contains(&k.as_str()) || (k.starts_with("data-") && k.len() <= 40))
            && v.len() <= MAX_ATTRIBUTE_VALUE
    });
    if t.attributes.len() != before {
        warnings.push(format!("{step_id}: removed unsupported element attributes"));
    }
    if let Some(href) = t.attributes.get_mut("href") {
        let (clean, changed) = sanitize_url(href);
        if changed {
            *href = clean;
            warnings.push(format!("{step_id}: removed credentials from a link URL"));
        }
    }
}

/// Cleans a workflow in place and returns human-readable notes about what
/// was changed.
pub fn sanitize_workflow(w: &mut Workflow) -> Vec<String> {
    let mut warnings = Vec::new();
    for step in &mut w.steps {
        let id = step.id.clone();
        match &mut step.action {
            Action::Navigate { url } => {
                let (clean, changed) = sanitize_url(url);
                if changed {
                    *url = clean;
                    warnings.push(format!("{id}: removed credentials or session data from URL"));
                }
            }
            Action::Wait { condition } | Action::Assert { condition } => {
                for t in [&mut condition.visible, &mut condition.hidden].into_iter().flatten() {
                    sanitize_target(t, &mut warnings, &id);
                }
            }
            action => {
                if let Some(t) = action.target_mut() {
                    sanitize_target(t, &mut warnings, &id);
                }
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_credentials_from_urls() {
        let (u, c) = sanitize_url("https://bob:pw@example.com/a?page=2&access_token=abc&sessionId=1#x");
        assert!(c);
        assert_eq!(u, "https://example.com/a?page=2#x");
        let (u, c) = sanitize_url("https://example.com/cb#access_token=abc&type=bearer");
        assert!(c);
        assert_eq!(u, "https://example.com/cb");
        let (u, c) = sanitize_url("https://example.com/r?date=2026-01-01");
        assert!(!c);
        assert_eq!(u, "https://example.com/r?date=2026-01-01");
        let (u, c) = sanitize_url("https://example.com/r?d={{date}}&token=x");
        assert!(!c, "templated urls are left alone");
        assert!(u.contains("{{date}}"));
    }

    #[test]
    fn strips_unsafe_attributes() {
        let mut t = Target::default();
        t.attributes.insert("value".into(), "secret".into());
        t.attributes.insert("onclick".into(), "steal()".into());
        t.attributes.insert("data-testid".into(), "save".into());
        t.attributes.insert("href".into(), "https://x.com/?token=1".into());
        let mut warnings = vec![];
        sanitize_target(&mut t, &mut warnings, "s1");
        assert_eq!(t.attributes.len(), 2);
        assert_eq!(t.attributes["href"], "https://x.com/");
        assert!(!warnings.is_empty());
    }
}

use chrono::{SecondsFormat, Utc};

/// Generates a time-sortable UUIDv7 identifier rendered as a lowercase string.
///
/// IDs are stored as `TEXT` in both SQLite and Postgres so the same repository
/// code works on every supported deployment. Postgres deployments may migrate
/// to native `UUID` columns without changing the wire format.
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A slug is a URL-safe, lowercase identifier. Common Turkish and Latin
/// diacritics are transliterated so names keep their meaning.
pub fn slugify(name: &str) -> String {
    fn transliterate(c: char) -> Option<&'static str> {
        match c {
            'ç' | 'Ç' => Some("c"),
            'ğ' | 'Ğ' => Some("g"),
            'ı' | 'İ' => Some("i"),
            'ö' | 'Ö' => Some("o"),
            'ş' | 'Ş' => Some("s"),
            'ü' | 'Ü' => Some("u"),
            'á' | 'à' | 'â' | 'ä' | 'å' | 'ã' => Some("a"),
            'é' | 'è' | 'ê' | 'ë' => Some("e"),
            'í' | 'ì' | 'î' | 'ï' => Some("i"),
            'ó' | 'ò' | 'ô' | 'õ' | 'ø' => Some("o"),
            'ú' | 'ù' | 'û' => Some("u"),
            'ñ' => Some("n"),
            'ß' => Some("ss"),
            'æ' => Some("ae"),
            'œ' => Some("oe"),
            _ => None,
        }
    }

    let mut out = String::new();
    let mut last_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if let Some(replacement) = transliterate(c) {
            out.push_str(replacement);
            last_dash = false;
        } else if !c.is_alphanumeric() && !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 80 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "automation".to_string()
    } else {
        out
    }
}

/// Parameter and variable names: `[A-Za-z_][A-Za-z0-9_]*`.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && name.len() <= 64
}

/// Block identifiers additionally allow dashes.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_and_stable() {
        assert_eq!(slugify("Daily Report 2026"), "daily-report-2026");
        assert_eq!(slugify("  ---  "), "automation");
        assert_eq!(slugify("Çok Güzel"), "cok-guzel");
    }

    #[test]
    fn ids_and_names() {
        assert!(is_valid_id("step-1"));
        assert!(is_valid_id("a_b"));
        assert!(!is_valid_id("-step"));
        assert!(!is_valid_id(""));
        assert!(is_valid_name("reportDate"));
        assert!(is_valid_name("_x"));
        assert!(!is_valid_name("1x"));
        assert!(!is_valid_name("a-b"));
    }

    #[test]
    fn ids_are_sortable() {
        let a = new_id();
        let b = new_id();
        assert!(a < b);
    }
}

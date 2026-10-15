//! Core types shared by every DoOnce component: the workflow format, its
//! validation rules, parameter handling, privacy sanitization, risk
//! classification and the compiler that turns raw recordings into workflows.

pub mod params;
pub mod recording;
pub mod risk;
pub mod sanitize;
pub mod template;
pub mod validate;
pub mod workflow;

pub use params::{ParamError, ResolvedParams};
pub use validate::{Issue, parse_workflow};
pub use workflow::*;

/// Turns a human name into a stable, URL- and CLI-friendly identifier.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'ç' => 'c',
            'ğ' => 'g',
            'ı' => 'i',
            'ö' | 'ó' | 'ò' | 'ô' => 'o',
            'ş' => 's',
            'ü' | 'ú' | 'ù' | 'û' => 'u',
            'á' | 'à' | 'â' | 'ä' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            other => other,
        };
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() { "automation".into() } else { slug }
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugify_names() {
        assert_eq!(slugify("Daily Report"), "daily-report");
        assert_eq!(slugify("  Fill   Contact form!! "), "fill-contact-form");
        assert_eq!(slugify("Günlük Rapor"), "gunluk-rapor");
        assert_eq!(slugify("***"), "automation");
    }
}

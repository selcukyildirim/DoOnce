//! `{{name}}` placeholders inside workflow strings.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("unclosed '{{{{' in \"{0}\"")]
    Unclosed(String),
    #[error("invalid parameter name \"{0}\"")]
    InvalidName(String),
    #[error("unknown parameter \"{0}\"")]
    Unknown(String),
}

enum Piece<'a> {
    Text(&'a str),
    Param(&'a str),
}

fn parse(input: &str) -> Result<Vec<Piece<'_>>, TemplateError> {
    let mut pieces = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            pieces.push(Piece::Text(&rest[..start]));
        }
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| TemplateError::Unclosed(input.into()))?;
        let name = after[..end].trim();
        if !is_valid_name(name) {
            return Err(TemplateError::InvalidName(name.into()));
        }
        pieces.push(Piece::Param(name));
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest));
    }
    Ok(pieces)
}

pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 64
}

/// Parameter names referenced by `input`.
pub fn references(input: &str) -> Result<Vec<String>, TemplateError> {
    Ok(parse(input)?
        .into_iter()
        .filter_map(|p| match p {
            Piece::Param(name) => Some(name.to_string()),
            Piece::Text(_) => None,
        })
        .collect())
}

pub fn render(input: &str, values: &HashMap<String, String>) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(input.len());
    for piece in parse(input)? {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Param(name) => out.push_str(
                values.get(name).ok_or_else(|| TemplateError::Unknown(name.into()))?,
            ),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_lists_references() {
        let values = HashMap::from([("date".to_string(), "2026-10-15".to_string())]);
        assert_eq!(render("/r?d={{date}}&x={{ date }}", &values).unwrap(), "/r?d=2026-10-15&x=2026-10-15");
        assert_eq!(references("a {{b}} c {{ d_1 }}").unwrap(), vec!["b", "d_1"]);
        assert_eq!(render("plain", &values).unwrap(), "plain");
    }

    #[test]
    fn rejects_bad_templates() {
        let values = HashMap::new();
        assert!(matches!(render("{{x", &values), Err(TemplateError::Unclosed(_))));
        assert!(matches!(render("{{1x}}", &values), Err(TemplateError::InvalidName(_))));
        assert!(matches!(render("{{x}}", &values), Err(TemplateError::Unknown(_))));
    }
}

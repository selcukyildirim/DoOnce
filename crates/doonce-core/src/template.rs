//! Safe template reference parsing.
//!
//! DoOnce uses `{{name}}` and `{{blockId.output.field}}` placeholders. We only
//! parse and validate references here; rendering is deliberately kept in the
//! engine so a persisted workflow can never execute arbitrary code.

use std::collections::HashMap;

use thiserror::Error;

pub const MAX_TEMPLATE_BYTES: usize = 4000;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateError {
    #[error("unclosed template expression (missing `}}}}`)")]
    Unclosed,
    #[error("empty template expression")]
    Empty,
    #[error("invalid template reference \"{0}\"")]
    InvalidReference(String),
    #[error("template value \"{0}\" is not available")]
    Unknown(String),
    #[error("template is longer than {MAX_TEMPLATE_BYTES} bytes")]
    TooLong,
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 64
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn valid_root(root: &str) -> bool {
    let mut chars = root.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Returns the distinct roots referenced by `input`, in first-seen order.
pub fn references(input: &str) -> Result<Vec<String>, TemplateError> {
    if input.len() > MAX_TEMPLATE_BYTES {
        return Err(TemplateError::TooLong);
    }
    let mut out: Vec<String> = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(TemplateError::Unclosed);
        };
        let inner = after[..end].trim();
        if inner.is_empty() {
            return Err(TemplateError::Empty);
        }
        let mut parts = inner.split('.');
        let root = parts.next().unwrap_or_default();
        if !valid_root(root) {
            return Err(TemplateError::InvalidReference(inner.to_string()));
        }
        for part in parts {
            if !valid_segment(part) {
                return Err(TemplateError::InvalidReference(inner.to_string()));
            }
        }
        if !out.iter().any(|r| r == root) {
            out.push(root.to_string());
        }
        rest = &after[end + 2..];
    }
    Ok(out)
}

/// Renders `{{name}}` references from `values`.
///
/// Nested paths are looked up verbatim first (`inputs.date`) and then by root
/// (`inputs`). The engine populates nested values explicitly, so no expression
/// evaluation ever happens here.
pub fn render(input: &str, values: &HashMap<String, String>) -> Result<String, TemplateError> {
    if input.len() > MAX_TEMPLATE_BYTES {
        return Err(TemplateError::TooLong);
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(TemplateError::Unclosed);
        };
        let inner = after[..end].trim();
        if inner.is_empty() {
            return Err(TemplateError::Empty);
        }
        let mut parts = inner.split('.');
        let root = parts.next().unwrap_or_default();
        if !valid_root(root) {
            return Err(TemplateError::InvalidReference(inner.to_string()));
        }
        for part in parts {
            if !valid_segment(part) {
                return Err(TemplateError::InvalidReference(inner.to_string()));
            }
        }
        let value = values
            .get(inner)
            .or_else(|| values.get(root))
            .ok_or_else(|| TemplateError::Unknown(inner.to_string()))?;
        out.push_str(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_roots() {
        assert_eq!(references("Hi {{name}}").unwrap(), vec!["name"]);
        assert_eq!(
            references("{{inputs.date}}/{{step-2.output.total}}").unwrap(),
            vec!["inputs", "step-2"]
        );
        assert_eq!(references("no templates").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn rejects_bad_references() {
        assert_eq!(references("{{"), Err(TemplateError::Unclosed));
        assert_eq!(references("{{}}"), Err(TemplateError::Empty));
        assert!(matches!(
            references("{{1x}}"),
            Err(TemplateError::InvalidReference(_))
        ));
        assert!(matches!(
            references("{{a..b}}"),
            Err(TemplateError::InvalidReference(_))
        ));
    }

    #[test]
    fn renders_values_and_nested_paths() {
        let values = HashMap::from([
            ("name".to_string(), "Ada".to_string()),
            ("inputs.date".to_string(), "2026-11-06".to_string()),
            ("total".to_string(), "42".to_string()),
        ]);
        assert_eq!(
            render("Hi {{name}}, {{inputs.date}}", &values).unwrap(),
            "Hi Ada, 2026-11-06"
        );
        assert_eq!(render("{{name}}{{name}}", &values).unwrap(), "AdaAda");
        assert_eq!(render("no refs", &values).unwrap(), "no refs");
        assert!(matches!(
            render("{{missing}}", &values),
            Err(TemplateError::Unknown(_))
        ));
        assert_eq!(render("{{", &values), Err(TemplateError::Unclosed));
    }
}

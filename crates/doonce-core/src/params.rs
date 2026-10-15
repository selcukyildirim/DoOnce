//! Resolution of run-time parameter values.

use std::collections::HashMap;

use crate::template::{self, TemplateError};
use crate::workflow::{ParamType, Workflow};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParamError {
    #[error("missing value for parameter \"{0}\"")]
    Missing(String),
    #[error("unknown parameter \"{0}\"")]
    Unknown(String),
    #[error("parameter \"{name}\" must be {expected}")]
    Invalid { name: String, expected: &'static str },
    #[error(transparent)]
    Template(#[from] TemplateError),
}

/// Parameter values ready for substitution. Secret values are kept out of
/// `Debug` output so they cannot end up in logs by accident.
#[derive(Clone, Default)]
pub struct ResolvedParams {
    values: HashMap<String, String>,
    secrets: Vec<String>,
}

impl std::fmt::Debug for ResolvedParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        for (k, v) in &self.values {
            if self.secrets.contains(k) {
                map.entry(k, &"••••••");
            } else {
                map.entry(k, v);
            }
        }
        map.finish()
    }
}

impl ResolvedParams {
    /// Validates `provided` against the workflow's declared parameters and
    /// fills in defaults.
    pub fn resolve(workflow: &Workflow, provided: &HashMap<String, String>) -> Result<Self, ParamError> {
        for name in provided.keys() {
            if !workflow.parameters.iter().any(|p| &p.name == name) {
                return Err(ParamError::Unknown(name.clone()));
            }
        }
        let mut out = ResolvedParams::default();
        for p in &workflow.parameters {
            let value = provided
                .get(&p.name)
                .cloned()
                .or_else(|| if p.kind == ParamType::Secret { None } else { p.default.clone() })
                .ok_or_else(|| ParamError::Missing(p.name.clone()))?;
            check_type(&p.name, p.kind, &value)?;
            if p.kind == ParamType::Secret {
                out.secrets.push(p.name.clone());
            }
            out.values.insert(p.name.clone(), value);
        }
        Ok(out)
    }

    pub fn render(&self, input: &str) -> Result<String, ParamError> {
        Ok(template::render(input, &self.values)?)
    }

    /// Masks every secret value that appears in `text` (for error messages
    /// that may echo page or driver output).
    pub fn scrub(&self, text: &str) -> String {
        let mut out = text.to_string();
        for name in &self.secrets {
            if let Some(v) = self.values.get(name) {
                if v.chars().count() >= 3 {
                    out = out.replace(v.as_str(), "••••••");
                }
            }
        }
        out
    }

    /// Whether `input` references a secret parameter.
    pub fn touches_secret(&self, input: &str) -> bool {
        template::references(input)
            .map(|refs| refs.iter().any(|r| self.secrets.contains(r)))
            .unwrap_or(false)
    }
}

pub fn check_type(name: &str, kind: ParamType, value: &str) -> Result<(), ParamError> {
    let ok = match kind {
        ParamType::String | ParamType::Secret => true,
        ParamType::Number => value.trim().parse::<f64>().is_ok_and(f64::is_finite),
        ParamType::Date => is_iso_date(value),
    };
    if ok {
        Ok(())
    } else {
        Err(ParamError::Invalid {
            name: name.into(),
            expected: match kind {
                ParamType::Number => "a number",
                _ => "a date in YYYY-MM-DD format",
            },
        })
    }
}

fn is_iso_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>(), parts[2].parse::<u32>())
    else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{Parameter, SCHEMA_VERSION};

    fn wf(params: Vec<Parameter>) -> Workflow {
        Workflow {
            schema_version: SCHEMA_VERSION.into(),
            name: "t".into(),
            description: String::new(),
            parameters: params,
            steps: vec![],
        }
    }

    fn param(name: &str, kind: ParamType, default: Option<&str>) -> Parameter {
        Parameter { name: name.into(), kind, default: default.map(Into::into), description: None }
    }

    #[test]
    fn resolves_defaults_and_types() {
        let w = wf(vec![
            param("day", ParamType::Date, Some("2026-01-31")),
            param("n", ParamType::Number, None),
        ]);
        let p = ResolvedParams::resolve(&w, &HashMap::from([("n".into(), "3".into())])).unwrap();
        assert_eq!(p.render("{{day}}/{{n}}").unwrap(), "2026-01-31/3");

        let err = ResolvedParams::resolve(&w, &HashMap::new()).unwrap_err();
        assert_eq!(err, ParamError::Missing("n".into()));

        let bad = HashMap::from([("n".into(), "1".into()), ("day".into(), "2026-02-30".into())]);
        assert!(matches!(ResolvedParams::resolve(&w, &bad), Err(ParamError::Invalid { .. })));

        let unknown = HashMap::from([("n".into(), "1".into()), ("zzz".into(), "1".into())]);
        assert_eq!(ResolvedParams::resolve(&w, &unknown).unwrap_err(), ParamError::Unknown("zzz".into()));
    }

    #[test]
    fn secrets_ignore_defaults_and_are_hidden_from_debug() {
        let w = wf(vec![param("password", ParamType::Secret, Some("leaked"))]);
        assert!(ResolvedParams::resolve(&w, &HashMap::new()).is_err());
        let p = ResolvedParams::resolve(&w, &HashMap::from([("password".into(), "hunter2".into())])).unwrap();
        assert!(!format!("{p:?}").contains("hunter2"));
        assert!(p.touches_secret("{{password}}"));
        assert_eq!(p.scrub("eval failed near 'hunter2'"), "eval failed near '••••••'");
        assert!(!p.touches_secret("hello"));
    }

    #[test]
    fn iso_dates() {
        assert!(is_iso_date("2024-02-29"));
        assert!(!is_iso_date("2023-02-29"));
        assert!(!is_iso_date("2026-13-01"));
        assert!(!is_iso_date("26-1-1"));
    }
}

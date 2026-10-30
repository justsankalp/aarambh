//! Failure modes from `data/taxonomy.toml`.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Taxonomy {
    #[serde(rename = "mode", default)]
    pub modes: Vec<FailureMode>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FailureMode {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub hint: String,
}

/// The shipped defaults, used until the repo's taxonomy has been fetched.
const DEFAULTS: [(&str, &str); 12] = [
    ("no-paying-customer", "No paying customer"),
    ("no-clear-wedge", "No clear wedge"),
    ("market-too-small", "Market too small"),
    ("crowded-no-edge", "Crowded, no edge"),
    ("slow-to-validate", "Slow to validate"),
    ("capability-gap", "Capability gap"),
    ("hard-to-reach-users", "Hard to reach users"),
    ("trust-or-regulation", "Trust or regulation"),
    ("unit-economics", "Unit economics"),
    ("bad-timing", "Bad timing"),
    ("distribution-dependency", "Distribution dependency"),
    ("low-conviction", "Low conviction"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaxonomyError {
    Parse(String),
    Empty,
    BadId(String),
    DuplicateId(String),
}

impl std::fmt::Display for TaxonomyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaxonomyError::Parse(e) => write!(f, "taxonomy.toml: {e}"),
            TaxonomyError::Empty => write!(f, "taxonomy.toml has no [[mode]] entries"),
            TaxonomyError::BadId(id) => write!(
                f,
                "taxonomy id {id:?} must be lowercase letters, digits and dashes"
            ),
            TaxonomyError::DuplicateId(id) => write!(f, "taxonomy id {id:?} appears twice"),
        }
    }
}

impl std::error::Error for TaxonomyError {}

impl Default for Taxonomy {
    fn default() -> Self {
        Taxonomy {
            modes: DEFAULTS
                .iter()
                .map(|(id, label)| FailureMode {
                    id: (*id).into(),
                    label: (*label).into(),
                    hint: String::new(),
                })
                .collect(),
        }
    }
}

impl Taxonomy {
    pub fn parse(text: &str) -> Result<Self, TaxonomyError> {
        let t: Taxonomy = toml::from_str(text).map_err(|e| TaxonomyError::Parse(e.to_string()))?;
        if t.modes.is_empty() {
            return Err(TaxonomyError::Empty);
        }
        let mut seen = std::collections::HashSet::new();
        for m in &t.modes {
            let ok = !m.id.is_empty()
                && m.id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-');
            if !ok {
                return Err(TaxonomyError::BadId(m.id.clone()));
            }
            if !seen.insert(m.id.as_str()) {
                return Err(TaxonomyError::DuplicateId(m.id.clone()));
            }
        }
        Ok(t)
    }

    pub fn get(&self, id: &str) -> Option<&FailureMode> {
        self.modes.iter().find(|m| m.id == id)
    }

    /// Label for a mode id, falling back to the id itself for modes that
    /// were removed from the file after a lesson used them.
    pub fn label<'a>(&'a self, id: &'a str) -> &'a str {
        self.get(id).map_or(id, |m| m.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO_FILE: &str = include_str!("../../../data/taxonomy.toml");

    #[test]
    fn repo_file_and_defaults_are_valid() {
        // The repo file is meant to be edited, so only check that it parses.
        let t = Taxonomy::parse(REPO_FILE).unwrap();
        assert!(t.modes.iter().all(|m| !m.label.is_empty()));
        let defaults = Taxonomy::default();
        let as_toml = defaults
            .modes
            .iter()
            .map(|m| format!("[[mode]]\nid = \"{}\"\nlabel = \"{}\"\n", m.id, m.label))
            .collect::<String>();
        assert_eq!(Taxonomy::parse(&as_toml).unwrap(), defaults);
        assert_eq!(defaults.label("unit-economics"), "Unit economics");
        assert_eq!(defaults.label("since-removed"), "since-removed");
    }

    #[test]
    fn rejects_bad_files() {
        assert_eq!(Taxonomy::parse(""), Err(TaxonomyError::Empty));
        assert!(matches!(
            Taxonomy::parse("[[mode]]\nid = 3"),
            Err(TaxonomyError::Parse(_))
        ));
        assert_eq!(
            Taxonomy::parse("[[mode]]\nid = \"Bad Id\"\nlabel = \"x\""),
            Err(TaxonomyError::BadId("Bad Id".into()))
        );
        assert_eq!(
            Taxonomy::parse(
                "[[mode]]\nid = \"a\"\nlabel = \"x\"\n[[mode]]\nid = \"a\"\nlabel = \"y\""
            ),
            Err(TaxonomyError::DuplicateId("a".into()))
        );
    }
}

//! Failure modes from `data/taxonomy.toml`.

use serde::{Deserialize, Serialize};

// Serde derives stay so the native judge can read the same file with `toml`.

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
    /// Parses `taxonomy.toml`. Only the subset the file needs is supported
    /// (`[[mode]]` tables of `key = "string"` lines, and `#` comments), which
    /// keeps a full TOML parser out of the web app. Unknown keys are ignored.
    pub fn parse(text: &str) -> Result<Self, TaxonomyError> {
        let mut modes: Vec<FailureMode> = Vec::new();
        let mut current: Option<(Option<String>, Option<String>, String)> = None;
        let finish = |cur: Option<(Option<String>, Option<String>, String)>, line: usize| match cur
        {
            None => Ok(None),
            Some((Some(id), Some(label), hint)) => Ok(Some(FailureMode { id, label, hint })),
            Some((id, _, _)) => Err(TaxonomyError::Parse(format!(
                "the [[mode]] ending before line {line} needs both id and label{}",
                id.map(|i| format!(" ({i})")).unwrap_or_default()
            ))),
        };
        for (n, raw) in text.lines().enumerate() {
            let line_no = n + 1;
            let err = |what: &str| TaxonomyError::Parse(format!("line {line_no}: {what}"));
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                if strip_comment(line) != "[[mode]]" {
                    return Err(err("only [[mode]] tables are supported"));
                }
                modes.extend(finish(current.take(), line_no)?);
                current = Some((None, None, String::new()));
                continue;
            }
            let Some((key, rest)) = line.split_once('=') else {
                return Err(err("expected key = \"value\""));
            };
            let key = key.trim();
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err(err("expected a plain key before ="));
            }
            let value =
                parse_string(rest.trim()).ok_or_else(|| err("values must be quoted strings"))?;
            let Some(cur) = current.as_mut() else {
                return Err(err("key outside a [[mode]] table"));
            };
            match key {
                "id" => cur.0 = Some(value),
                "label" => cur.1 = Some(value),
                "hint" => cur.2 = value,
                _ => {}
            }
        }
        modes.extend(finish(current.take(), text.lines().count() + 1)?);

        if modes.is_empty() {
            return Err(TaxonomyError::Empty);
        }
        let mut seen = std::collections::HashSet::new();
        for m in &modes {
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
        Ok(Taxonomy { modes })
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

/// Drops a trailing `# comment` from a line with no strings in it.
fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim()
}

/// A TOML basic ("...") or literal ('...') string, optionally followed by a
/// comment. Returns `None` for anything else.
fn parse_string(s: &str) -> Option<String> {
    let mut chars = s.chars();
    let quote = chars.next()?;
    let mut out = String::new();
    match quote {
        '\'' => {
            for c in chars.by_ref() {
                if c == '\'' {
                    return after_value(chars.as_str()).then_some(out);
                }
                out.push(c);
            }
            None
        }
        '"' => {
            while let Some(c) = chars.next() {
                match c {
                    '"' => return after_value(chars.as_str()).then_some(out),
                    '\\' => match chars.next()? {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'u' => {
                            let hex: String = chars.by_ref().take(4).collect();
                            out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                        }
                        _ => return None,
                    },
                    c if c.is_control() && c != '\t' => return None,
                    c => out.push(c),
                }
            }
            None
        }
        _ => None,
    }
}

fn after_value(rest: &str) -> bool {
    let rest = rest.trim_start();
    rest.is_empty() || rest.starts_with('#')
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

    #[test]
    fn handles_comments_quotes_and_escapes() {
        let text = r#"
# header comment
[[mode]]   # trailing comment
id = "a-1"          # why
label = "Say \"hi\" \u00e9\\ # not a comment"
hint = 'literal \n stays'
extra = "ignored"
"#;
        let t = Taxonomy::parse(text).unwrap();
        assert_eq!(t.modes[0].id, "a-1");
        assert_eq!(t.modes[0].label, "Say \"hi\" é\\ # not a comment");
        assert_eq!(t.modes[0].hint, "literal \\n stays");
    }

    #[test]
    fn reports_line_numbers() {
        let e = Taxonomy::parse("[[mode]]\nid = \"a\"\nlabel = x\n").unwrap_err();
        assert_eq!(
            e,
            TaxonomyError::Parse("line 3: values must be quoted strings".into())
        );
        assert!(
            matches!(Taxonomy::parse("[other]\n"), Err(TaxonomyError::Parse(m)) if m.starts_with("line 1"))
        );
        assert!(
            matches!(Taxonomy::parse("id = \"a\"\n"), Err(TaxonomyError::Parse(m)) if m.contains("outside"))
        );
        assert!(
            matches!(Taxonomy::parse("[[mode]]\nid = \"a\"\n"), Err(TaxonomyError::Parse(m)) if m.contains("needs both"))
        );
        assert!(
            Taxonomy::parse("[[mode]]\nid = \"a\nlabel = \"b\"").is_err(),
            "unterminated string"
        );
    }

    /// The small parser must agree with a real TOML parser on the files it accepts.
    #[test]
    fn agrees_with_the_toml_crate() {
        let samples = [
            REPO_FILE.to_owned(),
            "[[mode]]\nid = \"x\"\nlabel = 'Tab\there'\nhint = \"Line\\nbreak \\u00fc\"\n".to_owned(),
            "# c\n[[mode]] # c\nlabel = \"L\" # c\nid = \"y\"\n\n[[mode]]\nid = \"z\"\nlabel = \"Z\"\n".to_owned(),
        ];
        for text in samples {
            let ours = Taxonomy::parse(&text).unwrap();
            let theirs: Taxonomy = toml::from_str(&text).unwrap();
            assert_eq!(ours, theirs, "{text}");
        }
    }
}

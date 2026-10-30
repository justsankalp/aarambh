//! Machine judgments in `data/judgments/<note-id>.json`, written only by the
//! judge job.

use serde::{Deserialize, Serialize};

use crate::event::Rel;
use crate::id::Id;
use crate::time::Timestamp;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Judgment {
    pub note_id: Id,
    pub model: String,
    pub ts: Timestamp,
    #[serde(default)]
    pub edges: Vec<JudgedEdge>,
    #[serde(default)]
    pub risks: Vec<Risk>,
}

/// A suggested link from the judged note to another note.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct JudgedEdge {
    pub id: Id,
    pub to: Id,
    pub rel: EdgeRel,
    pub strength: Strength,
    pub why: String,
}

/// A human link relation, or `duplicates`, which only the judge suggests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeRel {
    Rel(Rel),
    Duplicates,
}

impl Serialize for EdgeRel {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            EdgeRel::Rel(r) => r.serialize(s),
            EdgeRel::Duplicates => s.serialize_str("duplicates"),
        }
    }
}

impl<'de> Deserialize<'de> for EdgeRel {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == "duplicates" {
            return Ok(EdgeRel::Duplicates);
        }
        Rel::deserialize(serde::de::value::StrDeserializer::<D::Error>::new(&s)).map(EdgeRel::Rel)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Strength {
    Clear,
    Possible,
}

/// The judged note resembles a past lesson's failure mode.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Risk {
    pub id: Id,
    /// The status event whose lesson this matches.
    pub lesson_event: Id,
    /// Failure-mode id from `data/taxonomy.toml`.
    pub mode: String,
    pub level: RiskLevel,
    pub why: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Clearly,
    Possibly,
}

impl Judgment {
    pub fn path(note_id: Id) -> String {
        format!("data/judgments/{note_id}.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(n: u64) -> Id {
        Id::new(1_761_843_481_000 + n, u128::from(n))
    }

    #[test]
    fn judgment_matches_documented_shape() {
        let j = Judgment {
            note_id: id(1),
            model: "claude-haiku-5-5".into(),
            ts: Timestamp(1_761_843_481_000),
            edges: vec![
                JudgedEdge {
                    id: id(10),
                    to: id(0),
                    rel: EdgeRel::Rel(Rel::Supports),
                    strength: Strength::Clear,
                    why: "Same clinic data".into(),
                },
                JudgedEdge {
                    id: id(11),
                    to: id(2),
                    rel: EdgeRel::Duplicates,
                    strength: Strength::Possible,
                    why: "Restates note 2".into(),
                },
            ],
            risks: vec![Risk {
                id: id(12),
                lesson_event: id(3),
                mode: "no-paying-customer".into(),
                level: RiskLevel::Possibly,
                why: "No buyer named".into(),
            }],
        };
        let v = serde_json::to_value(&j).unwrap();
        assert_eq!(v["edges"][0]["rel"], "supports");
        assert_eq!(v["edges"][1]["rel"], "duplicates");
        assert_eq!(v["edges"][1]["strength"], "possible");
        assert_eq!(v["risks"][0]["level"], "possibly");
        assert_eq!(v["ts"], "2025-10-30T16:58:01.000Z");
        assert_eq!(serde_json::from_value::<Judgment>(v).unwrap(), j);
        assert_eq!(
            Judgment::path(id(1)),
            format!("data/judgments/{}.json", id(1))
        );
    }

    #[test]
    fn empty_judgment_parses_and_bad_rel_fails() {
        let j: Judgment = serde_json::from_value(json!({
            "note_id": id(1).to_string(), "model": "m", "ts": "2025-10-30T16:58:01Z"
        }))
        .unwrap();
        assert!(j.edges.is_empty() && j.risks.is_empty());
        assert!(serde_json::from_value::<EdgeRel>(json!("loves")).is_err());
        assert_eq!(
            serde_json::from_value::<EdgeRel>(json!("answers")).unwrap(),
            EdgeRel::Rel(Rel::Answers)
        );
    }
}

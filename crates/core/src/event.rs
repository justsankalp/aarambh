//! Append-only events stored in `data/events/YYYY-MM-DD.json` (schema `v: 1`).

use serde::{Deserialize, Serialize};

use crate::id::Id;
use crate::time::Timestamp;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub v: u32,
    pub id: Id,
    pub ts: Timestamp,
    pub author: String,
    #[serde(flatten)]
    pub body: EventBody,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EventBody {
    Note(Note),
    Status(StatusChange),
    Verdict(Verdict),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub kind: NoteKind,
    pub text: String,
    #[serde(default)]
    pub links: Vec<Link>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum NoteKind {
    Idea,
    Belief,
    Question,
    Evidence,
    Disagreement,
    Note,
}

impl NoteKind {
    pub const ALL: [NoteKind; 6] = [
        NoteKind::Idea,
        NoteKind::Belief,
        NoteKind::Question,
        NoteKind::Evidence,
        NoteKind::Disagreement,
        NoteKind::Note,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            NoteKind::Idea => "idea",
            NoteKind::Belief => "belief",
            NoteKind::Question => "question",
            NoteKind::Evidence => "evidence",
            NoteKind::Disagreement => "disagreement",
            NoteKind::Note => "note",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub to: Id,
    pub rel: Rel,
}

/// How a note relates to the note it links to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Rel {
    Extends,
    Supports,
    Challenges,
    Answers,
    Related,
}

impl Rel {
    pub const ALL: [Rel; 5] = [
        Rel::Extends,
        Rel::Supports,
        Rel::Challenges,
        Rel::Answers,
        Rel::Related,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Rel::Extends => "extends",
            Rel::Supports => "supports",
            Rel::Challenges => "challenges",
            Rel::Answers => "answers",
            Rel::Related => "related",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StatusChange {
    pub idea: Id,
    pub status: IdeaStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lesson: Option<Lesson>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum IdeaStatus {
    Active,
    Parked,
    Dropped,
    Decided,
}

impl IdeaStatus {
    /// Dropped and decided ideas prompt the (skippable) post-mortem.
    pub fn prompts_lesson(self) -> bool {
        matches!(self, IdeaStatus::Dropped | IdeaStatus::Decided)
    }
}

/// The three-field post-mortem. Every field may be left empty.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Lesson {
    /// Failure-mode ids from `data/taxonomy.toml`.
    #[serde(default)]
    pub modes: Vec<String>,
    #[serde(default)]
    pub why: String,
    #[serde(default)]
    pub would_change_mind: String,
    #[serde(default)]
    pub check_earlier: String,
}

/// A person's answer to a machine-suggested edge or risk flag.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub target: Id,
    pub decision: Decision,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Confirm,
    Reject,
}

impl Event {
    pub fn new(id: Id, author: impl Into<String>, body: EventBody) -> Self {
        Event {
            v: SCHEMA_VERSION,
            id,
            ts: Timestamp(id.timestamp_ms()),
            author: author.into(),
            body,
        }
    }

    pub fn as_note(&self) -> Option<&Note> {
        match &self.body {
            EventBody::Note(n) => Some(n),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn id(n: u64) -> Id {
        Id::new(1_761_843_481_000 + n, u128::from(n))
    }

    fn round_trip(e: &Event) -> Value {
        let v = serde_json::to_value(e).unwrap();
        let back: Event = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(&back, e);
        v
    }

    #[test]
    fn note_matches_documented_shape() {
        let e = Event::new(
            id(1),
            "Sankalp",
            EventBody::Note(Note {
                kind: NoteKind::Belief,
                text: "Clinics will pay for this".into(),
                links: vec![Link {
                    to: id(0),
                    rel: Rel::Challenges,
                }],
            }),
        );
        let v = round_trip(&e);
        assert_eq!(
            v,
            json!({
                "v": 1, "id": id(1).to_string(), "ts": "2025-10-30T16:58:01.001Z",
                "author": "Sankalp", "type": "note", "kind": "belief",
                "text": "Clinics will pay for this",
                "links": [{"to": id(0).to_string(), "rel": "challenges"}]
            })
        );
    }

    #[test]
    fn status_with_and_without_lesson() {
        let plain = Event::new(
            id(2),
            "A",
            EventBody::Status(StatusChange {
                idea: id(1),
                status: IdeaStatus::Parked,
                lesson: None,
            }),
        );
        let v = round_trip(&plain);
        assert_eq!(v["type"], "status");
        assert_eq!(v["status"], "parked");
        assert!(
            v.get("lesson").is_none(),
            "absent lesson is omitted, not null"
        );

        let dropped = Event::new(
            id(3),
            "A",
            EventBody::Status(StatusChange {
                idea: id(1),
                status: IdeaStatus::Dropped,
                lesson: Some(Lesson {
                    modes: vec!["no-paying-customer".into(), "slow-to-validate".into()],
                    why: "Nobody would pay".into(),
                    would_change_mind: "A signed LOI".into(),
                    check_earlier: "Ask for money in week one".into(),
                }),
            }),
        );
        let v = round_trip(&dropped);
        assert_eq!(v["lesson"]["modes"][1], "slow-to-validate");
        assert_eq!(v["lesson"]["would_change_mind"], "A signed LOI");
    }

    #[test]
    fn skipped_post_mortem_fields_default_to_empty() {
        let raw = json!({
            "v": 1, "id": id(4).to_string(), "ts": "2025-10-30T16:58:01.004Z", "author": "B",
            "type": "status", "idea": id(1).to_string(), "status": "decided", "lesson": {}
        });
        let e: Event = serde_json::from_value(raw).unwrap();
        let EventBody::Status(s) = e.body else {
            panic!("expected status")
        };
        assert_eq!(s.lesson, Some(Lesson::default()));
    }

    #[test]
    fn verdict_round_trips() {
        let e = Event::new(
            id(5),
            "B",
            EventBody::Verdict(Verdict {
                target: id(9),
                decision: Decision::Reject,
            }),
        );
        let v = round_trip(&e);
        assert_eq!(v["type"], "verdict");
        assert_eq!(v["decision"], "reject");
    }

    #[test]
    fn note_links_default_to_empty_and_unknown_fields_are_ignored() {
        let raw = json!({
            "v": 1, "id": id(6).to_string(), "ts": "2025-10-30T16:58:01.006Z", "author": "A",
            "type": "note", "kind": "question", "text": "Why now?", "mood": "curious"
        });
        let e: Event = serde_json::from_value(raw).unwrap();
        assert_eq!(e.as_note().unwrap().links, vec![]);
    }

    #[test]
    fn unknown_kinds_types_and_rels_are_errors() {
        let base = json!({"v": 1, "id": id(7).to_string(), "ts": "2025-10-30T16:58:01.007Z", "author": "A"});
        let with = |extra: Value| {
            let mut v = base.clone();
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value::<Event>(v)
        };
        assert!(with(json!({"type": "note", "kind": "rumour", "text": "x"})).is_err());
        assert!(with(json!({"type": "reaction", "emoji": "x"})).is_err());
        assert!(with(json!({"type": "note", "kind": "note", "text": "x", "links": [{"to": id(1).to_string(), "rel": "hates"}]})).is_err());
    }

    #[test]
    fn user_text_is_stored_verbatim() {
        let text = "<img src=x onerror=alert(1)> & \"quotes\" — ünïcode 🌱\nnew line";
        let e = Event::new(
            id(8),
            "<b>me</b>",
            EventBody::Note(Note {
                kind: NoteKind::Note,
                text: text.into(),
                links: vec![],
            }),
        );
        round_trip(&e);
        let s = serde_json::to_string(&e).unwrap();
        let back: Event = serde_json::from_str(&s).unwrap();
        assert_eq!(back.as_note().unwrap().text, text);
        assert_eq!(back.author, "<b>me</b>");
    }
}

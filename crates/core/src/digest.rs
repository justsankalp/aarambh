//! The digest's running state, `data/digests/state.json`, written only by
//! the digest job and read by the app ("N new notes since the summary").

use serde::{Deserialize, Serialize};

use crate::event::{Event, EventBody, IdeaStatus, NoteKind};
use crate::feed::excerpt;
use crate::id::Id;
use crate::log::Log;
use crate::time::Timestamp;

pub const STATE_PATH: &str = "data/digests/state.json";
pub const DIGESTS_DIR: &str = "data/digests";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DigestState {
    pub v: u32,
    /// The newest event covered so far. Events after this are "new".
    pub through: Option<Timestamp>,
    pub generated_at: Option<Timestamp>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub ideas: Vec<IdeaState>,
    #[serde(default)]
    pub disagreements: Vec<Disagreement>,
    #[serde(default)]
    pub assumptions: Vec<Assumption>,
    #[serde(default)]
    pub open_questions: Vec<OpenQuestion>,
    #[serde(default)]
    pub patterns: Vec<Pattern>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct IdeaState {
    pub id: Id,
    pub title: String,
    pub status: IdeaStatus,
    #[serde(default)]
    pub timeline: Vec<TimelineEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TimelineEntry {
    /// UTC day, `YYYY-MM-DD`.
    pub date: String,
    pub what: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Disagreement {
    pub topic: String,
    pub positions: Vec<Position>,
    /// The assumption the two positions rest on.
    pub assumption: String,
    #[serde(default)]
    pub note_ids: Vec<Id>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Position {
    pub who: String,
    pub says: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AssumptionStatus {
    Untested,
    Supported,
    Challenged,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Assumption {
    pub text: String,
    pub status: AssumptionStatus,
    #[serde(default)]
    pub note_ids: Vec<Id>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OpenQuestion {
    pub text: String,
    pub note_id: Option<Id>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pattern {
    pub text: String,
    pub note_ids: Vec<Id>,
}

impl DigestState {
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// Events after the last digest, oldest first. Verdicts count: they
    /// change the graph even when no one wrote anything.
    pub fn new_events<'a>(&self, log: &'a Log) -> Vec<&'a Event> {
        log.iter()
            .filter(|e| self.through.is_none_or(|t| e.ts > t))
            .collect()
    }

    /// Notes written since the last digest.
    pub fn new_note_count(&self, log: &Log) -> usize {
        log.iter()
            .filter(|e| {
                matches!(e.body, EventBody::Note(_)) && self.through.is_none_or(|t| e.ts > t)
            })
            .count()
    }
}

/// Idea statuses and timelines, built only from events.
pub fn ideas_state(log: &Log) -> Vec<IdeaState> {
    log.iter()
        .filter(|e| e.as_note().is_some_and(|n| n.kind == NoteKind::Idea))
        .map(|idea| {
            let mut timeline = vec![TimelineEntry {
                date: idea.ts.day().to_string(),
                what: format!("Proposed by {}", idea.author),
            }];
            for e in log.iter() {
                if let EventBody::Status(s) = &e.body
                    && s.idea == idea.id
                {
                    let st = format!("{:?}", s.status).to_lowercase();
                    timeline.push(TimelineEntry {
                        date: e.ts.day().to_string(),
                        what: format!("Marked {st} by {}", e.author),
                    });
                }
            }
            IdeaState {
                id: idea.id,
                title: excerpt(
                    &idea.as_note().map_or(String::new(), |n| n.text.clone()),
                    80,
                ),
                status: log.status_of(idea.id),
                timeline,
            }
        })
        .collect()
}

/// `data/digests/2026-10-30.md` → its date. Other files are ignored.
pub fn digest_date(file_name: &str) -> Option<&str> {
    let d = file_name.strip_suffix(".md")?;
    d.parse::<crate::time::Day>().ok().map(|_| d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Decision, Note, NoteKind, Verdict};

    fn ev(n: u64, body: EventBody) -> Event {
        Event::new(
            Id::new(1_761_843_481_000 + n * 1000, u128::from(n)),
            "A",
            body,
        )
    }

    fn note(n: u64) -> Event {
        ev(
            n,
            EventBody::Note(Note {
                kind: NoteKind::Note,
                text: format!("{n}"),
                links: vec![],
            }),
        )
    }

    #[test]
    fn new_events_and_counts_since_the_last_digest() {
        let mut log = Log::new();
        let (a, b, c) = (note(1), note(2), note(3));
        let v = ev(
            4,
            EventBody::Verdict(Verdict {
                target: a.id,
                decision: Decision::Confirm,
            }),
        );
        log.extend([a.clone(), b.clone(), c.clone(), v.clone()]);

        let never = DigestState::default();
        assert_eq!(never.new_events(&log).len(), 4);
        assert_eq!(never.new_note_count(&log), 3);

        let after_b = DigestState {
            through: Some(b.ts),
            ..Default::default()
        };
        let ids: Vec<Id> = after_b.new_events(&log).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![c.id, v.id]);
        assert_eq!(after_b.new_note_count(&log), 1);
    }

    #[test]
    fn state_round_trips_and_tolerates_missing_fields() {
        let s = DigestState {
            v: 1,
            through: Some(Timestamp(1_761_843_481_000)),
            generated_at: Some(Timestamp(1_761_843_500_000)),
            model: "gemini-3.8-flash".into(),
            ideas: vec![IdeaState {
                id: Id::new(1, 1),
                title: "Clinic intake".into(),
                status: IdeaStatus::Active,
                timeline: vec![TimelineEntry {
                    date: "2025-10-30".into(),
                    what: "Proposed by Asha".into(),
                }],
            }],
            disagreements: vec![Disagreement {
                topic: "Who buys".into(),
                positions: vec![Position {
                    who: "Asha".into(),
                    says: "Owners".into(),
                }],
                assumption: "Owners control budgets".into(),
                note_ids: vec![],
            }],
            assumptions: vec![Assumption {
                text: "x".into(),
                status: AssumptionStatus::Untested,
                note_ids: vec![],
            }],
            open_questions: vec![OpenQuestion {
                text: "q".into(),
                note_id: None,
            }],
            patterns: vec![],
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(DigestState::parse(&json).unwrap(), s);
        let minimal = DigestState::parse(r#"{"v":1,"through":null,"generated_at":null}"#).unwrap();
        assert!(minimal.ideas.is_empty() && minimal.model.is_empty());
    }

    #[test]
    fn idea_timelines_come_from_events() {
        use crate::event::StatusChange;
        let idea = ev(
            1,
            EventBody::Note(Note {
                kind: NoteKind::Idea,
                text: "Sell to clinics\nmore".into(),
                links: vec![],
            }),
        );
        let other = note(2);
        let parked = ev(
            3,
            EventBody::Status(StatusChange {
                idea: idea.id,
                status: IdeaStatus::Parked,
                lesson: None,
            }),
        );
        let mut log = Log::new();
        log.extend([idea.clone(), other, parked]);
        let ideas = ideas_state(&log);
        assert_eq!(ideas.len(), 1);
        assert_eq!(ideas[0].title, "Sell to clinics…");
        assert_eq!(ideas[0].status, IdeaStatus::Parked);
        let what: Vec<&str> = ideas[0].timeline.iter().map(|t| t.what.as_str()).collect();
        assert_eq!(what, vec!["Proposed by A", "Marked parked by A"]);
    }

    #[test]
    fn reads_digest_file_names() {
        assert_eq!(digest_date("2026-10-30.md"), Some("2026-10-30"));
        assert_eq!(digest_date("state.json"), None);
        assert_eq!(digest_date("2026-13-40.md"), None);
    }
}

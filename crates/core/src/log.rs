//! Merging events from day files, the local cache and the offline queue, and
//! appending new events to a day file without touching existing ones.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::event::{Decision, Event, EventBody, IdeaStatus, SCHEMA_VERSION, StatusChange};
use crate::id::Id;
use crate::time::Timestamp;

/// All known events, ordered by `(ts, id)`. Events are immutable, so merging
/// is a set union keyed by id; the result does not depend on arrival order.
#[derive(Clone, Debug, Default)]
pub struct Log {
    by_order: BTreeMap<(Timestamp, Id), Event>,
    order_of: HashMap<Id, Timestamp>,
}

impl Log {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.by_order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_order.is_empty()
    }

    /// Adds an event. Returns `true` if the log changed.
    ///
    /// Two different events with the same id should never happen (80 random
    /// bits), but if they do, the one whose JSON sorts first wins so that every
    /// device converges on the same log.
    pub fn insert(&mut self, event: Event) -> bool {
        if let Some(&ts) = self.order_of.get(&event.id) {
            let existing = &self.by_order[&(ts, event.id)];
            if *existing == event || canonical(existing) <= canonical(&event) {
                return false;
            }
            self.by_order.remove(&(ts, event.id));
        }
        self.order_of.insert(event.id, event.ts);
        self.by_order.insert((event.ts, event.id), event);
        true
    }

    /// Merges events in. Returns how many changed the log.
    pub fn extend(&mut self, events: impl IntoIterator<Item = Event>) -> usize {
        events
            .into_iter()
            .map(|e| self.insert(e))
            .filter(|&changed| changed)
            .count()
    }

    pub fn get(&self, id: Id) -> Option<&Event> {
        self.order_of.get(&id).map(|&ts| &self.by_order[&(ts, id)])
    }

    pub fn contains(&self, id: Id) -> bool {
        self.order_of.contains_key(&id)
    }

    /// Oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Event> {
        self.by_order.values()
    }

    pub fn newest_first(&self) -> impl Iterator<Item = &Event> {
        self.by_order.values().rev()
    }

    /// The latest status change for an idea, if any.
    pub fn current_status(&self, idea: Id) -> Option<&StatusChange> {
        self.newest_first().find_map(|e| match &e.body {
            EventBody::Status(s) if s.idea == idea => Some(s),
            _ => None,
        })
    }

    /// Status of an idea, defaulting to active.
    pub fn status_of(&self, idea: Id) -> IdeaStatus {
        self.current_status(idea)
            .map_or(IdeaStatus::Active, |s| s.status)
    }

    /// The latest verdict on a machine suggestion, if any.
    pub fn current_verdict(&self, target: Id) -> Option<Decision> {
        self.newest_first().find_map(|e| match &e.body {
            EventBody::Verdict(v) if v.target == target => Some(v.decision),
            _ => None,
        })
    }
}

fn canonical(e: &Event) -> String {
    serde_json::to_string(e).unwrap_or_default()
}

/// Events read from one day file.
#[derive(Debug, Default)]
pub struct ParsedDay {
    pub events: Vec<Event>,
    /// Entries this version can't read (newer schema, unknown type, damage).
    /// They are kept in the file untouched.
    pub skipped: usize,
}

/// Parses a day file. Fails only if the file isn't a JSON array.
pub fn parse_day_file(text: &str) -> Result<ParsedDay, serde_json::Error> {
    let raw: Vec<Box<RawValue>> = serde_json::from_str(text)?;
    let mut out = ParsedDay::default();
    for r in raw {
        match serde_json::from_str::<Event>(r.get()) {
            Ok(e) if e.v == SCHEMA_VERSION => out.events.push(e),
            _ => out.skipped += 1,
        }
    }
    Ok(out)
}

/// The result of appending to a day file.
#[derive(Debug)]
pub struct Appended {
    pub text: String,
    /// Events added. Events whose id is already in the file are left out, so
    /// retrying a write whose response was lost doesn't duplicate anything.
    pub added: usize,
}

/// Appends events to a day file's text (`None` if the file doesn't exist
/// yet). Existing entries are copied byte for byte, even ones this version
/// can't read. Fails if the existing file isn't a JSON array.
pub fn append_to_day_file(
    existing: Option<&str>,
    new: &[Event],
) -> Result<Appended, serde_json::Error> {
    #[derive(Deserialize)]
    struct IdOnly<'a> {
        #[serde(borrow)]
        id: Option<&'a str>,
    }

    let raw: Vec<Box<RawValue>> = match existing {
        Some(t) if !t.trim().is_empty() => serde_json::from_str(t)?,
        _ => Vec::new(),
    };
    let present: std::collections::HashSet<String> = raw
        .iter()
        .filter_map(|r| {
            serde_json::from_str::<IdOnly>(r.get())
                .ok()?
                .id
                .map(str::to_owned)
        })
        .collect();

    let mut lines: Vec<String> = raw.iter().map(|r| r.get().to_owned()).collect();
    let mut added = 0;
    let mut seen = present;
    for e in new {
        if seen.insert(e.id.to_string()) {
            lines.push(serde_json::to_string(e)?);
            added += 1;
        }
    }
    Ok(Appended {
        text: render_lines(&lines),
        added,
    })
}

/// One event per line: readable in a browser and diff-friendly in git.
fn render_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return "[]\n".to_owned();
    }
    format!("[\n  {}\n]\n", lines.join(",\n  "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Lesson, Link, Note, NoteKind, Rel, Verdict};

    const T0: u64 = 1_761_843_481_000;

    fn note(n: u64, author: &str, text: &str) -> Event {
        Event::new(
            Id::new(T0 + n * 1000, u128::from(n)),
            author,
            EventBody::Note(Note {
                kind: NoteKind::Idea,
                text: text.into(),
                links: vec![],
            }),
        )
    }

    fn status(n: u64, idea: &Event, s: IdeaStatus) -> Event {
        Event::new(
            Id::new(T0 + n * 1000, u128::from(n)),
            "A",
            EventBody::Status(StatusChange {
                idea: idea.id,
                status: s,
                lesson: None,
            }),
        )
    }

    fn ids(log: &Log) -> Vec<Id> {
        log.iter().map(|e| e.id).collect()
    }

    #[test]
    fn merge_is_order_independent_and_idempotent() {
        let evs: Vec<Event> = (0..20)
            .map(|n| note(n, if n % 2 == 0 { "A" } else { "B" }, "x"))
            .collect();

        let mut forward = Log::new();
        assert_eq!(forward.extend(evs.clone()), 20);

        let mut shuffled = Log::new();
        // Deterministic shuffle plus duplicates, as if from cache + remote + queue.
        let order = [
            7, 3, 19, 0, 11, 3, 15, 1, 8, 2, 7, 4, 13, 5, 18, 6, 9, 10, 12, 14, 16, 17, 0,
        ];
        for i in order {
            shuffled.insert(evs[i].clone());
        }
        assert_eq!(ids(&forward), ids(&shuffled));
        assert_eq!(forward.extend(evs.clone()), 0, "re-merging changes nothing");
        assert_eq!(forward.len(), 20);
    }

    #[test]
    fn orders_by_timestamp_then_id() {
        let mut log = Log::new();
        let late = note(5, "A", "late");
        let early = note(1, "B", "early");
        // Same ts, different random part: id breaks the tie.
        let tie_a = Event {
            id: Id::new(T0 + 3000, 1),
            ..note(3, "A", "tie a")
        };
        let tie_b = Event {
            id: Id::new(T0 + 3000, 2),
            ..note(3, "B", "tie b")
        };
        log.extend([late.clone(), tie_b.clone(), early.clone(), tie_a.clone()]);
        assert_eq!(ids(&log), vec![early.id, tie_a.id, tie_b.id, late.id]);
        let newest: Vec<Id> = log.newest_first().map(|e| e.id).collect();
        assert_eq!(newest, vec![late.id, tie_b.id, tie_a.id, early.id]);
    }

    #[test]
    fn id_collisions_resolve_the_same_way_on_every_device() {
        let a = note(1, "A", "first");
        let b = Event {
            author: "B".into(),
            ..a.clone()
        };
        let mut one = Log::new();
        one.extend([a.clone(), b.clone()]);
        let mut two = Log::new();
        two.extend([b.clone(), a.clone()]);
        assert_eq!(one.len(), 1);
        assert_eq!(one.get(a.id), two.get(a.id));
    }

    #[test]
    fn latest_status_and_verdict_win() {
        let idea = note(1, "A", "idea");
        let mut log = Log::new();
        assert_eq!(log.status_of(idea.id), IdeaStatus::Active);
        let parked = status(2, &idea, IdeaStatus::Parked);
        let mut dropped = status(3, &idea, IdeaStatus::Dropped);
        if let EventBody::Status(s) = &mut dropped.body {
            s.lesson = Some(Lesson {
                why: "no buyer".into(),
                ..Default::default()
            });
        }
        log.extend([dropped.clone(), idea.clone(), parked]);
        assert_eq!(log.status_of(idea.id), IdeaStatus::Dropped);
        assert_eq!(
            log.current_status(idea.id)
                .unwrap()
                .lesson
                .as_ref()
                .unwrap()
                .why,
            "no buyer"
        );

        let target = Id::new(T0, 99);
        let v = |n: u64, d| {
            Event::new(
                Id::new(T0 + n * 1000, 0),
                "B",
                EventBody::Verdict(Verdict {
                    target,
                    decision: d,
                }),
            )
        };
        log.extend([v(5, Decision::Confirm), v(4, Decision::Reject)]);
        assert_eq!(log.current_verdict(target), Some(Decision::Confirm));
        assert_eq!(log.current_verdict(Id::new(T0, 7)), None);
    }

    #[test]
    fn parse_skips_unreadable_entries_but_keeps_the_rest() {
        let good = note(1, "A", "ok");
        let text = format!(
            "[{}, {{\"v\":2,\"type\":\"future\"}}, {{\"garbage\":true}}, 42]",
            serde_json::to_string(&good).unwrap()
        );
        let parsed = parse_day_file(&text).unwrap();
        assert_eq!(parsed.events, vec![good]);
        assert_eq!(parsed.skipped, 3);
        assert!(parse_day_file("{\"not\":\"an array\"}").is_err());
    }

    #[test]
    fn append_creates_a_new_file() {
        let a = note(1, "A", "hello");
        let out = append_to_day_file(None, std::slice::from_ref(&a)).unwrap();
        assert_eq!(out.added, 1);
        assert_eq!(
            out.text,
            format!("[\n  {}\n]\n", serde_json::to_string(&a).unwrap())
        );
        assert_eq!(parse_day_file(&out.text).unwrap().events, vec![a]);
    }

    #[test]
    fn append_never_rewrites_existing_entries() {
        // Hand-formatted entry, odd key order, and one from a future schema.
        let existing_note = note(1, "A", "first");
        let hand = format!(
            "[\n    {{ \"author\": \"A\", \"type\": \"note\", \"v\": 1, \"id\": \"{}\", \"ts\": \"{}\",\n      \"kind\": \"idea\", \"text\": \"first\" }},\n {{\"v\":2,\"id\":\"FUTURE\",\"type\":\"poll\"}}\n]",
            existing_note.id, existing_note.ts
        );
        let new = note(2, "B", "second");
        let out = append_to_day_file(Some(&hand), std::slice::from_ref(&new)).unwrap();
        assert_eq!(out.added, 1);
        assert!(
            out.text
                .contains("{ \"author\": \"A\", \"type\": \"note\", \"v\": 1,"),
            "kept verbatim"
        );
        assert!(
            out.text
                .contains("{\"v\":2,\"id\":\"FUTURE\",\"type\":\"poll\"}"),
            "future entry kept"
        );
        let parsed = parse_day_file(&out.text).unwrap();
        assert_eq!(parsed.events, vec![existing_note, new]);
        assert_eq!(parsed.skipped, 1);
    }

    #[test]
    fn append_is_safe_to_retry() {
        let a = note(1, "A", "one");
        let b = note(2, "B", "two");
        let first = append_to_day_file(None, std::slice::from_ref(&a)).unwrap();
        let retry =
            append_to_day_file(Some(&first.text), &[a.clone(), b.clone(), b.clone()]).unwrap();
        assert_eq!(retry.added, 1);
        assert_eq!(parse_day_file(&retry.text).unwrap().events, vec![a, b]);
    }

    #[test]
    fn append_refuses_a_damaged_file() {
        assert!(append_to_day_file(Some("[{\"v\":1,"), &[note(1, "A", "x")]).is_err());
        assert_eq!(append_to_day_file(Some("  "), &[]).unwrap().text, "[]\n");
    }

    #[test]
    fn links_survive_the_round_trip() {
        let target = note(1, "A", "idea");
        let reply = Event::new(
            Id::new(T0 + 9000, 9),
            "B",
            EventBody::Note(Note {
                kind: NoteKind::Question,
                text: "how?".into(),
                links: vec![Link {
                    to: target.id,
                    rel: Rel::Answers,
                }],
            }),
        );
        let out = append_to_day_file(None, &[target, reply.clone()]).unwrap();
        let back = parse_day_file(&out.text).unwrap();
        assert_eq!(back.events[1], reply);
    }
}

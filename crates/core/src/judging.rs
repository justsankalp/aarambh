//! Choosing what the judge looks at: which notes still need a judgment,
//! the few earlier notes most related to each, and past lessons.

use std::collections::HashSet;

use crate::event::{Event, EventBody, IdeaStatus, Lesson};
use crate::id::Id;
use crate::log::Log;
use crate::search::Index;

/// Related notes sent with each new note.
pub const SHORTLIST: usize = 8;
/// If search finds fewer than this many related notes, the most recent
/// earlier notes fill the gap (related notes don't always share words).
pub const MIN_CANDIDATES: usize = 3;
/// Most recent lessons sent with each note.
pub const MAX_LESSONS: usize = 30;
/// Notes judged per run, oldest first; the rest wait for the next run.
/// Keeps a first run over an existing log within free-tier limits.
pub const MAX_PER_RUN: usize = 20;

/// Notes without a judgment yet, oldest first.
pub fn unjudged<'a>(log: &'a Log, judged: &HashSet<Id>) -> Vec<&'a Event> {
    log.iter()
        .filter(|e| e.as_note().is_some() && !judged.contains(&e.id))
        .collect()
}

/// Earlier notes most related to `note`, best first.
pub fn shortlist<'a>(log: &'a Log, note: &Event) -> Vec<&'a Event> {
    let Some(n) = note.as_note() else {
        return Vec::new();
    };
    let earlier: Vec<&Event> = log
        .iter()
        .filter(|e| e.id != note.id && (e.ts, e.id) < (note.ts, note.id))
        .filter(|e| e.as_note().is_some())
        .collect();
    let index = Index::new(
        earlier
            .iter()
            .map(|e| (e.id, e.as_note().map_or("", |n| n.text.as_str()))),
    );
    let mut picked: Vec<&Event> = index
        .search(&n.text, SHORTLIST)
        .into_iter()
        .filter_map(|(id, _)| log.get(id))
        .collect();
    for e in earlier.iter().rev() {
        if picked.len() >= MIN_CANDIDATES.min(SHORTLIST) {
            break;
        }
        if !picked.iter().any(|p| p.id == e.id) {
            picked.push(e);
        }
    }
    picked
}

/// A post-mortem from a dropped or decided idea.
pub struct LessonRef<'a> {
    pub event: &'a Event,
    pub status: IdeaStatus,
    pub lesson: &'a Lesson,
    /// The idea's text, if it's loaded.
    pub idea_text: Option<&'a str>,
}

/// Non-empty lessons recorded before `before`, newest first.
pub fn lessons_before<'a>(log: &'a Log, before: &Event) -> Vec<LessonRef<'a>> {
    log.newest_first()
        .filter(|e| (e.ts, e.id) < (before.ts, before.id))
        .filter_map(|e| match &e.body {
            EventBody::Status(s) => {
                let lesson = s.lesson.as_ref().filter(|l| **l != Lesson::default())?;
                let idea_text = log
                    .get(s.idea)
                    .and_then(|i| i.as_note())
                    .map(|n| n.text.as_str());
                Some(LessonRef {
                    event: e,
                    status: s.status,
                    lesson,
                    idea_text,
                })
            }
            _ => None,
        })
        .take(MAX_LESSONS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Note, NoteKind, StatusChange};

    fn ev(n: u64, body: EventBody) -> Event {
        Event::new(
            Id::new(1_761_843_481_000 + n * 1000, u128::from(n)),
            "A",
            body,
        )
    }

    fn note(n: u64, text: &str) -> Event {
        ev(
            n,
            EventBody::Note(Note {
                kind: NoteKind::Idea,
                text: text.into(),
                links: vec![],
            }),
        )
    }

    fn lesson(n: u64, idea: &Event, why: &str) -> Event {
        let lesson = (!why.is_empty()).then(|| Lesson {
            why: why.into(),
            ..Default::default()
        });
        ev(
            n,
            EventBody::Status(StatusChange {
                idea: idea.id,
                status: IdeaStatus::Dropped,
                lesson,
            }),
        )
    }

    #[test]
    fn unjudged_notes_oldest_first() {
        let (a, b, c) = (note(1, "a"), note(2, "b"), note(3, "c"));
        let mut log = Log::new();
        log.extend([c.clone(), a.clone(), b.clone(), lesson(4, &a, "x")]);
        let judged = HashSet::from([b.id]);
        let ids: Vec<Id> = unjudged(&log, &judged).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![a.id, c.id], "statuses are never judged");
    }

    #[test]
    fn shortlist_uses_only_earlier_notes_and_ranks_by_relevance() {
        let mut log = Log::new();
        let notes = [
            note(1, "Clinics hate their intake software"),
            note(2, "Hire a designer"),
            note(3, "Clinic intake takes an hour"),
            note(4, "Schools budget in April"),
            note(5, "Clinic owners decide on software"),
        ];
        log.extend(notes.clone());
        let later = note(9, "Intake software for clinics");
        log.insert(later.clone());
        let after = note(10, "clinic intake software again");
        log.insert(after.clone());

        let picked: Vec<Id> = shortlist(&log, &later).iter().map(|e| e.id).collect();
        assert_eq!(picked[0], notes[0].id, "best match first");
        assert!(picked.contains(&notes[2].id) && picked.contains(&notes[4].id));
        assert!(
            !picked.contains(&later.id) && !picked.contains(&after.id),
            "never itself or later notes"
        );
        assert!(
            !picked.contains(&notes[1].id),
            "unrelated notes stay out once there are enough"
        );
    }

    #[test]
    fn shortlist_falls_back_to_recent_notes() {
        let mut log = Log::new();
        let old = [
            note(1, "alpha"),
            note(2, "beta"),
            note(3, "gamma"),
            note(4, "delta"),
        ];
        log.extend(old.clone());
        let n = note(5, "something unrelated entirely");
        log.insert(n.clone());
        let picked: Vec<Id> = shortlist(&log, &n).iter().map(|e| e.id).collect();
        assert_eq!(
            picked,
            vec![old[3].id, old[2].id, old[1].id],
            "three most recent"
        );
        let first = note(0, "the very first note");
        let mut solo = Log::new();
        solo.insert(first.clone());
        assert!(shortlist(&solo, &first).is_empty());
    }

    #[test]
    fn lessons_are_non_empty_earlier_and_newest_first() {
        let idea = note(1, "Sell to clinics");
        let mut log = Log::new();
        let l1 = lesson(2, &idea, "No budget");
        let skipped = lesson(3, &idea, "");
        let l2 = lesson(4, &idea, "Too slow");
        let n = note(5, "New idea");
        let later = lesson(6, &idea, "After the note");
        log.extend([
            idea.clone(),
            l1.clone(),
            skipped,
            l2.clone(),
            n.clone(),
            later,
        ]);
        let got = lessons_before(&log, &n);
        let ids: Vec<Id> = got.iter().map(|l| l.event.id).collect();
        assert_eq!(ids, vec![l2.id, l1.id]);
        assert_eq!(got[0].idea_text, Some("Sell to clinics"));
        assert_eq!(got[0].lesson.why, "Too slow");
    }
}

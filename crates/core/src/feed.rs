//! Pure helpers for the notes feed and composing notes.

use crate::event::{Event, EventBody, NoteKind, Rel};
use crate::log::Log;

/// Longest note we accept, in characters.
pub const MAX_NOTE_CHARS: usize = 4000;

/// Which events the feed shows. `None` means "all".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeedFilter {
    pub author: Option<String>,
    pub kind: Option<NoteKind>,
}

impl FeedFilter {
    pub fn is_active(&self) -> bool {
        self.author.is_some() || self.kind.is_some()
    }

    /// Notes match on author and kind. Status changes count as being about
    /// an idea, so they show under "all kinds" and "idea". Verdicts are
    /// never shown in the feed.
    pub fn matches(&self, e: &Event) -> bool {
        if self.author.as_ref().is_some_and(|a| *a != e.author) {
            return false;
        }
        match &e.body {
            EventBody::Note(n) => self.kind.is_none_or(|k| k == n.kind),
            EventBody::Status(_) => self.kind.is_none_or(|k| k == NoteKind::Idea),
            EventBody::Verdict(_) => false,
        }
    }

    /// Feed events, newest first.
    pub fn apply<'a>(&'a self, log: &'a Log) -> impl Iterator<Item = &'a Event> + 'a {
        log.newest_first().filter(move |e| self.matches(e))
    }
}

/// The first line of a note, cut at a word boundary to at most `max` chars.
pub fn excerpt(text: &str, max: usize) -> String {
    let line = text
        .split('\n')
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let more_lines = text.trim().contains('\n');
    if line.chars().count() <= max {
        return if more_lines {
            format!("{line}…")
        } else {
            line.to_owned()
        };
    }
    let cut: String = line.chars().take(max).collect();
    let at_word = match cut.rfind(char::is_whitespace) {
        Some(i) if i > max / 2 => &cut[..i],
        _ => &cut,
    };
    format!(
        "{}…",
        at_word.trim_end_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
    )
}

/// A sensible starting relation when responding to a note of this kind.
pub fn default_rel(target: NoteKind) -> Rel {
    match target {
        NoteKind::Question => Rel::Answers,
        NoteKind::Idea => Rel::Extends,
        NoteKind::Belief | NoteKind::Disagreement => Rel::Challenges,
        NoteKind::Evidence | NoteKind::Note => Rel::Related,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum NoteTextError {
    Empty,
    TooLong { chars: usize },
}

/// Trims surrounding whitespace and checks the length.
pub fn clean_note_text(text: &str) -> Result<String, NoteTextError> {
    let t = text.trim();
    let chars = t.chars().count();
    if chars == 0 {
        Err(NoteTextError::Empty)
    } else if chars > MAX_NOTE_CHARS {
        Err(NoteTextError::TooLong { chars })
    } else {
        Ok(t.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Decision, IdeaStatus, Note, StatusChange, Verdict};
    use crate::id::Id;

    fn ev(n: u64, author: &str, body: EventBody) -> Event {
        Event::new(
            Id::new(1_761_843_481_000 + n * 1000, u128::from(n)),
            author,
            body,
        )
    }

    fn note(n: u64, author: &str, kind: NoteKind) -> Event {
        ev(
            n,
            author,
            EventBody::Note(Note {
                kind,
                text: format!("note {n}"),
                links: vec![],
            }),
        )
    }

    #[test]
    fn filters_by_author_and_kind() {
        let idea = note(1, "Asha", NoteKind::Idea);
        let mut log = Log::new();
        log.extend([
            idea.clone(),
            note(2, "Sankalp", NoteKind::Question),
            note(3, "Asha", NoteKind::Question),
            ev(
                4,
                "Sankalp",
                EventBody::Status(StatusChange {
                    idea: idea.id,
                    status: IdeaStatus::Parked,
                    lesson: None,
                }),
            ),
            ev(
                5,
                "Asha",
                EventBody::Verdict(Verdict {
                    target: idea.id,
                    decision: Decision::Confirm,
                }),
            ),
        ]);
        let ids = |f: &FeedFilter| {
            f.apply(&log)
                .map(|e| (e.id.timestamp_ms() - 1_761_843_481_000) / 1000)
                .collect::<Vec<_>>()
        };

        let all = FeedFilter::default();
        assert!(!all.is_active());
        assert_eq!(ids(&all), vec![4, 3, 2, 1], "newest first, verdicts hidden");

        let asha = FeedFilter {
            author: Some("Asha".into()),
            kind: None,
        };
        assert_eq!(ids(&asha), vec![3, 1]);

        let questions = FeedFilter {
            author: None,
            kind: Some(NoteKind::Question),
        };
        assert_eq!(ids(&questions), vec![3, 2]);

        let ideas = FeedFilter {
            author: None,
            kind: Some(NoteKind::Idea),
        };
        assert_eq!(ids(&ideas), vec![4, 1], "status changes are about ideas");

        let both = FeedFilter {
            author: Some("Sankalp".into()),
            kind: Some(NoteKind::Question),
        };
        assert!(both.is_active());
        assert_eq!(ids(&both), vec![2]);
    }

    #[test]
    fn excerpts_are_one_short_line() {
        assert_eq!(excerpt("Short", 40), "Short");
        assert_eq!(excerpt("  \nFirst line\nsecond", 40), "First line…");
        assert_eq!(
            excerpt("Clinics will pay for intake automation, probably", 30),
            "Clinics will pay for intake…"
        );
        assert_eq!(
            excerpt("Supercalifragilisticexpialidocious", 10),
            "Supercalif…"
        );
        assert_eq!(excerpt("ünïcödé ñötes are fine", 8), "ünïcödé…");
        assert_eq!(excerpt("", 10), "");
    }

    #[test]
    fn default_relations_fit_the_target() {
        assert_eq!(default_rel(NoteKind::Question), Rel::Answers);
        assert_eq!(default_rel(NoteKind::Belief), Rel::Challenges);
        assert_eq!(default_rel(NoteKind::Idea), Rel::Extends);
        assert_eq!(default_rel(NoteKind::Note), Rel::Related);
    }

    #[test]
    fn note_text_is_trimmed_and_bounded() {
        assert_eq!(clean_note_text("  hello \n"), Ok("hello".into()));
        assert_eq!(clean_note_text(" \n\t "), Err(NoteTextError::Empty));
        let long = "x".repeat(MAX_NOTE_CHARS + 1);
        assert_eq!(
            clean_note_text(&long),
            Err(NoteTextError::TooLong {
                chars: MAX_NOTE_CHARS + 1
            })
        );
        assert!(
            clean_note_text(&"é".repeat(MAX_NOTE_CHARS)).is_ok(),
            "counts chars, not bytes"
        );
    }
}

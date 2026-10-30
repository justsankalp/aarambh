//! The judge: links and risk flags for new notes, and the daily digest.
//! Runs in GitHub Actions; the model is behind the [`llm::Judge`] trait.

pub mod data;
pub mod digest;
pub mod gemini;
pub mod llm;
pub mod notes;
pub mod util;

/// "1 note", "3 notes".
pub fn notes_label(n: usize) -> String {
    if n == 1 {
        "1 note".into()
    } else {
        format!("{n} notes")
    }
}

//! Shared types and pure logic for aarambh. Builds for native and wasm32.

pub mod digest;
pub mod event;
pub mod feed;
pub mod id;
pub mod judging;
pub mod judgment;
pub mod log;
pub mod search;
pub mod sync;
pub mod taxonomy;
pub mod time;

pub use event::{
    Decision, Event, EventBody, IdeaStatus, Lesson, Link, Note, NoteKind, Rel, StatusChange,
    Verdict,
};
pub use id::Id;
pub use judgment::{EdgeRel, JudgedEdge, Judgment, Risk, RiskLevel, Strength};
pub use log::{Appended, Log, ParsedDay, append_to_day_file, parse_day_file};
pub use taxonomy::{FailureMode, Taxonomy};
pub use time::{Day, Timestamp};

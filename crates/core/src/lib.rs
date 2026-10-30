//! Shared types and pure logic for aarambh. Builds for native and wasm32.

pub mod event;
pub mod id;
pub mod judgment;
pub mod time;

pub use event::{
    Decision, Event, EventBody, IdeaStatus, Lesson, Link, Note, NoteKind, Rel, StatusChange,
    Verdict,
};
pub use id::Id;
pub use judgment::{EdgeRel, JudgedEdge, Judgment, Risk, RiskLevel, Strength};
pub use time::{Day, Timestamp};

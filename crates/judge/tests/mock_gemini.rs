//! The judge against a mock Gemini server and a temporary data directory.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use aarambh_core::digest::{DigestState, STATE_PATH};
use aarambh_core::{
    Event, EventBody, Id, IdeaStatus, Judgment, Lesson, Note, NoteKind, StatusChange, Timestamp,
    append_to_day_file,
};
use judge::data::Repo;
use judge::digest::{Outcome, write_digest};
use judge::gemini::Gemini;
use judge::llm::{Judge, LlmError, Request};
use judge::notes::judge_notes;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MODEL: &str = "gemini-test";
const T0: u64 = 1_761_843_481_000;
const NOW: Timestamp = Timestamp(T0 + 86_400_000);

static DIRS: AtomicU32 = AtomicU32::new(0);

/// A fresh `data/` directory.
fn data_dir() -> PathBuf {
    let n = DIRS.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("aarambh-judge-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("events")).unwrap();
    d
}

fn write_events(dir: &std::path::Path, events: &[Event]) {
    let mut by_day = std::collections::BTreeMap::<String, Vec<Event>>::new();
    for e in events {
        by_day
            .entry(e.ts.day().to_string())
            .or_default()
            .push(e.clone());
    }
    for (day, evs) in by_day {
        let p = dir.join(format!("events/{day}.json"));
        let existing = fs::read_to_string(&p).ok();
        fs::write(
            &p,
            append_to_day_file(existing.as_deref(), &evs).unwrap().text,
        )
        .unwrap();
    }
}

fn note(n: u64, author: &str, kind: NoteKind, text: &str) -> Event {
    Event::new(
        Id::new(T0 + n * 1000, u128::from(n)),
        author,
        EventBody::Note(Note {
            kind,
            text: text.into(),
            links: vec![],
        }),
    )
}

fn answer(v: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "candidates": [{ "finishReason": "STOP", "content": { "parts": [{ "text": v.to_string() }] } }]
    }))
}

async fn gemini(server: &MockServer) -> Gemini {
    let mut g = Gemini::new("test-key".into(), Some(server.uri())).unwrap();
    g.backoff = Duration::from_millis(1);
    g
}

fn endpoint() -> String {
    format!("/v1beta/models/{MODEL}:generateContent")
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn user_text(body: &Value) -> String {
    body["contents"][0]["parts"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

// ---- Gemini client ----------------------------------------------------------

#[tokio::test]
async fn sends_the_key_and_schema_and_retries_rate_limits() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(endpoint()))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(endpoint()))
        .and(header("x-goog-api-key", "test-key"))
        .respond_with(answer(json!({ "ok": true })))
        .mount(&server)
        .await;
    let g = gemini(&server).await;
    let schema = json!({ "type": "object", "properties": { "ok": { "type": "boolean" } }, "required": ["ok"] });
    let out = g
        .generate_json(&Request {
            model: MODEL,
            system: "sys",
            user: "hello",
            schema: &schema,
            max_output_tokens: 100,
        })
        .await
        .unwrap();
    assert_eq!(out, json!({ "ok": true }));
    let sent = bodies(&server).await;
    assert_eq!(sent.len(), 3, "two 429s, then success");
    let b = &sent[2];
    assert_eq!(b["systemInstruction"]["parts"][0]["text"], "sys");
    assert_eq!(user_text(b), "hello");
    assert_eq!(
        b["generationConfig"]["responseMimeType"],
        "application/json"
    );
    assert_eq!(b["generationConfig"]["responseSchema"]["type"], "OBJECT");
    assert_eq!(b["generationConfig"]["maxOutputTokens"], 100);
}

#[tokio::test]
async fn gives_up_after_four_retries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let g = gemini(&server).await;
    let schema = json!({ "type": "object" });
    let err = g
        .generate_json(&Request {
            model: MODEL,
            system: "",
            user: "",
            schema: &schema,
            max_output_tokens: 10,
        })
        .await
        .unwrap_err();
    assert_eq!(err, LlmError::Unavailable("HTTP 503".into()));
    assert_eq!(server.received_requests().await.unwrap().len(), 5);
}

#[tokio::test]
async fn client_errors_are_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({ "error": { "message": "model not found" } })),
        )
        .mount(&server)
        .await;
    let g = gemini(&server).await;
    let schema = json!({ "type": "object" });
    let err = g
        .generate_json(&Request {
            model: MODEL,
            system: "",
            user: "",
            schema: &schema,
            max_output_tokens: 10,
        })
        .await
        .unwrap_err();
    assert_eq!(
        err,
        LlmError::Rejected {
            status: 400,
            message: "model not found".into()
        }
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[test]
fn a_missing_key_is_reported() {
    assert!(matches!(
        Gemini::new("  ".into(), None),
        Err(LlmError::MissingKey)
    ));
}

// ---- Judging notes ----------------------------------------------------------

#[tokio::test]
async fn judges_only_new_notes_with_a_shortlist_and_validates_the_answer() {
    let dir = data_dir();
    let first = note(
        1,
        "Asha",
        NoteKind::Idea,
        "Clinics will pay for intake automation",
    );
    let mut events = vec![first.clone()];
    // Twelve filler notes: the request must carry at most 8 of them.
    for n in 2..14 {
        events.push(note(
            n,
            "Sankalp",
            NoteKind::Note,
            &format!("Clinic intake detail number {n}"),
        ));
    }
    let already = note(20, "Asha", NoteKind::Note, "Already judged");
    let target = note(
        30,
        "Sankalp",
        NoteKind::Question,
        "Do clinics pay for intake tools?",
    );
    events.extend([already.clone(), target.clone()]);
    write_events(&dir, &events);
    let repo = Repo::new(&dir);
    // Everything except `target` and `first` was judged before.
    for e in &events[1..events.len() - 1] {
        repo.write_judgment(&Judgment {
            note_id: e.id,
            model: "old".into(),
            ts: NOW,
            edges: vec![],
            risks: vec![],
        })
        .unwrap();
    }

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(answer(json!({
            "edges": [
                { "to": "c1", "rel": "answers", "strength": "clear", "why": "It asks\nwhat the idea claims." },
                { "to": "c1", "rel": "related", "strength": "possible", "why": "Duplicate target, dropped." },
                { "to": "c42", "rel": "supports", "strength": "clear", "why": "Unknown handle, dropped." },
                { "to": "c2", "rel": "hates", "strength": "clear", "why": "Unknown relation, dropped." },
                { "to": "c2", "rel": "duplicates", "strength": "possible", "why": "Same thing." }
            ]
        })))
        .mount(&server)
        .await;
    let g = gemini(&server).await;
    let report = judge_notes(&g, &repo, MODEL, NOW, None).await.unwrap();
    assert_eq!(
        (report.judged, report.without_call, report.remaining),
        (1, 1, 0)
    );
    assert!(report.failed.is_empty() && report.stopped.is_none());

    let sent = bodies(&server).await;
    assert_eq!(
        sent.len(),
        1,
        "the first note has nothing to compare against: no call"
    );
    let text = user_text(&sent[0]);
    assert!(text.contains("Do clinics pay for intake tools?"));
    assert!(
        text.contains("c8 (") && !text.contains("c9 ("),
        "at most 8 earlier notes"
    );

    let j: Judgment = serde_json::from_str(
        &fs::read_to_string(dir.join(format!("judgments/{}.json", target.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(j.model, MODEL);
    assert_eq!(
        j.edges.len(),
        2,
        "unknown handles, relations and duplicate targets are dropped"
    );
    assert_eq!(j.edges[0].why, "It asks what the idea claims.");
    assert!(j.risks.is_empty());
    let empty: Judgment = serde_json::from_str(
        &fs::read_to_string(dir.join(format!("judgments/{}.json", first.id))).unwrap(),
    )
    .unwrap();
    assert!(empty.edges.is_empty() && empty.risks.is_empty());

    // A second run has nothing left to do.
    let again = judge_notes(&g, &repo, MODEL, NOW, None).await.unwrap();
    assert_eq!((again.judged, again.without_call), (0, 0));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn risk_flags_must_cite_a_shown_lesson_and_a_known_mode() {
    let dir = data_dir();
    let old_idea = note(1, "Asha", NoteKind::Idea, "Sell scheduling to dentists");
    let dropped = Event::new(
        Id::new(T0 + 2000, 2),
        "Asha",
        EventBody::Status(StatusChange {
            idea: old_idea.id,
            status: IdeaStatus::Dropped,
            lesson: Some(Lesson {
                modes: vec!["no-paying-customer".into()],
                why: "Nobody had budget".into(),
                ..Default::default()
            }),
        }),
    );
    let new_idea = note(3, "Sankalp", NoteKind::Idea, "Sell scheduling to vets");
    write_events(&dir, &[old_idea, dropped.clone(), new_idea.clone()]);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(answer(json!({
            "edges": [],
            "risks": [
                { "lesson": "l1", "mode": "no-paying-customer", "level": "possibly", "why": "Same buyer, no budget named." },
                { "lesson": "l1", "mode": "made-up-mode", "level": "clearly", "why": "dropped" },
                { "lesson": "l7", "mode": "bad-timing", "level": "clearly", "why": "dropped" }
            ]
        })))
        .mount(&server)
        .await;
    let repo = Repo::new(&dir);
    judge_notes(&gemini(&server).await, &repo, MODEL, NOW, None)
        .await
        .unwrap();
    let text = user_text(&bodies(&server).await[0]);
    assert!(
        text.contains("PAST LESSONS")
            && text.contains("Nobody had budget")
            && text.contains("No paying customer")
    );
    let j: Judgment = serde_json::from_str(
        &fs::read_to_string(dir.join(format!("judgments/{}.json", new_idea.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(j.risks.len(), 1);
    assert_eq!(j.risks[0].lesson_event, dropped.id);
    assert_eq!(j.risks[0].mode, "no-paying-customer");
}

#[tokio::test]
async fn a_rate_limited_run_stops_and_leaves_the_rest_for_later() {
    let dir = data_dir();
    let notes: Vec<Event> = (1..=4)
        .map(|n| note(n, "Asha", NoteKind::Note, &format!("clinic note {n}")))
        .collect();
    write_events(&dir, &notes);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    let repo = Repo::new(&dir);
    let r = judge_notes(&gemini(&server).await, &repo, MODEL, NOW, None)
        .await
        .unwrap();
    assert_eq!(r.without_call, 1, "the first note needs no call");
    assert_eq!(r.judged, 0);
    assert_eq!(r.remaining, 3);
    assert_eq!(r.stopped.as_deref(), Some("rate limited; try again later"));
    assert_eq!(
        repo.judged_ids().unwrap().len(),
        1,
        "nothing written for unjudged notes"
    );
}

// ---- Digest -----------------------------------------------------------------

fn digest_answer() -> Value {
    json!({
        "brief": "Asha proposed selling to clinics; Sankalp asked who pays.",
        "moved": ["Asha proposed clinic intake automation.", "Sankalp asked who signs the cheque."],
        "diverge": [
            { "topic": "Who buys", "positions": [
                { "who": "Asha", "says": "Clinic managers" }, { "who": "Sankalp", "says": "Clinic owners" }
            ], "assumption": "Managers control software budgets", "notes": ["n1", "n2"] },
            { "topic": "Has only one side", "positions": [{ "who": "Asha", "says": "x" }], "assumption": "", "notes": [] },
            { "topic": "Unknown person", "positions": [
                { "who": "Asha", "says": "a" }, { "who": "Mallory", "says": "b" }
            ], "assumption": "", "notes": [] }
        ],
        "patterns": [
            { "text": "Buyer is unclear", "notes": ["n1", "n2"] },
            { "text": "Only one note", "notes": ["n1"] },
            { "text": "Same note twice", "notes": ["n1", "n1"] }
        ],
        "untested": [{ "text": "Clinics have budget for this", "notes": ["n1"] }],
        "next_moves": ["Ask three clinic owners who approves software spend."],
        "state": {
            "disagreements": [],
            "assumptions": [{ "text": "Clinics have budget", "status": "untested", "notes": ["n1", "n99"] }],
            "open_questions": [{ "text": "Who signs the cheque?", "note": "n2" }],
            "patterns": []
        }
    })
}

#[tokio::test]
async fn digest_skips_quietly_when_nothing_is_new() {
    let dir = data_dir();
    let n = note(1, "Asha", NoteKind::Idea, "An idea");
    write_events(&dir, std::slice::from_ref(&n));
    let repo = Repo::new(&dir);
    repo.write_state(&DigestState {
        v: 1,
        through: Some(n.ts),
        ..Default::default()
    })
    .unwrap();
    let before = fs::read_to_string(dir.join("digests/state.json")).unwrap();
    let server = MockServer::start().await;
    let out = write_digest(&gemini(&server).await, &repo, MODEL, NOW)
        .await
        .unwrap();
    assert!(matches!(out, Outcome::NothingNew));
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "no model call"
    );
    assert_eq!(
        fs::read_to_string(dir.join("digests/state.json")).unwrap(),
        before,
        "nothing rewritten"
    );
    assert!(
        fs::read_dir(dir.join("digests")).unwrap().count() == 1,
        "no markdown written"
    );
}

#[tokio::test]
async fn digest_writes_markdown_and_state_from_a_checked_answer() {
    let dir = data_dir();
    let idea = note(
        1,
        "Asha",
        NoteKind::Idea,
        "Clinics will pay for intake automation",
    );
    let q = note(
        2,
        "Sankalp",
        NoteKind::Question,
        "Who signs the cheque at a clinic?",
    );
    let parked = Event::new(
        Id::new(T0 + 3000, 3),
        "Sankalp",
        EventBody::Status(StatusChange {
            idea: idea.id,
            status: IdeaStatus::Parked,
            lesson: None,
        }),
    );
    write_events(&dir, &[idea.clone(), q.clone(), parked.clone()]);
    let repo = Repo::new(&dir);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(answer(digest_answer()))
        .mount(&server)
        .await;

    let out = write_digest(&gemini(&server).await, &repo, MODEL, NOW)
        .await
        .unwrap();
    let Outcome::Written { path, new_events } = out else {
        panic!("expected a digest")
    };
    assert_eq!(new_events, 3);
    assert!(path.ends_with(format!("digests/{}.md", NOW.day())));

    let sent = bodies(&server).await;
    let text = user_text(&sent[0]);
    assert!(text.contains("People: Asha, Sankalp"));
    assert!(text.contains("This is the first digest."));
    assert!(text.contains("[n3]") && text.contains("marked idea [n1] parked"));
    let who = &sent[0]["generationConfig"]["responseSchema"]["properties"]["diverge"]["items"]["properties"]
        ["positions"]["items"]["properties"]["who"];
    assert_eq!(
        who["enum"],
        json!(["Asha", "Sankalp"]),
        "people are constrained to the notebook's authors"
    );

    let md = fs::read_to_string(&path).unwrap();
    assert!(
        md.starts_with("# Friday 31 Oct 2025\n\n## Summary · 16:58 UTC"),
        "{md}"
    );
    for section in [
        "### Today in brief",
        "### What moved",
        "### Where you two diverge",
        "### Patterns",
        "### Untested assumptions",
        "### Suggested next moves",
    ] {
        assert!(md.contains(section), "missing {section}");
    }
    assert!(
        md.contains("- **Asha:** Clinic managers")
            && md.contains("- *Underneath:* Managers control software budgets")
    );
    assert!(
        !md.contains("Has only one side") && !md.contains("Mallory"),
        "one-sided or unknown-person divergences dropped"
    );
    assert!(md.contains("- Buyer is unclear (2 notes)"));
    assert!(
        !md.contains("Only one note") && !md.contains("Same note twice"),
        "patterns need two distinct notes"
    );
    assert!(md.contains("1. Ask three clinic owners"));

    let state =
        DigestState::parse(&fs::read_to_string(dir.join("digests/state.json")).unwrap()).unwrap();
    assert_eq!(state.through, Some(parked.ts));
    assert_eq!(state.model, MODEL);
    assert_eq!(state.ideas.len(), 1);
    assert_eq!(
        state.ideas[0].status,
        IdeaStatus::Parked,
        "status comes from events"
    );
    assert_eq!(state.ideas[0].timeline.len(), 2);
    assert_eq!(
        state.assumptions[0].note_ids,
        vec![idea.id],
        "unknown handles dropped"
    );
    assert_eq!(state.open_questions[0].note_id, Some(q.id));
    assert_eq!(STATE_PATH, "data/digests/state.json");

    // Nothing new: skipped. Then a later note the same day appends.
    assert!(matches!(
        write_digest(&gemini(&server).await, &repo, MODEL, NOW)
            .await
            .unwrap(),
        Outcome::NothingNew
    ));
    let later = note(
        5,
        "Asha",
        NoteKind::Evidence,
        "Two owners said they approve software",
    );
    write_events(&dir, std::slice::from_ref(&later));
    write_digest(
        &gemini(&server).await,
        &repo,
        MODEL,
        Timestamp(NOW.0 + 3_600_000),
    )
    .await
    .unwrap();
    let md2 = fs::read_to_string(&path).unwrap();
    assert!(md2.starts_with(&md.trim_end()[..40]));
    assert_eq!(md2.matches("## Summary · ").count(), 2);
    assert!(md2.contains("\n---\n\n## Summary · 17:58 UTC"));
    let second = bodies(&server).await;
    let text2 = user_text(second.last().unwrap());
    assert!(
        text2.contains("Two owners said")
            && !text2.contains("Clinics will pay for intake automation\n\"\"\"")
    );
    assert!(
        text2.contains("\"Clinics have budget\""),
        "previous state is passed on"
    );
}

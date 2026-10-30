//! The Map's graph: notes as nodes; human links, machine suggestions and risk
//! flags as edges; uncertain suggestions set aside in a "Maybe" list until
//! someone confirms or rejects them.

use std::collections::{HashMap, HashSet};

use crate::event::{Decision, EventBody, IdeaStatus, NoteKind};
use crate::feed::excerpt;
use crate::id::Id;
use crate::judgment::{EdgeRel, Judgment, RiskLevel, Strength};
use crate::log::Log;

/// Most notes drawn at once (the newest).
pub const MAX_NODES: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: Id,
    pub kind: NoteKind,
    pub author: String,
    pub label: String,
    /// Ideas are drawn larger.
    pub hub: bool,
    pub status: Option<IdeaStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// A link someone made by responding.
    Human,
    /// A clear machine suggestion nobody has ruled on.
    Machine,
    /// A machine suggestion someone confirmed.
    Confirmed,
    /// The note may repeat a past lesson's failure mode (to that idea).
    Risk,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// Indices into `Graph::nodes`.
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Maybe {
    Link {
        id: Id,
        from: Id,
        to: Id,
        rel: &'static str,
        why: String,
    },
    Risk {
        id: Id,
        note: Id,
        idea: Option<Id>,
        mode: String,
        why: String,
    },
}

impl Maybe {
    pub fn id(&self) -> Id {
        match self {
            Maybe::Link { id, .. } | Maybe::Risk { id, .. } => *id,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Possible links and risks awaiting a verdict, newest note first.
    pub maybe: Vec<Maybe>,
}

impl Graph {
    pub fn index_of(&self, id: Id) -> Option<usize> {
        self.nodes.binary_search_by(|n| n.id.cmp(&id)).ok()
    }
}

fn rel_label(rel: EdgeRel) -> &'static str {
    match rel {
        EdgeRel::Rel(r) => r.as_str(),
        EdgeRel::Duplicates => "duplicates",
    }
}

pub fn build(log: &Log, judgments: &[Judgment]) -> Graph {
    let mut nodes: Vec<Node> = log
        .newest_first()
        .filter_map(|e| {
            let n = e.as_note()?;
            Some(Node {
                id: e.id,
                kind: n.kind,
                author: e.author.clone(),
                label: excerpt(&n.text, 60),
                hub: n.kind == NoteKind::Idea,
                status: (n.kind == NoteKind::Idea).then(|| log.status_of(e.id)),
            })
        })
        .take(MAX_NODES)
        .collect();
    nodes.sort_by_key(|n| n.id);
    let mut g = Graph {
        nodes,
        ..Default::default()
    };
    let index: HashMap<Id, usize> = g.nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();

    let mut joined: HashSet<(usize, usize)> = HashSet::new();
    let pair = |a: usize, b: usize| (a.min(b), a.max(b));
    let mut add = |g: &mut Graph, from: Id, to: Id, kind: EdgeKind, label: &str| {
        let (Some(&a), Some(&b)) = (index.get(&from), index.get(&to)) else {
            return;
        };
        if a != b && joined.insert(pair(a, b)) {
            g.edges.push(Edge {
                from: a,
                to: b,
                kind,
                label: label.to_owned(),
            });
        }
    };

    // Human links first, so a machine suggestion never doubles one.
    for n in &g.nodes.clone() {
        if let Some(note) = log.get(n.id).and_then(|e| e.as_note()) {
            for l in &note.links {
                add(&mut g, n.id, l.to, EdgeKind::Human, l.rel.as_str());
            }
        }
    }

    // Machine edges are added after human ones, strongest claim first, so
    // when two connect the same pair the one a person stands behind wins.
    let mut confirmed: Vec<(Id, Id, &str)> = Vec::new();
    let mut machine: Vec<(Id, Id, &str)> = Vec::new();
    let mut risky: Vec<(Id, Id, &str)> = Vec::new();
    let mut maybe: Vec<(crate::time::Timestamp, Maybe)> = Vec::new();
    for j in judgments {
        let Some(note) = log.get(j.note_id) else {
            continue;
        };
        for e in &j.edges {
            if !log.contains(e.to) {
                continue;
            }
            let label = rel_label(e.rel);
            match (log.current_verdict(e.id), e.strength) {
                (Some(Decision::Reject), _) => {}
                (Some(Decision::Confirm), _) => confirmed.push((j.note_id, e.to, label)),
                (None, Strength::Clear) => machine.push((j.note_id, e.to, label)),
                (None, Strength::Possible) => maybe.push((
                    note.ts,
                    Maybe::Link {
                        id: e.id,
                        from: j.note_id,
                        to: e.to,
                        rel: label,
                        why: e.why.clone(),
                    },
                )),
            }
        }
        for r in &j.risks {
            let idea = log.get(r.lesson_event).and_then(|ev| match &ev.body {
                EventBody::Status(s) => Some(s.idea),
                _ => None,
            });
            match (log.current_verdict(r.id), r.level) {
                (Some(Decision::Reject), _) => {}
                (Some(Decision::Confirm), _) | (None, RiskLevel::Clearly) => {
                    if let Some(idea) = idea {
                        risky.push((j.note_id, idea, &r.mode));
                    }
                }
                (None, RiskLevel::Possibly) => maybe.push((
                    note.ts,
                    Maybe::Risk {
                        id: r.id,
                        note: j.note_id,
                        idea,
                        mode: r.mode.clone(),
                        why: r.why.clone(),
                    },
                )),
            }
        }
    }
    for (list, kind) in [
        (confirmed, EdgeKind::Confirmed),
        (machine, EdgeKind::Machine),
        (risky, EdgeKind::Risk),
    ] {
        for (from, to, label) in list {
            add(&mut g, from, to, kind, label);
        }
    }
    maybe.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.id().cmp(&a.1.id())));
    g.maybe = maybe.into_iter().map(|(_, m)| m).collect();
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, Lesson, Link, Note, Rel, StatusChange, Verdict};
    use crate::judgment::{JudgedEdge, Risk};
    use crate::time::Timestamp;

    const T0: u64 = 1_761_843_481_000;

    fn ev(n: u64, body: EventBody) -> Event {
        Event::new(Id::new(T0 + n * 1000, u128::from(n)), "A", body)
    }

    fn note(n: u64, kind: NoteKind, links: Vec<Link>) -> Event {
        ev(
            n,
            EventBody::Note(Note {
                kind,
                text: format!("note {n}"),
                links,
            }),
        )
    }

    fn edge(n: u64, to: Id, rel: EdgeRel, strength: Strength) -> JudgedEdge {
        JudgedEdge {
            id: Id::new(T0, u128::from(1000 + n)),
            to,
            rel,
            strength,
            why: format!("why {n}"),
        }
    }

    fn verdict(n: u64, target: Id, decision: Decision) -> Event {
        ev(n, EventBody::Verdict(Verdict { target, decision }))
    }

    fn judgment(note: &Event, edges: Vec<JudgedEdge>, risks: Vec<Risk>) -> Judgment {
        Judgment {
            note_id: note.id,
            model: "m".into(),
            ts: Timestamp(T0),
            edges,
            risks,
        }
    }

    #[test]
    fn nodes_edges_and_verdicts() {
        let idea = note(1, NoteKind::Idea, vec![]);
        let q = note(
            2,
            NoteKind::Question,
            vec![Link {
                to: idea.id,
                rel: Rel::Challenges,
            }],
        );
        let ev3 = note(3, NoteKind::Evidence, vec![]);
        // `clear` (ev3 -> idea) and `confirmed` (idea -> ev3) join the same pair.
        let b = note(4, NoteKind::Belief, vec![]);
        let clear = edge(1, idea.id, EdgeRel::Rel(Rel::Supports), Strength::Clear);
        let doubles_human = edge(2, idea.id, EdgeRel::Rel(Rel::Related), Strength::Clear);
        let possible = edge(3, ev3.id, EdgeRel::Duplicates, Strength::Possible);
        let confirmed = edge(4, ev3.id, EdgeRel::Rel(Rel::Extends), Strength::Possible);
        let rejected = edge(5, q.id, EdgeRel::Rel(Rel::Related), Strength::Clear);
        let mut log = Log::new();
        log.extend([idea.clone(), q.clone(), ev3.clone(), b.clone()]);
        log.extend([
            verdict(10, confirmed.id, Decision::Confirm),
            verdict(11, rejected.id, Decision::Reject),
        ]);
        let js = [
            judgment(&ev3, vec![clear.clone()], vec![]),
            judgment(&q, vec![doubles_human], vec![]),
            judgment(&b, vec![possible.clone(), rejected], vec![]),
            judgment(&idea, vec![confirmed], vec![]),
        ];
        let g = build(&log, &js);

        assert_eq!(g.nodes.len(), 4);
        assert!(
            g.nodes.windows(2).all(|w| w[0].id < w[1].id),
            "sorted by id"
        );
        let hub = &g.nodes[g.index_of(idea.id).unwrap()];
        assert!(hub.hub && hub.status == Some(IdeaStatus::Active));

        let kinds: Vec<(EdgeKind, &str)> =
            g.edges.iter().map(|e| (e.kind, e.label.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (EdgeKind::Human, "challenges"),
                (EdgeKind::Confirmed, "extends")
            ],
            "the confirmed link wins over the unconfirmed one between the same notes"
        );
        assert_eq!(
            g.maybe,
            vec![Maybe::Link {
                id: possible.id,
                from: b.id,
                to: ev3.id,
                rel: "duplicates",
                why: "why 3".into()
            }]
        );

        // Confirming the possible link moves it out of Maybe and onto the map.
        log.insert(verdict(12, possible.id, Decision::Confirm));
        let g2 = build(&log, &js);
        assert!(g2.maybe.is_empty());
        assert!(
            g2.edges
                .iter()
                .any(|e| e.kind == EdgeKind::Confirmed && e.label == "duplicates")
        );
    }

    #[test]
    fn risks_point_at_the_lesson_idea() {
        let idea = note(1, NoteKind::Idea, vec![]);
        let dropped = ev(
            2,
            EventBody::Status(StatusChange {
                idea: idea.id,
                status: IdeaStatus::Dropped,
                lesson: Some(Lesson {
                    why: "no budget".into(),
                    ..Default::default()
                }),
            }),
        );
        let new = note(3, NoteKind::Idea, vec![]);
        let clearly = Risk {
            id: Id::new(T0, 1),
            lesson_event: dropped.id,
            mode: "unit-economics".into(),
            level: RiskLevel::Clearly,
            why: "w".into(),
        };
        let possibly = Risk {
            id: Id::new(T0, 2),
            lesson_event: dropped.id,
            mode: "bad-timing".into(),
            level: RiskLevel::Possibly,
            why: "p".into(),
        };
        let mut log = Log::new();
        log.extend([idea.clone(), dropped, new.clone()]);
        let g = build(
            &log,
            &[judgment(&new, vec![], vec![clearly, possibly.clone()])],
        );
        assert_eq!(g.edges.len(), 1);
        assert_eq!(
            (g.edges[0].kind, g.edges[0].label.as_str()),
            (EdgeKind::Risk, "unit-economics")
        );
        assert_eq!(g.nodes[g.edges[0].to].id, idea.id);
        assert_eq!(
            g.nodes[g.index_of(idea.id).unwrap()].status,
            Some(IdeaStatus::Dropped)
        );
        assert!(
            matches!(&g.maybe[..], [Maybe::Risk { id, idea: Some(i), .. }] if *id == possibly.id && *i == idea.id)
        );
    }

    #[test]
    fn caps_nodes_and_drops_edges_to_missing_notes() {
        let mut log = Log::new();
        let notes: Vec<Event> = (0..(MAX_NODES as u64 + 20))
            .map(|n| note(n, NoteKind::Note, vec![]))
            .collect();
        log.extend(notes.clone());
        let oldest = &notes[0];
        let newest = notes.last().unwrap();
        let ghost = Id::new(T0 - 1, 9);
        let js = [judgment(
            newest,
            vec![
                edge(1, oldest.id, EdgeRel::Rel(Rel::Related), Strength::Clear),
                edge(2, ghost, EdgeRel::Rel(Rel::Related), Strength::Possible),
            ],
            vec![],
        )];
        let g = build(&log, &js);
        assert_eq!(g.nodes.len(), MAX_NODES);
        assert!(g.index_of(oldest.id).is_none(), "the oldest notes fall off");
        assert!(g.edges.is_empty(), "edge to a note not drawn is skipped");
        assert!(
            g.maybe.is_empty(),
            "suggestions about unknown notes are ignored"
        );
    }
}

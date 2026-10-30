//! The Map tab: a graph of notes and links on a canvas, a text list of ideas
//! and the digest's running state, and the "Maybe" tray of uncertain
//! machine suggestions to confirm or reject.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aarambh_core::digest::{AssumptionStatus, ideas_state};
use aarambh_core::feed::excerpt;
use aarambh_core::graph::{EdgeKind, Graph, Maybe, build};
use aarambh_core::layout::Layout;
use aarambh_core::{Decision, EventBody, Id, IdeaStatus, NoteKind, Verdict};
use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::CanvasRenderingContext2d;

use crate::cache;
use crate::status::status_label;
use crate::store::Store;
use crate::util::{clock, day_label, local_get, local_set, now_ms};

const MODE_KEY: &str = "aarambh.map";
/// Layout work per animation frame, so scrolling and taps stay smooth.
const FRAME_BUDGET_MS: f64 = 8.0;
const TAP_SLOP_PX: f64 = 8.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Graph,
    List,
}

/// Everything the canvas needs between frames. Kept out of the reactive
/// system: it changes every frame and only the canvas reads it.
struct Canvas {
    graph: Graph,
    layout: Layout,
    scale: f64,
    tx: f64,
    ty: f64,
    /// Fit the whole graph on screen until the person pans or zooms.
    auto_fit: bool,
    pointers: Vec<(i32, f64, f64)>,
    /// Where a single-pointer press started, and how far it has moved.
    press: Option<(f64, f64, f64)>,
    width: f64,
    height: f64,
    running: bool,
    reduce_motion: bool,
}

fn now() -> f64 {
    window().performance().map_or(0.0, |p| p.now())
}

fn css_var(name: &str) -> String {
    document()
        .document_element()
        .and_then(|el| window().get_computed_style(&el).ok().flatten())
        .and_then(|s| s.get_property_value(name).ok())
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "#888".into())
}

struct Palette {
    bg: String,
    ink: String,
    muted: String,
    line: String,
    sage: String,
    amber: String,
    amber_ink: String,
    clay: String,
}

impl Palette {
    fn read() -> Self {
        Palette {
            bg: css_var("--card"),
            ink: css_var("--ink"),
            muted: css_var("--muted"),
            line: css_var("--line"),
            sage: css_var("--sage"),
            amber: css_var("--amber"),
            amber_ink: css_var("--amber-ink"),
            clay: css_var("--clay"),
        }
    }
}

impl Canvas {
    fn to_screen(&self, p: [f64; 2]) -> (f64, f64) {
        (p[0] * self.scale + self.tx, p[1] * self.scale + self.ty)
    }

    fn fit(&mut self) {
        if self.layout.pos.is_empty() {
            return;
        }
        let [x0, y0, x1, y1] = self.layout.bounds();
        let pad = 36.0;
        let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
        self.scale = ((self.width - 2.0 * pad) / w)
            .min((self.height - 2.0 * pad) / h)
            .clamp(0.15, 2.0);
        self.tx = self.width / 2.0 - (x0 + x1) / 2.0 * self.scale;
        self.ty = self.height / 2.0 - (y0 + y1) / 2.0 * self.scale;
    }

    fn radius(&self, hub: bool) -> f64 {
        (if hub { 9.0 } else { 5.5 }) * self.scale.sqrt().clamp(0.7, 1.6)
    }

    fn hit(&self, x: f64, y: f64) -> Option<usize> {
        self.graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let (sx, sy) = self.to_screen(self.layout.pos[i]);
                (
                    i,
                    ((sx - x).powi(2) + (sy - y).powi(2)).sqrt() - self.radius(n.hub),
                )
            })
            .filter(|(_, d)| *d < 12.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    fn zoom_at(&mut self, x: f64, y: f64, factor: f64) {
        let new = (self.scale * factor).clamp(0.15, 4.0);
        let f = new / self.scale;
        self.tx = x - (x - self.tx) * f;
        self.ty = y - (y - self.ty) * f;
        self.scale = new;
        self.auto_fit = false;
    }

    fn draw(&self, ctx: &CanvasRenderingContext2d, dpr: f64, selected: Option<Id>, pal: &Palette) {
        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        ctx.set_fill_style_str(&pal.bg);
        ctx.fill_rect(0.0, 0.0, self.width, self.height);
        if self.reduce_motion && self.running {
            ctx.set_fill_style_str(&pal.muted);
            ctx.set_font("14px system-ui, sans-serif");
            ctx.set_text_align("center");
            let _ = ctx.fill_text("Arranging…", self.width / 2.0, self.height / 2.0);
            return;
        }
        let dash = |on: bool| {
            let pattern = js_sys::Array::new();
            if on {
                pattern.push(&JsValue::from_f64(5.0));
                pattern.push(&JsValue::from_f64(4.0));
            }
            let _ = ctx.set_line_dash(&pattern);
        };
        for e in &self.graph.edges {
            let (ax, ay) = self.to_screen(self.layout.pos[e.from]);
            let (bx, by) = self.to_screen(self.layout.pos[e.to]);
            let (color, width, dashed) = match e.kind {
                EdgeKind::Human => (&pal.muted, 1.4, false),
                EdgeKind::Confirmed => (&pal.sage, 1.6, false),
                EdgeKind::Machine => (&pal.sage, 1.3, true),
                EdgeKind::Risk => (&pal.clay, 1.6, true),
            };
            ctx.set_stroke_style_str(color);
            ctx.set_line_width(width);
            dash(dashed);
            ctx.begin_path();
            ctx.move_to(ax, ay);
            ctx.line_to(bx, by);
            ctx.stroke();
        }
        dash(false);
        for (i, n) in self.graph.nodes.iter().enumerate() {
            let (x, y) = self.to_screen(self.layout.pos[i]);
            let r = self.radius(n.hub);
            // Filled kinds vs. ring kinds keep them apart without colour alone.
            let (fill, ring): (Option<&String>, Option<&String>) = match n.kind {
                NoteKind::Idea => match n.status {
                    Some(IdeaStatus::Decided) => (Some(&pal.amber), Some(&pal.amber_ink)),
                    Some(IdeaStatus::Dropped) => (Some(&pal.line), Some(&pal.muted)),
                    Some(IdeaStatus::Parked) => (Some(&pal.bg), Some(&pal.sage)),
                    _ => (Some(&pal.sage), None),
                },
                NoteKind::Belief => (Some(&pal.amber), None),
                NoteKind::Question => (Some(&pal.bg), Some(&pal.ink)),
                NoteKind::Evidence => (Some(&pal.ink), None),
                NoteKind::Disagreement => (Some(&pal.bg), Some(&pal.clay)),
                NoteKind::Note => (Some(&pal.bg), Some(&pal.muted)),
            };
            ctx.begin_path();
            let _ = ctx.arc(x, y, r, 0.0, std::f64::consts::TAU);
            if let Some(f) = fill {
                ctx.set_fill_style_str(f);
                ctx.fill();
            }
            if let Some(s) = ring {
                ctx.set_stroke_style_str(s);
                ctx.set_line_width(2.0);
                ctx.stroke();
            }
            if Some(n.id) == selected {
                ctx.begin_path();
                let _ = ctx.arc(x, y, r + 4.0, 0.0, std::f64::consts::TAU);
                ctx.set_stroke_style_str(&pal.ink);
                ctx.set_line_width(2.0);
                ctx.stroke();
            }
        }
        if self.scale >= 0.55 {
            ctx.set_font("12px system-ui, -apple-system, sans-serif");
            ctx.set_text_align("center");
            ctx.set_text_baseline("top");
            // A halo in the background colour keeps labels readable over lines.
            ctx.set_line_width(4.0);
            ctx.set_line_join("round");
            ctx.set_stroke_style_str(&pal.bg);
            ctx.set_fill_style_str(&pal.ink);
            for (i, n) in self.graph.nodes.iter().enumerate().filter(|(_, n)| n.hub) {
                let (x, y) = self.to_screen(self.layout.pos[i]);
                let label = excerpt(&n.label, 28);
                let y = y + self.radius(true) + 4.0;
                let _ = ctx.stroke_text(&label, x, y);
                let _ = ctx.fill_text(&label, x, y);
            }
        }
    }
}

type Cell = StoredValue<Option<Canvas>, LocalStorage>;

fn context(canvas: &web_sys::HtmlCanvasElement) -> Option<CanvasRenderingContext2d> {
    canvas.get_context("2d").ok().flatten()?.dyn_into().ok()
}

/// Matches the canvas's pixel size to its CSS size and device pixel ratio.
fn resize(canvas: &web_sys::HtmlCanvasElement, cell: Cell) -> f64 {
    let dpr = window().device_pixel_ratio().max(1.0);
    let (w, h) = (
        f64::from(canvas.client_width()),
        f64::from(canvas.client_height()),
    );
    canvas.set_width((w * dpr) as u32);
    canvas.set_height((h * dpr) as u32);
    cell.update_value(|c| {
        if let Some(c) = c {
            c.width = w;
            c.height = h;
            if c.auto_fit {
                c.fit();
            }
        }
    });
    dpr
}

fn redraw(canvas: NodeRef<html::Canvas>, cell: Cell, selected: Option<Id>) {
    let Some(el) = canvas.get_untracked() else {
        return;
    };
    let Some(ctx) = context(&el) else { return };
    let dpr = window().device_pixel_ratio().max(1.0);
    let pal = Palette::read();
    cell.with_value(|c| {
        if let Some(c) = c {
            c.draw(&ctx, dpr, selected, &pal);
        }
    });
}

/// Steps the layout for about one frame's budget, draws, and schedules the
/// next frame until the layout settles; then saves positions.
fn tick(
    canvas: NodeRef<html::Canvas>,
    cell: Cell,
    alive: Arc<AtomicBool>,
    selected: RwSignal<Option<Id>>,
) {
    if !alive.load(Ordering::Relaxed) {
        return;
    }
    let start = now();
    let mut done = false;
    cell.update_value(|c| {
        let Some(c) = c else {
            done = true;
            return;
        };
        while now() - start < FRAME_BUDGET_MS {
            if c.layout.step(2) {
                break;
            }
        }
        done = c.layout.is_done();
        c.running = !done;
        if c.auto_fit {
            c.fit();
        }
    });
    redraw(canvas, cell, selected.get_untracked());
    if done {
        save_positions(cell);
    } else {
        request_animation_frame(move || tick(canvas, cell, alive, selected));
    }
}

fn save_positions(cell: Cell) {
    let positions: Vec<(String, [f64; 2])> = cell.with_value(|c| {
        c.as_ref().map_or_else(Vec::new, |c| {
            c.graph
                .nodes
                .iter()
                .zip(&c.layout.pos)
                .map(|(n, p)| {
                    (
                        n.id.to_string(),
                        [(p[0] * 10.0).round() / 10.0, (p[1] * 10.0).round() / 10.0],
                    )
                })
                .collect()
        })
    });
    spawn_local(async move {
        let mut map: HashMap<String, [f64; 2]> = cache::get(cache::META, cache::META_LAYOUT)
            .await
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        map.extend(positions);
        if let Ok(json) = serde_json::to_string(&map) {
            let _ = cache::put(cache::META, cache::META_LAYOUT, &json).await;
        }
    });
}

#[component]
fn GraphCanvas(graph: Memo<Graph>, selected: RwSignal<Option<Id>>) -> impl IntoView {
    let canvas = NodeRef::<html::Canvas>::new();
    let cell: Cell = StoredValue::new_local(None);
    let alive = Arc::new(AtomicBool::new(true));
    let cached = StoredValue::new_local(None::<HashMap<Id, [f64; 2]>>);
    {
        let alive = alive.clone();
        on_cleanup(move || alive.store(false, Ordering::Relaxed));
    }

    // (Re)build the layout whenever the graph changes, warm-starting from
    // where nodes are now or were last time.
    let start = {
        let alive = alive.clone();
        move |g: Graph| {
            let Some(el) = canvas.get_untracked() else {
                return;
            };
            let mut known: HashMap<Id, [f64; 2]> = cached.get_value().unwrap_or_default();
            cell.with_value(|c| {
                if let Some(c) = c {
                    known.extend(
                        c.graph
                            .nodes
                            .iter()
                            .map(|n| n.id)
                            .zip(c.layout.pos.iter().copied()),
                    );
                }
            });
            let ids: Vec<Id> = g.nodes.iter().map(|n| n.id).collect();
            let hubs: Vec<bool> = g.nodes.iter().map(|n| n.hub).collect();
            let edges: Vec<(usize, usize)> = g.edges.iter().map(|e| (e.from, e.to)).collect();
            let layout = Layout::new(&ids, &hubs, &edges, &known);
            let reduce_motion = window()
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
                .is_some_and(|m| m.matches());
            let was_running = cell.with_value(|c| c.as_ref().is_some_and(|c| c.running));
            cell.update_value(|c| {
                let (scale, tx, ty, auto_fit, w, h) =
                    c.as_ref().map_or((1.0, 0.0, 0.0, true, 0.0, 0.0), |c| {
                        (c.scale, c.tx, c.ty, c.auto_fit, c.width, c.height)
                    });
                *c = Some(Canvas {
                    graph: g,
                    layout,
                    scale,
                    tx,
                    ty,
                    auto_fit,
                    pointers: Vec::new(),
                    press: None,
                    width: w,
                    height: h,
                    running: true,
                    reduce_motion,
                });
            });
            resize(&el, cell);
            if !was_running {
                let alive = alive.clone();
                request_animation_frame(move || tick(canvas, cell, alive, selected));
            }
        }
    };

    Effect::new({
        let start = start.clone();
        move |_| {
            let g = graph.get();
            if cached.with_value(Option::is_some) {
                start(g);
                return;
            }
            // First run: load cached positions, then lay out.
            let start = start.clone();
            spawn_local(async move {
                let map: HashMap<Id, [f64; 2]> = cache::get(cache::META, cache::META_LAYOUT)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|s| serde_json::from_str::<HashMap<String, [f64; 2]>>(&s).ok())
                    .map(|m| {
                        m.into_iter()
                            .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
                            .collect()
                    })
                    .unwrap_or_default();
                cached.set_value(Some(map));
                start(graph.get_untracked());
            });
        }
    });

    Effect::new(move |_| {
        let sel = selected.get();
        redraw(canvas, cell, sel);
    });

    let resize_listener = window_event_listener(ev::resize, move |_| {
        if let Some(el) = canvas.get_untracked() {
            resize(&el, cell);
            redraw(canvas, cell, selected.get_untracked());
        }
    });
    on_cleanup(move || resize_listener.remove());

    let local =
        move |ev: &web_sys::PointerEvent| (f64::from(ev.offset_x()), f64::from(ev.offset_y()));
    let on_down = move |ev: web_sys::PointerEvent| {
        if let Some(el) = canvas.get_untracked() {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        let (x, y) = local(&ev);
        cell.update_value(|c| {
            if let Some(c) = c {
                c.pointers.retain(|p| p.0 != ev.pointer_id());
                c.pointers.push((ev.pointer_id(), x, y));
                c.press = (c.pointers.len() == 1).then_some((x, y, 0.0));
            }
        });
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let (x, y) = local(&ev);
        let mut changed = false;
        cell.update_value(|c| {
            let Some(c) = c else { return };
            let Some(i) = c.pointers.iter().position(|p| p.0 == ev.pointer_id()) else {
                return;
            };
            let (_, px, py) = c.pointers[i];
            if c.pointers.len() == 1 {
                c.tx += x - px;
                c.ty += y - py;
                if let Some(p) = c.press.as_mut() {
                    p.2 += (x - px).abs() + (y - py).abs();
                    if p.2 > TAP_SLOP_PX {
                        c.auto_fit = false;
                    }
                }
            } else if c.pointers.len() == 2 {
                let other = c.pointers[1 - i];
                let before = ((px - other.1).powi(2) + (py - other.2).powi(2))
                    .sqrt()
                    .max(1.0);
                let after = ((x - other.1).powi(2) + (y - other.2).powi(2))
                    .sqrt()
                    .max(1.0);
                let (mx, my) = ((x + other.1) / 2.0, (y + other.2) / 2.0);
                c.zoom_at(mx, my, after / before);
                c.tx += (x - px) / 2.0;
                c.ty += (y - py) / 2.0;
                c.press = None;
            }
            c.pointers[i] = (ev.pointer_id(), x, y);
            changed = true;
        });
        if changed {
            redraw(canvas, cell, selected.get_untracked());
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let (x, y) = local(&ev);
        let mut tapped = None;
        cell.update_value(|c| {
            let Some(c) = c else { return };
            if let Some((_, _, moved)) = c.press.take()
                && moved <= TAP_SLOP_PX
                && c.pointers.len() == 1
            {
                tapped = Some(c.hit(x, y).map(|i| c.graph.nodes[i].id));
            }
            c.pointers.retain(|p| p.0 != ev.pointer_id());
        });
        if let Some(t) = tapped {
            selected.set(t);
        }
    };
    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        let factor = (-ev.delta_y() * 0.0015).exp();
        cell.update_value(|c| {
            if let Some(c) = c {
                c.zoom_at(f64::from(ev.offset_x()), f64::from(ev.offset_y()), factor);
            }
        });
        redraw(canvas, cell, selected.get_untracked());
    };
    let refit = move |_| {
        cell.update_value(|c| {
            if let Some(c) = c {
                c.auto_fit = true;
                c.fit();
            }
        });
        redraw(canvas, cell, selected.get_untracked());
    };

    let label = move || {
        graph.with(|g| {
            format!(
                "Map of {} notes and {} links. The list view has the same information as text.",
                g.nodes.len(),
                g.edges.len()
            )
        })
    };
    view! {
        <div class="graph-wrap">
            <canvas
                class="graph"
                node_ref=canvas
                role="img"
                aria-label=label
                on:pointerdown=on_down
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=on_up
                on:wheel=on_wheel
            ></canvas>
            <button type="button" class="btn small fit" on:click=refit>
                "Fit"
            </button>
        </div>
        <ul class="legend" aria-label="Legend">
            <li><span class="sw idea"></span>"idea"</li>
            <li><span class="sw belief"></span>"belief"</li>
            <li><span class="sw question"></span>"question"</li>
            <li><span class="sw evidence"></span>"evidence"</li>
            <li><span class="sw disagreement"></span>"disagreement"</li>
            <li><span class="sw note"></span>"note"</li>
            <li><span class="ln human"></span>"your link"</li>
            <li><span class="ln machine"></span>"suggested"</li>
            <li><span class="ln risk"></span>"risk"</li>
        </ul>
    }
}

/// The tapped note, with a way into the feed.
#[component]
fn NoteCard(selected: RwSignal<Option<Id>>) -> impl IntoView {
    let store = expect_context::<Store>();
    move || {
        let id = selected.get()?;
        let (kind, author, ts, text) = store.log.with(|l| {
            let e = l.get(id)?;
            let n = e.as_note()?;
            Some((n.kind, e.author.clone(), e.ts.0, n.text.clone()))
        })?;
        let status = (kind == NoteKind::Idea).then(|| store.log.with(|l| l.status_of(id)));
        Some(view! {
            <article class="card map-card" aria-live="polite">
                <div class="meta">
                    <span class="kind">{kind.as_str()}</span>
                    <span>{author}</span>
                    <span>{day_label(ts, now_ms())}" "{clock(ts)}</span>
                    {status.map(|s| view! { <span class="badge" data-status=status_label(s)>{status_label(s)}</span> })}
                </div>
                <p class="note-text">{text}</p>
                <div class="card-actions">
                    <button
                        type="button"
                        class="link-btn"
                        on:click=move |_| {
                            store.focus.set(Some(id));
                            let _ = window().location().set_hash("notes");
                        }
                    >
                        "Open in Notes"
                    </button>
                    <button type="button" class="link-btn" on:click=move |_| selected.set(None)>
                        "Close"
                    </button>
                </div>
            </article>
        })
    }
}

fn quote(store: Store, id: Id) -> String {
    store
        .log
        .with_untracked(|l| {
            l.get(id)
                .and_then(|e| Some(excerpt(&e.as_note()?.text, 70)))
        })
        .unwrap_or_else(|| "an older note".into())
}

/// Uncertain suggestions, each waiting for a person's call.
#[component]
fn MaybeTray(graph: Memo<Graph>) -> impl IntoView {
    let store = expect_context::<Store>();
    let decide = move |target: Id, decision: Decision| {
        store.post(store.new_event(EventBody::Verdict(Verdict { target, decision })));
    };
    view! {
        <Show when=move || graph.with(|g| !g.maybe.is_empty())>
            <section class="maybe">
                <h3>"Maybe"</h3>
                <p class="muted small-print">"Links and risks the judge wasn't sure about. Confirm to add them to the map."</p>
                <For
                    each=move || graph.with(|g| g.maybe.clone())
                    key=Maybe::id
                    children=move |m| {
                        let id = m.id();
                        let (what, why) = match m {
                            Maybe::Link { from, to, rel, why, .. } => (
                                view! {
                                    <p class="maybe-what">
                                        <q>{quote(store, from)}</q>" "<span class="rel">{rel}</span>" "<q>{quote(store, to)}</q>
                                    </p>
                                }
                                .into_any(),
                                why,
                            ),
                            Maybe::Risk { note, idea, mode, why, .. } => {
                                let label = store.taxonomy.with_untracked(|t| t.label(&mode).to_owned());
                                (
                                    view! {
                                        <p class="maybe-what">
                                            <q>{quote(store, note)}</q>" may repeat "<span class="risk">{label}</span>
                                            {idea.map(|i| view! { " from "<q>{quote(store, i)}</q> })}
                                        </p>
                                    }
                                    .into_any(),
                                    why,
                                )
                            }
                        };
                        view! {
                            <div class="card maybe-item">
                                {what}
                                <p class="muted small-print">{why}</p>
                                <div class="actions">
                                    <button type="button" class="btn small" on:click=move |_| decide(id, Decision::Confirm)>
                                        "Confirm"
                                    </button>
                                    <button type="button" class="btn small quiet" on:click=move |_| decide(id, Decision::Reject)>
                                        "Reject"
                                    </button>
                                </div>
                            </div>
                        }
                    }
                />
            </section>
        </Show>
    }
}

const STATUS_ORDER: [(IdeaStatus, &str); 4] = [
    (IdeaStatus::Active, "Open"),
    (IdeaStatus::Parked, "Parked"),
    (IdeaStatus::Decided, "Decided"),
    (IdeaStatus::Dropped, "Dropped"),
];

#[component]
fn MapList() -> impl IntoView {
    let store = expect_context::<Store>();
    let ideas = Memo::new(move |_| store.log.with(ideas_state));
    let open = move |id: Id| {
        store.focus.set(Some(id));
        let _ = window().location().set_hash("notes");
    };
    view! {
        <section class="map-list">
            <h3>"Ideas"</h3>
            <Show when=move || ideas.with(Vec::is_empty)>
                <p class="muted">"No ideas yet. Write a note of kind idea and it shows up here."</p>
            </Show>
            {STATUS_ORDER.map(|(status, heading)| view! {
                <Show when=move || ideas.with(|v| v.iter().any(|i| i.status == status))>
                    <h4>{heading}</h4>
                    <ul class="idea-list">
                        <For
                            each=move || ideas.with(|v| v.iter().filter(|i| i.status == status).cloned().collect::<Vec<_>>())
                            key=|i| (i.id, i.timeline.len())
                            children=move |i| {
                                let id = i.id;
                                view! {
                                    <li class="card">
                                        <button type="button" class="idea-title" on:click=move |_| open(id)>{i.title}</button>
                                        <ol class="timeline">
                                            {i.timeline.into_iter().map(|t| view! { <li><span class="muted">{t.date}</span>" "{t.what}</li> }).collect_view()}
                                        </ol>
                                    </li>
                                }
                            }
                        />
                    </ul>
                </Show>
            })}
            {move || match store.digest_state.get() {
                None => view! {
                    <p class="muted small-print">"Where you diverge, assumptions and open questions appear after the first digest."</p>
                }
                .into_any(),
                Some(s) => view! {
                    <h3>"Where you diverge"</h3>
                    {if s.disagreements.is_empty() {
                        view! { <p class="muted">"Nothing open."</p> }.into_any()
                    } else {
                        s.disagreements.into_iter().map(|d| view! {
                            <div class="card diverge">
                                <p class="diverge-topic">{d.topic}</p>
                                <ul>
                                    {d.positions.into_iter().map(|p| view! { <li><strong>{p.who}": "</strong>{p.says}</li> }).collect_view()}
                                </ul>
                                {(!d.assumption.is_empty()).then(|| view! { <p class="muted">"Underneath: "{d.assumption}</p> })}
                            </div>
                        }).collect_view().into_any()
                    }}
                    <h3>"Assumptions"</h3>
                    {if s.assumptions.is_empty() {
                        view! { <p class="muted">"None recorded."</p> }.into_any()
                    } else {
                        view! {
                            <ul class="plain">
                                {s.assumptions.into_iter().map(|a| {
                                    let label = match a.status {
                                        AssumptionStatus::Untested => "untested",
                                        AssumptionStatus::Supported => "supported",
                                        AssumptionStatus::Challenged => "challenged",
                                    };
                                    view! { <li>{a.text}" "<span class="badge" data-status=label>{label}</span></li> }
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    }}
                    <h3>"Open questions"</h3>
                    {if s.open_questions.is_empty() {
                        view! { <p class="muted">"None recorded."</p> }.into_any()
                    } else {
                        view! {
                            <ul class="plain">
                                {s.open_questions.into_iter().map(|q| match q.note_id {
                                    Some(id) => view! { <li><button type="button" class="idea-title" on:click=move |_| open(id)>{q.text}</button></li> }.into_any(),
                                    None => view! { <li>{q.text}</li> }.into_any(),
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    }}
                }
                .into_any(),
            }}
        </section>
    }
}

#[component]
pub fn MapView() -> impl IntoView {
    let store = expect_context::<Store>();
    let mode = RwSignal::new(if local_get(MODE_KEY).as_deref() == Some("list") {
        Mode::List
    } else {
        Mode::Graph
    });
    let selected = RwSignal::new(None::<Id>);
    let graph = Memo::new(move |_| {
        store.log.with(|l| {
            store
                .judgments
                .with(|j| build(l, &j.values().cloned().collect::<Vec<_>>()))
        })
    });
    spawn_local(async move {
        let _ = store.refresh_judgments().await;
        let _ = store.refresh_digests().await;
    });
    let set_mode = move |m: Mode| {
        local_set(MODE_KEY, if m == Mode::List { "list" } else { "graph" });
        mode.set(m);
    };
    // A memo, so the canvas isn't remounted every time the graph changes.
    let has_notes = Memo::new(move |_| graph.with(|g| !g.nodes.is_empty()));
    view! {
        <div class="digest-head">
            <h2>"Map"</h2>
            <div class="segmented" role="group" aria-label="View">
                <button type="button" aria-pressed=move || (mode.get() == Mode::Graph).to_string() on:click=move |_| set_mode(Mode::Graph)>
                    "Graph"
                </button>
                <button type="button" aria-pressed=move || (mode.get() == Mode::List).to_string() on:click=move |_| set_mode(Mode::List)>
                    "List"
                </button>
            </div>
        </div>
        {move || match mode.get() {
            Mode::Graph if has_notes.get() => view! {
                <GraphCanvas graph=graph selected=selected />
                <NoteCard selected=selected />
            }
            .into_any(),
            Mode::Graph => view! {
                <div class="empty"><p>"The map fills in as you write notes and link them."</p></div>
            }
            .into_any(),
            Mode::List => view! { <MapList /> }.into_any(),
        }}
        <MaybeTray graph=graph />
    }
}

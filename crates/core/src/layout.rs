//! Force-directed layout (Fruchterman–Reingold with cooling), deterministic
//! and steppable so the app can spread the work across animation frames.
//!
//! Starting positions come from a hash of each note's id (or from cached
//! positions), so the same notebook always lays out the same way and the
//! map doesn't jump around between visits.

use std::collections::HashMap;

use crate::id::Id;

/// Iterations for a fresh layout and for one warm-started from the cache.
pub const COLD_ITERATIONS: u32 = 300;
pub const WARM_ITERATIONS: u32 = 80;
/// Ideal distance between connected nodes, in layout units.
const SPACING: f64 = 60.0;
const GRAVITY: f64 = 0.04;
/// Fraction of the net force applied per step.
const STEP: f64 = 0.08;

pub struct Layout {
    pub pos: Vec<[f64; 2]>,
    edges: Vec<(usize, usize)>,
    /// Larger nodes (ideas) push a little harder.
    weight: Vec<f64>,
    iteration: u32,
    total: u32,
    start_temp: f64,
}

/// Two numbers in [0, 1) from an id (FNV-1a over its text).
fn unit_pair(id: Id, salt: u8) -> (f64, f64) {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.to_string().bytes().chain([salt]) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (
        (h >> 40) as f64 / (1u64 << 24) as f64,
        (h & 0xff_ffff) as f64 / (1u64 << 24) as f64,
    )
}

impl Layout {
    /// `ids[i]` is node `i`; `edges` are index pairs; `hubs` mark ideas;
    /// `cached` holds positions from an earlier layout.
    pub fn new(
        ids: &[Id],
        hubs: &[bool],
        edges: &[(usize, usize)],
        cached: &HashMap<Id, [f64; 2]>,
    ) -> Self {
        let n = ids.len();
        let radius = SPACING * (n.max(1) as f64).sqrt();
        let mut pos: Vec<Option<[f64; 2]>> = ids.iter().map(|id| cached.get(id).copied()).collect();
        let warm = n > 0 && pos.iter().filter(|p| p.is_some()).count() * 10 >= n * 8;

        // New nodes start near a placed neighbour if they have one,
        // otherwise at a hashed spot on a disc.
        for i in 0..n {
            if pos[i].is_some() {
                continue;
            }
            let (a, b) = unit_pair(ids[i], 0);
            let neighbour = edges
                .iter()
                .filter_map(|&(x, y)| {
                    if x == i {
                        Some(y)
                    } else if y == i {
                        Some(x)
                    } else {
                        None
                    }
                })
                .find_map(|j| pos[j]);
            pos[i] = Some(match neighbour {
                Some([x, y]) => {
                    let angle = a * std::f64::consts::TAU;
                    [
                        x + SPACING * 0.5 * angle.cos(),
                        y + SPACING * 0.5 * angle.sin(),
                    ]
                }
                None => {
                    let angle = a * std::f64::consts::TAU;
                    let r = radius * b.sqrt();
                    [r * angle.cos(), r * angle.sin()]
                }
            });
        }
        Layout {
            pos: pos.into_iter().map(|p| p.unwrap_or([0.0, 0.0])).collect(),
            edges: edges.to_vec(),
            weight: hubs.iter().map(|&h| if h { 1.6 } else { 1.0 }).collect(),
            iteration: 0,
            total: if warm {
                WARM_ITERATIONS
            } else {
                COLD_ITERATIONS
            },
            start_temp: if warm { SPACING * 0.3 } else { radius * 0.2 },
        }
    }

    pub fn is_done(&self) -> bool {
        self.iteration >= self.total || self.pos.len() < 2
    }

    /// Fraction of the work done, 0 to 1.
    pub fn progress(&self) -> f64 {
        if self.pos.len() < 2 {
            1.0
        } else {
            f64::from(self.iteration) / f64::from(self.total)
        }
    }

    /// Runs up to `iterations` steps. Returns true once the layout is done.
    pub fn step(&mut self, iterations: u32) -> bool {
        let n = self.pos.len();
        let k = SPACING;
        let mut disp = vec![[0.0f64; 2]; n];
        for _ in 0..iterations {
            if self.is_done() {
                break;
            }
            let temp =
                self.start_temp * (1.0 - f64::from(self.iteration) / f64::from(self.total)) + 0.5;
            disp.iter_mut().for_each(|d| *d = [0.0, 0.0]);
            // Every pair repels.
            for i in 0..n {
                for j in (i + 1)..n {
                    let dx = self.pos[i][0] - self.pos[j][0];
                    let dy = self.pos[i][1] - self.pos[j][1];
                    let d2 = (dx * dx + dy * dy).max(0.01);
                    let f = k * k * self.weight[i] * self.weight[j] / d2;
                    disp[i][0] += dx * f;
                    disp[i][1] += dy * f;
                    disp[j][0] -= dx * f;
                    disp[j][1] -= dy * f;
                }
            }
            // Linked nodes attract.
            for &(a, b) in &self.edges {
                let dx = self.pos[a][0] - self.pos[b][0];
                let dy = self.pos[a][1] - self.pos[b][1];
                let d = (dx * dx + dy * dy).sqrt().max(0.1);
                let f = d / k;
                disp[a][0] -= dx * f;
                disp[a][1] -= dy * f;
                disp[b][0] += dx * f;
                disp[b][1] += dy * f;
            }
            // A gentle pull to the middle keeps separate clusters on screen.
            for (i, p) in self.pos.iter_mut().enumerate() {
                disp[i][0] -= p[0] * GRAVITY;
                disp[i][1] -= p[1] * GRAVITY;
                // Move a fraction of the net force, capped by the temperature:
                // jumping the full capped distance makes hubs oscillate.
                let len = (disp[i][0] * disp[i][0] + disp[i][1] * disp[i][1]).sqrt();
                if len > 0.0 {
                    let m = (len * STEP).min(temp) / len;
                    p[0] += disp[i][0] * m;
                    p[1] += disp[i][1] * m;
                }
            }
            // Only shape matters, so keep the centroid at the origin rather
            // than let the whole graph drift.
            let (cx, cy) = self
                .pos
                .iter()
                .fold((0.0, 0.0), |(x, y), p| (x + p[0], y + p[1]));
            let (cx, cy) = (cx / n as f64, cy / n as f64);
            self.pos.iter_mut().for_each(|p| {
                p[0] -= cx;
                p[1] -= cy;
            });
            self.iteration += 1;
        }
        self.is_done()
    }

    /// Runs to completion.
    pub fn finish(&mut self) {
        while !self.step(50) {}
    }

    /// The bounding box `[min_x, min_y, max_x, max_y]`.
    pub fn bounds(&self) -> [f64; 4] {
        self.pos
            .iter()
            .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: u64) -> Vec<Id> {
        (0..n)
            .map(|i| Id::new(1_761_843_481_000 + i, u128::from(i) * 7919))
            .collect()
    }

    /// Two clusters of 10 joined by one link.
    fn clusters() -> (Vec<Id>, Vec<(usize, usize)>) {
        let mut edges = Vec::new();
        for c in [0, 10] {
            for i in 1..10 {
                edges.push((c, c + i));
            }
        }
        edges.push((0, 10));
        (ids(20), edges)
    }

    fn mean(v: impl Iterator<Item = f64>) -> f64 {
        let v: Vec<f64> = v.collect();
        v.iter().sum::<f64>() / v.len() as f64
    }

    fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    }

    #[test]
    fn same_input_same_layout_whatever_the_chunking() {
        let (ids, edges) = clusters();
        let hubs = vec![false; ids.len()];
        let mut a = Layout::new(&ids, &hubs, &edges, &HashMap::new());
        a.finish();
        let mut b = Layout::new(&ids, &hubs, &edges, &HashMap::new());
        while !b.step(7) {}
        assert_eq!(a.pos, b.pos, "bit-for-bit identical");
        assert!(a.pos.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
    }

    #[test]
    fn linked_notes_end_up_closer_than_unlinked_ones() {
        let (ids, edges) = clusters();
        let mut l = Layout::new(&ids, &[false; 20], &edges, &HashMap::new());
        l.finish();
        let linked = mean(edges.iter().map(|&(a, b)| dist(l.pos[a], l.pos[b])));
        let across = mean(
            (1..10)
                .flat_map(|i| (11..20).map(move |j| (i, j)))
                .map(|(i, j)| dist(l.pos[i], l.pos[j])),
        );
        assert!(
            linked * 2.0 < across,
            "linked {linked:.1} vs across clusters {across:.1}"
        );
        let min_gap = (0..20)
            .flat_map(|i| ((i + 1)..20).map(move |j| (i, j)))
            .map(|(i, j)| dist(l.pos[i], l.pos[j]))
            .fold(f64::MAX, f64::min);
        assert!(
            min_gap > 5.0,
            "nodes don't sit on top of each other ({min_gap:.1})"
        );
    }

    #[test]
    fn a_warm_start_from_the_cache_barely_moves() {
        let (ids, edges) = clusters();
        let hubs = vec![false; 20];
        let mut first = Layout::new(&ids, &hubs, &edges, &HashMap::new());
        first.finish();
        let cache: HashMap<Id, [f64; 2]> =
            ids.iter().copied().zip(first.pos.iter().copied()).collect();
        let mut again = Layout::new(&ids, &hubs, &edges, &cache);
        assert_eq!(again.progress(), 0.0);
        again.finish();
        let moved = mean(first.pos.iter().zip(&again.pos).map(|(a, b)| dist(*a, *b)));
        assert!(moved < SPACING * 0.3, "moved {moved:.1}");
    }

    #[test]
    fn a_new_note_starts_beside_its_neighbour() {
        let ids = ids(3);
        let cache: HashMap<Id, [f64; 2]> =
            [(ids[0], [100.0, 100.0]), (ids[1], [-100.0, -100.0])].into();
        let l = Layout::new(&ids, &[false; 3], &[(1, 2)], &cache);
        assert!(dist(l.pos[2], [-100.0, -100.0]) <= SPACING * 0.5 + 1e-9);
    }

    #[test]
    fn tiny_and_large_graphs() {
        let mut empty = Layout::new(&[], &[], &[], &HashMap::new());
        assert!(empty.step(1) && empty.progress() == 1.0);
        let one = ids(1);
        assert!(Layout::new(&one, &[true], &[], &HashMap::new()).is_done());

        // 500 notes: a few chunks run and stay finite.
        let many = ids(500);
        let edges: Vec<(usize, usize)> = (1..500).map(|i| (i, i / 3)).collect();
        let mut l = Layout::new(&many, &vec![false; 500], &edges, &HashMap::new());
        let t = std::time::Instant::now();
        l.step(10);
        eprintln!(
            "500 nodes: {:.1} ms per iteration (native)",
            t.elapsed().as_secs_f64() * 100.0
        );
        assert!(l.pos.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
        let b = l.bounds();
        assert!(b[2] > b[0] && b[3] > b[1]);
    }
}

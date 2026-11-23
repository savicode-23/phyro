use super::{centerline_vertices, flatten, Coord, Vertex};
use crate::{PathNode, Shape, VectorObject, MAX_STROKE_SAMPLES};

type Key = [f32; 2];

fn value(keys: &[Key], t: f64) -> f32 {
    if keys.is_empty() {
        return 1.;
    }
    let i = keys.partition_point(|k| f64::from(k[0]) < t);
    if i == 0 {
        return keys[0][1];
    }
    if i == keys.len() {
        return keys[i - 1][1];
    }
    let [a, left] = keys[i - 1].map(f64::from);
    let [b, right] = keys[i].map(f64::from);
    (left + (right - left) * (t - a) / (b - a)) as f32
}

// Map directed intervals from the old stroke(s) into their new arc lengths.
// The new connector between joined paths gets a linear pressure transition.
fn remap(parts: &[(&[Key], f64, f64, f64)]) -> Vec<Key> {
    let total: f64 = parts.iter().map(|p| p.3).sum();
    if total <= 0. {
        return Vec::new();
    }
    let mut result = Vec::<Key>::new();
    let mut add = |t: f64, v| {
        let t = t.clamp(0., 1.) as f32;
        if let Some(previous) = result.last_mut().filter(|p| p[0] == t) {
            previous[1] = v;
        } else {
            result.push([t, v]);
        }
    };
    let mut distance = 0.;
    for &(keys, from, to, length) in parts {
        if length <= 0. || from == to {
            continue;
        }
        add(distance / total, value(keys, from));
        let mut intermediate = |[t, v]: Key| {
            let t = f64::from(t);
            if t > from.min(to) && t < from.max(to) {
                add((distance + length * (t - from) / (to - from)) / total, v);
            }
        };
        if from < to {
            keys.iter().copied().for_each(&mut intermediate);
        } else {
            keys.iter().rev().copied().for_each(&mut intermediate);
        }
        distance += length;
        add(distance / total, value(keys, to));
    }
    // A join can combine two full profiles. Remove the least visible width
    // deviations until the result fits in the same native sample budget.
    while result.len() > MAX_STROKE_SAMPLES {
        let error = |w: &[Key]| (w[1][1] - value(&[w[0], w[2]], f64::from(w[1][0]))).abs();
        let i = result
            .windows(3)
            .enumerate()
            .min_by(|a, b| error(a.1).total_cmp(&error(b.1)))
            .unwrap()
            .0;
        result.remove(i + 1);
    }
    result
}

impl VectorObject {
    fn centerline_length(&self, frame: u32) -> f64 {
        self.outline(frame)
            .and_then(|path| centerline_vertices(&path, 0.025).last().map(|v| v.distance))
            .unwrap_or(0.)
    }

    pub fn split_pressure_profile(&mut self, tail: &mut Self, frame: u32) {
        if self.stroke_profile.is_empty() {
            return;
        }
        let left = self.centerline_length(frame);
        let right = tail.centerline_length(frame);
        if left + right > 0. {
            let cut = left / (left + right);
            tail.stroke_profile = remap(&[(&self.stroke_profile, cut, 1., right)]);
            self.stroke_profile = remap(&[(&self.stroke_profile, 0., cut, left)]);
        }
    }

    /// An extension continues the tip's existing width. The original pressure
    /// stops stay on their source arc, and a closing edge joins both end widths.
    pub(crate) fn extended_pressure_profile(
        &self,
        frame: u32,
        shape: &Shape,
        original: std::ops::Range<usize>,
        closed: bool,
    ) -> Option<Vec<Key>> {
        let keys = &self.stroke_profile;
        if keys.is_empty() || keys.iter().all(|k| k[1] == keys[0][1]) {
            return Some(keys.clone());
        }
        let world = self.local_to_world_at(frame);
        let nodes = shape.contours().first()?;
        let mut walk = Vec::new();
        let budget = (131_072 / (nodes.len() + 2)).max(1);
        let error = (f64::from(self.stroke_width_at(frame)) * 0.01).clamp(0.000_01, 0.025);
        let mut length = |a: PathNode, b: PathNode| {
            let curve = [a.point, a.outgoing, b.incoming, b.point].map(|p| world(p).map(f64::from));
            walk.clear();
            walk.push(Vertex {
                point: curve[0],
                start: true,
                distance: 0.,
            });
            flatten(&mut walk, curve, error * error, budget);
            walk.last().unwrap().distance
        };
        let mut lengths = [0.; 4]; // prepended, original, appended, closing
        for (i, pair) in nodes.windows(2).enumerate() {
            let part = if i < original.start {
                0
            } else if i + 1 < original.end {
                1
            } else {
                2
            };
            lengths[part] += length(pair[0], pair[1]);
        }
        if closed {
            lengths[3] = length(*nodes.last()?, nodes[0]);
        }
        if !lengths.into_iter().all(f64::is_finite) {
            return None;
        }
        let [before, old, after, closing] = lengths;
        if before + after + closing == 0. {
            return Some(keys.clone());
        }
        let head = keys[0][1];
        let tail = keys.last()?[1];
        Some(remap(&[
            (&[[0., head], [1., head]], 0., 1., before),
            (keys, 0., 1., old),
            (&[[0., tail], [1., tail]], 0., 1., after),
            (&[[0., tail], [1., head]], 0., 1., closing),
        ]))
    }

    pub fn joined_pressure_profile(
        &self,
        other: &Self,
        frame: u32,
        reverse_a: bool,
        reverse_b: bool,
        gap: f32,
    ) -> Vec<Key> {
        if self.stroke_profile.is_empty() && other.stroke_profile.is_empty() {
            return Vec::new();
        }
        let (a, b) = if reverse_a { (1., 0.) } else { (0., 1.) };
        let (c, d) = if reverse_b { (1., 0.) } else { (0., 1.) };
        let gap_widths = [
            [0., value(&self.stroke_profile, b)],
            [1., value(&other.stroke_profile, c)],
        ];
        remap(&[
            (&self.stroke_profile, a, b, self.centerline_length(frame)),
            (&gap_widths, 0., 1., f64::from(gap)),
            (&other.stroke_profile, c, d, other.centerline_length(frame)),
        ])
    }

    /// Keep the brush widths attached to the artwork when a contour changes
    /// its winding or a closed path opens at an interior anchor.
    pub fn reorder_pressure_profile(
        &mut self,
        frame: u32,
        contour: usize,
        first_node: usize,
        reverse: bool,
    ) {
        if self.stroke_profile.is_empty() {
            return;
        }
        let world = self.local_to_world_at(frame);
        let world = |p| world(p).map(f64::from);
        let closed = !matches!(self.shape, Shape::Path { closed: false, .. });
        let mut walk = Vec::new();
        let (mut start, mut seam, mut end, mut total) = (0., 0., 0., 0.);
        for (c, nodes) in self.shape.contours().iter().enumerate() {
            if c == contour {
                start = total;
                seam = total;
            }
            for i in 0..nodes.len() - usize::from(!closed) {
                let j = (i + 1) % nodes.len();
                let curve: [Coord; 4] = [
                    nodes[i].point,
                    nodes[i].outgoing,
                    nodes[j].incoming,
                    nodes[j].point,
                ]
                .map(world);
                walk.clear();
                walk.push(Vertex {
                    point: curve[0],
                    distance: total,
                    start: false,
                });
                flatten(&mut walk, curve, 0.025 * 0.025, 256);
                total = walk.last().unwrap().distance;
                if c == contour && j == first_node && j != 0 {
                    seam = total;
                }
            }
            if c == contour {
                end = total;
            }
        }
        if total <= 0. {
            return;
        }
        let [a, b, c] = [start, seam, end].map(|x| x / total);
        let intervals = if reverse {
            [(0., a), (b, a), (c, b), (c, 1.)]
        } else {
            [(0., a), (b, c), (a, b), (c, 1.)]
        };
        self.stroke_profile = remap(
            &intervals
                .map(|(from, to)| (self.stroke_profile.as_slice(), from, to, (to - from).abs())),
        );
    }
}

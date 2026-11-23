//! A motion guide is a copy of the artist's cubic path in artboard pixels.
use crate::{PathNode, Shape, VectorObject};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionPath {
    nodes: Vec<PathNode>,
    closed: bool,
    pub align_tangent: bool,
    pub reverse: bool,
    #[serde(skip)]
    samples: OnceLock<Arc<[Sample]>>,
}

impl PartialEq for MotionPath {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes
            && self.closed == other.closed
            && self.align_tangent == other.align_tangent
            && self.reverse == other.reverse
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    point: [f32; 2],
    length: f64,
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f64 {
    (f64::from(a[0]) - f64::from(b[0])).hypot(f64::from(a[1]) - f64::from(b[1]))
}

fn midpoint(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]
}

fn add_segment(samples: &mut Vec<Sample>, p: [[f32; 2]; 4], depth: u32) {
    let chord = distance(p[0], p[3]);
    let polygon = distance(p[0], p[1]) + distance(p[1], p[2]) + distance(p[2], p[3]);
    // Bounding each input cubic's depth also bounds work on pathological paths.
    let deviation = |q: [f32; 2]| {
        if chord > 0.0 {
            ((f64::from(p[3][0] - p[0][0]) * f64::from(q[1] - p[0][1])
                - f64::from(p[3][1] - p[0][1]) * f64::from(q[0] - p[0][0]))
                / chord)
                .abs()
        } else {
            distance(q, p[0])
        }
    };
    if depth >= 7 || (polygon - chord <= 0.03 && deviation(p[1]).max(deviation(p[2])) <= 0.05) {
        if chord > 0. {
            samples.push(Sample {
                point: p[3],
                length: samples.last().unwrap().length + chord,
            });
        }
        return;
    }
    let a = midpoint(p[0], p[1]);
    let b = midpoint(p[1], p[2]);
    let c = midpoint(p[2], p[3]);
    let ab = midpoint(a, b);
    let bc = midpoint(b, c);
    let mid = midpoint(ab, bc);
    add_segment(samples, [p[0], a, ab, mid], depth + 1);
    add_segment(samples, [mid, bc, c, p[3]], depth + 1);
}

impl MotionPath {
    pub fn new(nodes: Vec<PathNode>, closed: bool) -> Result<Self, String> {
        let path = Self {
            nodes,
            closed,
            align_tangent: false,
            reverse: false,
            samples: OnceLock::new(),
        };
        path.validate()?;
        Ok(path)
    }

    pub fn from_guide(guide: &VectorObject, frame: u32) -> Result<Self, String> {
        Self::from_guide_with(guide, frame, |p| p)
    }

    /// Capture the drawn guide in the target's bind space, where motion offsets are applied.
    pub fn from_guide_for(
        guide: &VectorObject,
        frame: u32,
        target: &VectorObject,
    ) -> Result<Self, String> {
        Self::from_guide_with(guide, frame, |p| target.unrig_point_at(p, frame))
    }

    fn from_guide_with(
        guide: &VectorObject,
        frame: u32,
        map: impl Fn([f32; 2]) -> [f32; 2],
    ) -> Result<Self, String> {
        let evaluated = guide.evaluated_shape_at(frame);
        let Shape::Path { nodes, closed } = evaluated.as_ref() else {
            return Err("Choose a single path as the motion guide.".into());
        };
        let capture = |p| map(guide.local_to_world(p, frame));
        Self::new(
            nodes
                .iter()
                .map(|n| PathNode {
                    handle_style: n.handle_style,
                    point: capture(n.point),
                    incoming: capture(n.incoming),
                    outgoing: capture(n.outgoing),
                })
                .collect(),
            *closed,
        )
    }

    pub fn nodes(&self) -> &[PathNode] {
        &self.nodes
    }
    pub fn closed(&self) -> bool {
        self.closed
    }
    pub fn set_node(&mut self, index: usize, node: PathNode) -> Result<(), String> {
        let Some(old) = self.nodes.get(index).copied() else {
            return Err("Unknown guide node.".into());
        };
        self.nodes[index] = node;
        self.samples.take();
        if let Err(err) = self.validate() {
            self.nodes[index] = old;
            self.samples.take();
            return Err(err);
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(2..=512).contains(&self.nodes.len())
            || self.nodes.iter().any(|n| {
                [n.point, n.incoming, n.outgoing]
                    .into_iter()
                    .flatten()
                    .any(|v| !v.is_finite() || v.abs() > 10_000_000.)
            })
        {
            return Err(
                "Motion paths require 2–512 finite nodes within the editable coordinate range."
                    .into(),
            );
        }
        if self.samples().len() < 2 {
            return Err("The motion guide has no length.".into());
        }
        Ok(())
    }

    fn samples(&self) -> &[Sample] {
        self.samples.get_or_init(|| {
            let mut samples = vec![Sample {
                point: self.nodes[0].point,
                length: 0.,
            }];
            for (a, b) in self
                .nodes
                .iter()
                .zip(self.nodes.iter().cycle().skip(1))
                .take(self.nodes.len() - usize::from(!self.closed))
            {
                add_segment(&mut samples, [a.point, a.outgoing, b.incoming, b.point], 0);
            }
            samples.into()
        })
    }

    /// Absolute guide position and travel heading at normalized arc length.
    pub fn sample(&self, progress: f32) -> ([f32; 2], f32) {
        let samples = self.samples();
        if samples.len() < 2 {
            return (samples[0].point, 0.);
        }
        let progress = progress.clamp(0., 1.);
        let d = f64::from(if self.reverse {
            1. - progress
        } else {
            progress
        }) * samples.last().unwrap().length;
        let i = samples
            .partition_point(|p| p.length < d)
            .clamp(1, samples.len() - 1);
        let a = samples[i - 1];
        let b = samples[i];
        let t = ((d - a.length) / (b.length - a.length)) as f32;
        let dx = b.point[0] - a.point[0];
        let dy = b.point[1] - a.point[1];
        let angle = dy.atan2(dx).to_degrees() + if self.reverse { 180. } else { 0. };
        ([a.point[0] + dx * t, a.point[1] + dy * t], angle)
    }
}

impl VectorObject {
    pub fn motion_offset_at(&self, frame: u32) -> [f32; 2] {
        self.motion.as_ref().map_or([0.; 2], |m| {
            let p = m.sample(self.progress_at(frame)).0;
            [p[0] - m.nodes[0].point[0], p[1] - m.nodes[0].point[1]]
        })
    }
    pub fn motion_heading_at(&self, frame: u32) -> f32 {
        self.motion
            .as_ref()
            .filter(|m| m.align_tangent)
            .map_or(0., |m| m.sample(self.progress_at(frame)).1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distance_progress_crosses_unequal_segments_and_reverse_faces_back() {
        let mut guide = MotionPath::new(
            vec![
                PathNode::corner([1., 1.]),
                PathNode::corner([3., 1.]),
                PathNode::corner([3., 9.]),
            ],
            false,
        )
        .unwrap();
        assert_eq!(guide.sample(0.).0, [1., 1.]);
        assert_eq!(guide.sample(0.5), ([3., 4.], 90.));
        guide.reverse = true;
        assert_eq!(guide.sample(0.).0, [3., 9.]);
        assert_eq!(guide.sample(0.5), ([3., 4.], 270.));
    }
    #[test]
    fn modifying_a_curve_rebuilds_distance_cache_and_rejects_invalid_edits() {
        let mut a = PathNode::corner([0., 0.]);
        a.outgoing = [0., 10.];
        let mut b = PathNode::corner([10., 0.]);
        b.incoming = [10., 10.];
        let mut path = MotionPath::new(vec![a, b], false).unwrap();
        assert!((path.sample(0.5).0[1] - 7.5).abs() < 0.06);
        a.outgoing = [0., 0.];
        path.set_node(0, a).unwrap();
        let after = path.sample(0.5);
        assert!(after.0[1] < 7.);
        let mut bad = a;
        bad.point[0] = f32::INFINITY;
        assert!(path.set_node(0, bad).is_err());
        assert_eq!(after, path.sample(0.5));
    }
}

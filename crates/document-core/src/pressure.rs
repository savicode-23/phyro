use crate::{bezier, VectorObject, MAX_STROKE_SAMPLES};
use tiny_skia_path::{Path, PathSegment, Point};
mod edit;
mod outline;
use outline::{Disk, Outline};

type Coord = [f64; 2];

pub(super) fn valid_profile(profile: &[[f32; 2]]) -> bool {
    profile.is_empty()
        || (2..=MAX_STROKE_SAMPLES).contains(&profile.len())
            && profile[0][0] == 0.
            && profile.last().unwrap()[0] == 1.
            && profile.iter().all(|p| (0. ..=1.).contains(&p[1]))
            && profile.windows(2).all(|w| w[0][0] < w[1][0])
}

#[derive(Clone, Copy)]
struct Vertex {
    point: Coord,
    distance: f64,
    start: bool,
}

fn coord(p: Point) -> Coord {
    [p.x.into(), p.y.into()]
}

fn mix(a: Coord, b: Coord, t: f64) -> Coord {
    std::array::from_fn(|axis| a[axis] + (b[axis] - a[axis]) * t)
}

fn line(points: &mut Vec<Vertex>, point: Coord) {
    let previous = points.last().unwrap();
    let length = (point[0] - previous.point[0]).hypot(point[1] - previous.point[1]);
    if length > 0. {
        points.push(Vertex {
            point,
            distance: previous.distance + length,
            start: false,
        });
    }
}

fn flatten(points: &mut Vec<Vertex>, curve: [Coord; 4], error_sq: f64, budget: usize) {
    let [a, b, c, d] = curve;
    if budget < 2
        || bezier::distance_sq(b, a, d) <= error_sq && bezier::distance_sq(c, a, d) <= error_sq
    {
        line(points, d);
    } else {
        let [left, right] = bezier::split_cubic(curve);
        flatten(points, left, error_sq, budget / 2);
        flatten(points, right, error_sq, budget - budget / 2);
    }
}

fn centerline_vertices(path: &Path, error: f64) -> Vec<Vertex> {
    let mut points = Vec::with_capacity(path.verbs().len());
    // Reserve work for every authored segment, including on enormous transformed curves.
    let budget = (131_072 / path.verbs().len().max(1)).max(1);
    let mut start = [0.; 2];
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                start = coord(p);
                points.push(Vertex {
                    point: start,
                    distance: points.last().map_or(0., |v: &Vertex| v.distance),
                    start: true,
                });
            }
            PathSegment::LineTo(p) => line(&mut points, coord(p)),
            PathSegment::QuadTo(c, p) => {
                let a = points.last().unwrap().point;
                flatten(
                    &mut points,
                    bezier::quadratic(a, coord(c), coord(p)),
                    error * error,
                    budget,
                );
            }
            PathSegment::CubicTo(b, c, d) => {
                let a = points.last().unwrap().point;
                flatten(
                    &mut points,
                    [a, coord(b), coord(c), coord(d)],
                    error * error,
                    budget,
                );
            }
            PathSegment::Close => line(&mut points, start),
        }
    }
    points
}

impl VectorObject {
    /// Round variable-width brush region in the same coordinate system as the evaluated centerline.
    /// Fill this path once with nonzero winding. Uniform legacy strokes have no pressure outline.
    pub fn pressure_outline(&self, centerline: &Path, frame: u32) -> Option<Path> {
        let keys = &self.stroke_profile;
        let width = self.stroke_width_at(frame);
        if keys.is_empty() || !valid_profile(keys) || width <= 0. {
            return None;
        }
        if keys.iter().all(|k| k[1] == keys[0][1]) {
            let width = width * keys[0][1];
            if width == 0. {
                return None;
            }
            return centerline.stroke(
                &tiny_skia_path::Stroke {
                    width,
                    line_cap: tiny_skia_path::LineCap::Round,
                    line_join: tiny_skia_path::LineJoin::Round,
                    ..Default::default()
                },
                1.,
            );
        }
        let r = f64::from(width) * 0.5;
        let points = centerline_vertices(centerline, (r * 0.02).clamp(0.000_01, 0.025));
        let total = points.last()?.distance;
        let mut path = Outline::default();
        let mut key = 0;
        let mut previous: Option<Vertex> = None;
        for vertex in points {
            let t = if total == 0. {
                0.
            } else {
                f64::from((vertex.distance / total) as f32)
            };
            while key + 1 < keys.len() && f64::from(keys[key + 1][0]) <= t {
                key += 1;
                // Include a pressure peak even on a straight, two-anchor centerline.
                if let Some(last) = previous {
                    let distance = f64::from(keys[key][0]) * total;
                    if !vertex.start && f64::from(keys[key][0]) < t && distance > last.distance {
                        let at = (distance - last.distance) / (vertex.distance - last.distance);
                        let disk = Disk {
                            p: mix(last.point, vertex.point, at),
                            r: r * f64::from(keys[key][1]),
                        };
                        path.push(disk);
                    }
                }
            }
            let factor = if let Some(next) = keys.get(key + 1) {
                let a = keys[key].map(f64::from);
                let b = next.map(f64::from);
                a[1] + (b[1] - a[1]) * ((t - a[0]) / (b[0] - a[0])).clamp(0., 1.)
            } else {
                f64::from(keys[key][1])
            };
            let disk = Disk {
                p: vertex.point,
                r: r * factor,
            };
            if vertex.start {
                path.begin(disk);
            } else {
                path.push(disk);
            }
            previous = Some(vertex);
        }
        path.finish()
    }
}

#[cfg(test)]
mod tests;

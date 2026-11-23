use super::Coord;
use std::f64::consts::{FRAC_PI_2, TAU};
use tiny_skia_path::{Path, PathBuilder};

#[derive(Clone, Copy)]
pub(super) struct Disk {
    pub p: Coord,
    pub r: f64,
}

fn add(p: Coord, v: Coord, k: f64) -> Coord {
    [p[0] + v[0] * k, p[1] + v[1] * k]
}
fn delta(a: Coord, b: Coord) -> Coord {
    [b[0] - a[0], b[1] - a[1]]
}
fn cross(a: Coord, b: Coord) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn dot(a: Coord, b: Coord) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn line(path: &mut PathBuilder, p: Coord) {
    path.line_to(p[0] as f32, p[1] as f32);
}

fn arc(path: &mut PathBuilder, disk: Disk, from: Coord, to: Coord) {
    if disk.r == 0. || from == to {
        return;
    }
    let sweep = cross(from, to).atan2(dot(from, to)).rem_euclid(TAU);
    let max_step = (240. / disk.r).powf(1. / 6.).clamp(0.18, FRAC_PI_2);
    let steps = (sweep / max_step).ceil() as usize;
    if steps == 0 {
        return;
    }
    let step = sweep / steps as f64;
    let (sin, cos) = step.sin_cos();
    let k = (4. / 3.) * (step * 0.25).tan();
    let mut n = from;
    for i in 1..=steps {
        let m = if i == steps {
            to
        } else {
            [cos * n[0] - sin * n[1], sin * n[0] + cos * n[1]]
        };
        let a = add(disk.p, [n[0] - k * n[1], n[1] + k * n[0]], disk.r);
        let b = add(disk.p, [m[0] + k * m[1], m[1] - k * m[0]], disk.r);
        let end = add(disk.p, m, disk.r);
        path.cubic_to(
            a[0] as f32,
            a[1] as f32,
            b[0] as f32,
            b[1] as f32,
            end[0] as f32,
            end[1] as f32,
        );
        n = m;
    }
}

#[derive(Clone, Copy)]
struct Span {
    a: Disk,
    b: Disk,
    top: Coord,
    bottom: Coord,
}

impl Span {
    // The convex hull of the endpoint disks is the exact swept brush for
    // a straight segment with linearly varying pressure.
    fn new(a: Disk, b: Disk) -> Option<Self> {
        let [dx, dy] = delta(a.p, b.p);
        let length = dx.hypot(dy);
        let dr = b.r - a.r;
        if length <= dr.abs() || a.r == 0. && b.r == 0. {
            return None;
        }
        let (x, y) = (dx / length, dy / length);
        let slope = dr / length;
        let h = (1. - slope * slope).sqrt();
        Some(Self {
            a,
            b,
            top: [-slope * x + h * y, -slope * y - h * x],
            bottom: [-slope * x - h * y, -slope * y + h * x],
        })
    }
    fn reversed(self) -> Self {
        Self {
            a: self.b,
            b: self.a,
            top: self.bottom,
            bottom: self.top,
        }
    }
}

#[derive(Clone, Copy)]
enum Join {
    Point(Coord),
    Round { disk: Disk, from: Coord, to: Coord },
}

impl Join {
    fn between(before: Span, after: Span) -> Option<Self> {
        let disk = before.b;
        let from = before.top;
        let to = after.top;
        let p = add(disk.p, from, disk.r);
        let q = add(disk.p, to, disk.r);
        if disk.r == 0. || (dot(from, to) > 0. && cross(from, to).abs() < 1e-9) {
            return Some(Self::Point(p));
        }
        if cross(from, to) > 0. {
            return Some(Self::Round { disk, from, to });
        }
        // Inside corners intersect their two finite tangent lines. Limit the
        // trim to half of each edge so trims at opposite ends cannot cross.
        let incoming = delta(add(before.a.p, from, before.a.r), p);
        let outgoing = delta(q, add(after.b.p, to, after.b.r));
        let det = cross(incoming, outgoing);
        if det == 0. {
            return None;
        }
        let gap = delta(p, q);
        let left = cross(gap, outgoing) / det;
        let right = cross(gap, incoming) / det;
        ((-0.5..=0.).contains(&left) && (0. ..=0.5).contains(&right))
            .then(|| Self::Point(add(p, incoming, left)))
    }
    fn draw(self, path: &mut PathBuilder) {
        match self {
            Self::Point(p) => line(path, p),
            Self::Round { disk, from, to } => {
                line(path, add(disk.p, from, disk.r));
                arc(path, disk, from, to);
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Outline {
    path: PathBuilder,
    previous: Option<Disk>,
    has_length: bool,
    first: Option<Span>,
    last: Option<Span>,
    joins: Vec<(Join, Join)>,
}

impl Outline {
    pub fn begin(&mut self, disk: Disk) {
        self.end();
        self.previous = Some(disk);
    }
    pub fn push(&mut self, disk: Disk) {
        let Some(previous) = self.previous.replace(disk) else {
            return;
        };
        self.has_length = true;
        let Some(span) = Span::new(previous, disk) else {
            self.flush();
            let larger = if previous.r > disk.r { previous } else { disk };
            if larger.r > 0. {
                self.path
                    .push_circle(larger.p[0] as f32, larger.p[1] as f32, larger.r as f32);
            }
            return;
        };
        if let Some(before) = self.last {
            let join =
                Join::between(before, span).zip(Join::between(span.reversed(), before.reversed()));
            if let Some(join) = join {
                self.joins.push(join);
            } else {
                self.flush();
            }
        }
        self.first.get_or_insert(span);
        self.last = Some(span);
    }
    fn flush(&mut self) {
        let (Some(first), Some(last)) = (self.first.take(), self.last.take()) else {
            return;
        };
        let p = &mut self.path;
        let [x, y] = add(first.a.p, first.top, first.a.r);
        p.move_to(x as f32, y as f32);
        for &(top, _) in &self.joins {
            top.draw(p);
        }
        line(p, add(last.b.p, last.top, last.b.r));
        arc(p, last.b, last.top, last.bottom);
        for &(_, bottom) in self.joins.iter().rev() {
            bottom.draw(p);
        }
        line(p, add(first.a.p, first.bottom, first.a.r));
        arc(p, first.a, first.bottom, first.top);
        p.close();
        self.joins.clear();
    }
    fn end(&mut self) {
        self.flush();
        if !self.has_length {
            if let Some(disk) = self.previous.filter(|d| d.r > 0.) {
                self.path
                    .push_circle(disk.p[0] as f32, disk.p[1] as f32, disk.r as f32);
            }
        }
        self.has_length = false;
        self.previous = None;
    }
    pub fn finish(mut self) -> Option<Path> {
        self.end();
        self.path.finish()
    }
}

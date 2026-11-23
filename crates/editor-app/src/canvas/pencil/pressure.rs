use document_core::MAX_STROKE_SAMPLES;
use eframe::egui::{self, Color32, Painter, Pos2};

/// Measured along the filtered centerline, independently of its fitted Bézier
/// anchors. A straight stroke can still contain many pressure changes.
pub(super) struct Track {
    distances: Vec<f32>,
    widths: Vec<[f32; 2]>,
    force: Option<f32>,
}

impl Track {
    pub(super) fn new(force: Option<f32>) -> Self {
        let mut track = Self {
            distances: vec![0.],
            widths: Vec::new(),
            force: None,
        };
        track.set_force(force);
        track
    }

    pub(super) fn set_force(&mut self, force: Option<f32>) {
        if let Some(f) = force.filter(|v| v.is_finite()) {
            let width = 0.12 + 0.88 * f.clamp(0., 1.);
            self.force = Some(width);
            if self.widths.is_empty() {
                self.widths.push([0., width]);
            }
        }
    }

    pub(super) fn push(&mut self, step: f32) {
        let distance = *self.distances.last().unwrap() + step;
        self.distances.push(distance);
        if let Some(width) = self.force {
            if self.widths.len() >= MAX_STROKE_SAMPLES * 16 {
                self.widths = fit(&self.widths, MAX_STROKE_SAMPLES * 8);
            }
            append(&mut self.widths, distance, width);
        }
    }

    pub(super) fn retain_points(&mut self, keep: &[usize]) {
        self.distances = keep.iter().map(|&i| self.distances[i]).collect();
    }
    pub(super) fn has_force(&self) -> bool {
        self.force.is_some()
    }
    pub(super) fn tip_width(&self) -> f32 {
        self.force.unwrap_or(1.)
    }
    pub(super) fn at_point(&self, index: usize) -> f32 {
        interpolate(&self.widths, self.distances[index])
    }

    pub(super) fn finish(&self, last: usize, tail: f32, closed: bool) -> Vec<[f32; 2]> {
        let Some(tip) = self.force else {
            return Vec::new();
        };
        let cut = self.distances[last];
        let total = cut + tail;
        if !total.is_finite() || total <= 0. {
            return Vec::new();
        }
        let mut widths = self.widths[..self.widths.partition_point(|p| p[0] < cut)].to_vec();
        append(&mut widths, cut, self.at_point(last));
        // Loop snapping may discard the wobble after the nearest return to
        // the beginning. Join the surviving widths across that same seam.
        let end = if closed { self.widths[0][1] } else { tip };
        append(&mut widths, total, end);
        let mut result = Vec::new();
        for [distance, width] in fit(&widths, MAX_STROKE_SAMPLES) {
            append(&mut result, distance / total, width);
        }
        result
    }
}

fn append(points: &mut Vec<[f32; 2]>, distance: f32, width: f32) {
    if let Some(last) = points.last_mut().filter(|p| p[0] == distance) {
        last[1] = width;
    } else {
        points.push([distance, width]);
    }
}

fn interpolate(points: &[[f32; 2]], distance: f32) -> f32 {
    let next = points.partition_point(|p| p[0] < distance);
    match next {
        0 => points[0][1],
        i if i == points.len() => points[i - 1][1],
        i => {
            let [a, x] = points[i - 1];
            let [b, y] = points[i];
            x + (y - x) * ((distance - a) / (b - a))
        }
    }
}

// Keep a run while a straight interpolation from its first sample is
// within 1% of the intermediate widths. Pathological jitter is resampled
// on an evenly spaced distance grid once the pressure key budget is full.
fn fit(points: &[[f32; 2]], budget: usize) -> Vec<[f32; 2]> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut result = vec![points[0]];
    let (mut low, mut high) = (f32::NEG_INFINITY, f32::INFINITY);
    let mut start = points[0];
    for (i, p) in points.iter().enumerate().skip(1) {
        let slope = (p[1] - start[1]) / (p[0] - start[0]);
        if slope < low || slope > high {
            start = points[i - 1];
            result.push(start);
            (low, high) = (f32::NEG_INFINITY, f32::INFINITY);
        }
        let dy = p[1] - start[1];
        let dx = p[0] - start[0];
        low = low.max((dy - 0.01) / dx);
        high = high.min((dy + 0.01) / dx);
    }
    let last = *points.last().unwrap();
    result.push(last);
    if result.len() > budget {
        result = (0..budget)
            .map(|i| {
                if i == budget - 1 {
                    return last;
                }
                let distance =
                    points[0][0] + (last[0] - points[0][0]) * (i as f32 / (budget - 1) as f32);
                [distance, interpolate(points, distance)]
            })
            .collect();
    }
    result
}

pub(super) fn paint(
    painter: &Painter,
    points: &[Pos2],
    widths: &[f32],
    color: Color32,
    closed: bool,
) {
    if points.len() < 2 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    let feather = 1. / painter.ctx().pixels_per_point();
    let last = points.len() - 1;
    for (i, (&p, &width)) in points.iter().zip(widths).enumerate() {
        let before = if closed && i == 0 {
            last - 1
        } else {
            i.saturating_sub(1)
        };
        let after = if closed && i == last {
            1
        } else {
            (i + 1).min(last)
        };
        let incoming = (p - points[before]).normalized();
        let outgoing = (points[after] - p).normalized();
        let tangent = (incoming + outgoing).normalized();
        let tangent = if tangent == egui::Vec2::ZERO {
            incoming
        } else {
            tangent
        };
        let normal = egui::vec2(-tangent.y, tangent.x);
        let outer = 0.5 * (width + feather);
        let inner = (0.5 * (width - feather)).max(0.);
        let ink = color.gamma_multiply((width / feather).min(1.));
        for (offset, color) in [
            (outer, Color32::TRANSPARENT),
            (inner, ink),
            (-inner, ink),
            (-outer, Color32::TRANSPARENT),
        ] {
            mesh.colored_vertex(p + normal * offset, color);
        }
        if i > 0 {
            let start = (i as u32 - 1) * 4;
            for k in start..start + 3 {
                mesh.add_triangle(k, k + 4, k + 1);
                mesh.add_triangle(k + 1, k + 4, k + 5);
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pencil_pressure_limits_noisy_long_strokes_and_preserves_exact_endpoints() {
        let mut track = Track::new(Some(-7.));
        for i in 0..30_000 {
            track.set_force(Some((i % 11) as f32 / 10.));
            track.push(0.5);
            assert!(track.widths.len() <= MAX_STROKE_SAMPLES * 16);
        }
        track.set_force(Some(3.));
        track.set_force(Some(f32::NAN));
        let profile = track.finish(30_000, 1., false);
        assert!((3..=MAX_STROKE_SAMPLES).contains(&profile.len()));
        assert_eq!(profile[0], [0., 0.12]);
        assert_eq!(profile.last().copied(), Some([1., 1.]));
        assert!(profile.windows(2).all(|w| w[0][0] < w[1][0]));
        let mut unavailable = Track::new(None);
        unavailable.set_force(Some(f32::INFINITY));
        unavailable.push(10.);
        assert!(unavailable.finish(1, 0., false).is_empty());
    }
}

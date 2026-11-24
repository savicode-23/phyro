use super::{is_editable, paths, Action, Canvas, CanvasStyle, View};
use document_core::{path_edit::HandleStyle, Document, PathNode};
use eframe::egui::{self, Color32, Event, Painter, Pos2, Response, Ui, Vec2};
use std::cell::OnceCell;
use std::sync::Arc;
mod fit;
mod pressure;

// A single extremely long stylus gesture should have bounded memory and paint cost.
const MAX_SAMPLES: usize = 16_384;
const PREVIEW_POINTS: usize = 4096;

pub(super) struct Stroke {
    source: Source,
    pressure: Option<pressure::Track>,
    points: Vec<Pos2>,
    tip: Pos2,
    min_spacing: f32,
    tolerance: f32,
    filter_radius: f32,
    scale: f32,
    extent: f32,
    // The approach inside the snap radius can change while the rest of the loop stays fixed.
    loop_body: OnceCell<Option<usize>>,
}

impl Stroke {
    fn new(start: Pos2, scale: f32, smoothing: u8) -> Self {
        let percent = f32::from(smoothing.min(200)) / 100.;
        Self {
            source: Source::Pointer,
            pressure: None,
            points: vec![start],
            tip: start,
            min_spacing: (0.55 + 0.5 * percent) / scale,
            // Strong smoothing filters hand jitter while the fit stays close
            // to the visible stroke, including small deliberate changes of direction.
            tolerance: (0.4 + 1.5 * percent) / scale,
            filter_radius: (3. * percent + 6. * (percent - 1.).max(0.)) / scale,
            scale,
            extent: 0.,
            loop_body: OnceCell::new(),
        }
    }

    fn push(&mut self, p: Pos2) {
        if !p.is_finite() {
            return;
        }
        self.tip = p;
        self.extent = self.extent.max(p.distance(self.points[0]));
        let last = *self.points.last().unwrap();
        let distance = last.distance(p);
        if distance < self.min_spacing {
            return;
        }
        if self.points.len() >= MAX_SAMPLES {
            self.loop_body.take();
            // Keep detailed turns when possible. A worst-case scribble still has
            // room for the rest of the gesture instead of truncating its end.
            let mut keep = simplify_indices(&self.points, self.tolerance);
            if keep.len() > MAX_SAMPLES * 3 / 4 {
                let end = *keep.last().unwrap();
                keep = keep.into_iter().step_by(2).collect();
                if keep.last() != Some(&end) {
                    keep.push(end);
                }
                self.min_spacing *= 1.5;
            }
            if let Some(pressure) = &mut self.pressure {
                pressure.retain_points(&keep);
            }
            self.points = keep.into_iter().map(|i| self.points[i]).collect();
        }
        // Bound the filter's lag in screen space even when the mouse jumps.
        // Sparse events, e.g. the vertex of a fast drawn corner, carry more
        // positional information than closely spaced jitter around the tip.
        let lag = (self.filter_radius / (distance + self.filter_radius)).powi(2) * 0.75;
        let sample = p.lerp(last, lag);
        if sample.distance(self.points[0]) > paths::CLOSE_RADIUS / self.scale {
            self.loop_body.take();
        }
        self.points.push(sample);
        if let Some(pressure) = &mut self.pressure {
            pressure.push(sample.distance(last));
        }
    }

    fn closing_prefix(&self) -> Option<usize> {
        let start = self.points[0];
        let radius = paths::CLOSE_RADIUS / self.scale;
        if self.extent <= radius || self.tip.distance(start) > radius {
            return None;
        }
        let body = (*self.loop_body.get_or_init(|| {
            let end = self
                .points
                .iter()
                .rposition(|p| p.distance_sq(start) > radius * radius)?
                + 1;
            let arc = &self.points[..end];
            // A straight out-and-back stroke can travel far and still have no loop.
            let farthest = arc
                .iter()
                .max_by(|a, b| a.distance_sq(start).total_cmp(&b.distance_sq(start)))?;
            let axis = (*farthest - start).normalized();
            let tolerance = self.tolerance.min(self.extent * 0.25);
            arc.iter()
                .any(|p| {
                    let leg = *p - start;
                    (axis.x * leg.y - axis.y * leg.x).abs() > tolerance
                })
                .then_some(end)
        }))?;
        // Follow the traced curve all the way in. Moving away from its closest
        // approach inside the ring should not leave a little hook at the seam.
        let mut end = self.points.len();
        let mut nearest = self.tip.distance_sq(start);
        for (index, p) in self.points[body..].iter().enumerate() {
            let distance = p.distance_sq(start);
            if distance < nearest {
                nearest = distance;
                end = body + index + 1;
            }
        }
        Some(end)
    }

    fn finish(mut self) -> Option<(Vec<PathNode>, bool, Vec<[f32; 2]>)> {
        if self.extent * self.scale < 2.5 {
            return None;
        }
        let closing = self.closing_prefix();
        if let Some(end) = closing {
            self.points.truncate(end);
            self.tip = self.points[0];
        }
        if self.points.len() > 1
            && self.points.last().unwrap().distance(self.tip) < self.min_spacing
        {
            self.points.pop();
        }
        let profile = self.pressure.as_ref().map_or_else(Vec::new, |p| {
            let last = self.points.len() - 1;
            p.finish(
                last,
                self.points[last].distance(self.tip),
                closing.is_some(),
            )
        });
        if self.points.last() != Some(&self.tip) {
            self.points.push(self.tip);
        }
        let closed = closing.is_some();
        let nodes = fit::nodes(&self.points, closed, self.tolerance, self.scale)?;
        Some((nodes, closed, profile))
    }

    pub(super) fn paint(&self, painter: &Painter, view: View, style: &CanvasStyle) {
        let closing = self.closing_prefix();
        let samples = &self.points[..closing.unwrap_or(self.points.len())];
        let step = samples.len().div_ceil(PREVIEW_POINTS);
        let mut points: Vec<_> = samples
            .iter()
            .step_by(step)
            .map(|&p| view.screen(p))
            .collect();
        points.push(view.screen(if closing.is_some() {
            samples[0]
        } else {
            self.tip
        }));
        let preview_color = style.stroke_gradient.as_deref().map_or(style.stroke, |g| {
            g.stops
                .iter()
                .find(|s| s.color[3] > 0)
                .map_or([0; 4], |s| s.color)
        });
        let has_stroke = !style.no_stroke && style.stroke_width > 0. && preview_color[3] > 0;
        let (width, color) = if has_stroke {
            let [r, g, b, a] = preview_color;
            (
                (style.stroke_width * view.scale).max(1.),
                Color32::from_rgba_unmultiplied(r, g, b, a),
            )
        } else {
            (1.5, Color32::from_rgb(94, 214, 183))
        };
        if let Some(profile) = self
            .pressure
            .as_ref()
            .filter(|p| p.has_force() && has_stroke)
        {
            let mut widths: Vec<_> = (0..samples.len())
                .step_by(step)
                .map(|i| width * profile.at_point(i))
                .collect();
            widths.push(
                width
                    * if closing.is_some() {
                        profile.at_point(0)
                    } else {
                        profile.tip_width()
                    },
            );
            pressure::paint(painter, &points, &widths, color, closing.is_some());
        } else {
            painter.add(egui::Shape::line(points, egui::Stroke::new(width, color)));
        }
        if closing.is_some() {
            super::overlay::close_ring(painter, view.screen(samples[0]));
        }
    }
}

// Iterative Ramer–Douglas–Peucker. On a badly unbalanced long interval,
// split in its middle and check both children independently. This bounds
// the depth on adversarial input and still tests every retained chord.
fn simplify_indices(points: &[Pos2], tolerance: f32) -> Vec<usize> {
    if points.len() < 3 {
        return (0..points.len()).collect();
    }
    let last = points.len() - 1;
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[last] = true;
    let mut spans = vec![(0, last)];
    while let Some((first, last)) = spans.pop() {
        let origin = points[first];
        let chord = points[last] - origin;
        let length2 = chord.length_sq();
        let mut farthest = tolerance * tolerance;
        let mut split = first;
        for (offset, &p) in points[first + 1..last].iter().enumerate() {
            let t = if length2 > 0. {
                ((p - origin).dot(chord) / length2).clamp(0., 1.)
            } else {
                0.
            };
            let error = p.distance_sq(origin + t * chord);
            if error > farthest {
                farthest = error;
                split = first + 1 + offset;
            }
        }
        if split != first {
            let width = last - first;
            // A closed loop needs its farthest point to separate its two arcs.
            if length2 > 0. && width > 96 && (split - first).min(last - split) < width / 8 {
                split = first + width / 2;
            }
            keep[split] = true;
            if split - first > 1 {
                spans.push((first, split));
            }
            if last - split > 1 {
                spans.push((split, last));
            }
        }
    }
    keep.into_iter()
        .enumerate()
        .filter_map(|(i, keep)| keep.then_some(i))
        .collect()
}

pub(crate) struct FinishedStroke {
    nodes: Vec<PathNode>,
    closed: bool,
    style: CanvasStyle,
    profile: Vec<[f32; 2]>,
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Pointer,
    Touch(egui::TouchDeviceId, egui::TouchId),
}

enum Input {
    Press(Source, Pos2, Option<f32>),
    Move(Source, Pos2, Option<f32>),
    Release(Source, Pos2, Option<f32>),
    Cancel(Option<Source>),
}

fn input_events(events: &[Event]) -> Vec<Input> {
    use egui::TouchPhase::{Cancel, End, Move, Start};
    // egui-winit mirrors the primary touch into pointer events for ordinary
    // widgets. Ignore those copies only; independent mouse samples still work.
    let mut mirror = None;
    events
        .iter()
        .filter_map(|e| match *e {
            Event::Touch {
                device_id,
                id,
                phase,
                pos,
                force,
            } => {
                mirror = Some((pos, phase));
                let source = Source::Touch(device_id, id);
                Some(match phase {
                    Start => Input::Press(source, pos, force),
                    Move => Input::Move(source, pos, force),
                    End => Input::Release(source, pos, force),
                    Cancel => Input::Cancel(Some(source)),
                })
            }
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                ..
            } => {
                let duplicate = mirror.take().is_some_and(|(at, phase)| {
                    at == pos && matches!((phase, pressed), (Start, true) | (End | Cancel, false))
                });
                if duplicate {
                    None
                } else if pressed {
                    Some(Input::Press(Source::Pointer, pos, None))
                } else {
                    Some(Input::Release(Source::Pointer, pos, None))
                }
            }
            Event::PointerMoved(p) => {
                match mirror {
                    Some((at, Start)) if at == p => None, // A press follows this copy.
                    Some((at, Move)) if at == p => {
                        mirror = None;
                        None
                    }
                    _ => {
                        mirror = None;
                        Some(Input::Move(Source::Pointer, p, None))
                    }
                }
            }
            Event::PointerGone => Some(Input::Cancel(Some(Source::Pointer))),
            Event::WindowFocused(false)
            | Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                ..
            } => Some(Input::Cancel(None)),
            _ => None,
        })
        .collect()
}

impl FinishedStroke {
    pub(crate) fn apply(mut self, doc: &mut Document, selection: &mut Vec<u64>) -> bool {
        if let Some(id) = paths::finish_pen(&mut self.nodes, self.closed, doc, &self.style) {
            doc.objects
                .last_mut()
                .expect("finish_pen appends the stroke")
                .stroke_profile = self.profile;
            *selection = vec![id];
            true
        } else {
            false
        }
    }
}

impl Canvas {
    // A repaint can contain multiple complete gestures. Give the editor each
    // additional path in order so its normal undo transaction covers one stroke.
    pub(crate) fn take_extra_pencil_strokes(&mut self) -> Vec<FinishedStroke> {
        std::mem::take(&mut self.extra_pencil_strokes)
    }

    pub(super) fn pencil_input(
        &mut self,
        ui: &Ui,
        response: &Response,
        doc: &mut Arc<Document>,
        selection: &mut Vec<u64>,
        style: &CanvasStyle,
        view: View,
    ) -> bool {
        if !is_editable(doc, style.layer) {
            self.pencil = None;
            return false;
        }
        if response.hovered() || self.pencil.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        // A tablet can send many locations between redraws, or even a complete
        // stroke. Keep native order instead of sampling egui's final cursor only.
        let events = ui.input(|i| input_events(&i.events));
        let mut committed = false;
        for event in events {
            match event {
                Input::Press(source, pos, force) => {
                    if self.pencil.is_none()
                        && response.interact_rect.contains(pos)
                        && ui.ctx().layer_id_at(pos) == Some(response.layer_id)
                    {
                        let mut stroke = Stroke::new(
                            view.world(pos),
                            view.scale,
                            self.pencil_smoothing
                                .unwrap_or(crate::tool_keys::DEFAULT_PENCIL_SMOOTHING),
                        );
                        stroke.source = source;
                        if self.pencil_pressure && matches!(source, Source::Touch(..)) {
                            stroke.pressure = Some(pressure::Track::new(force));
                        }
                        self.pencil = Some(stroke);
                        self.note(Action::Draw);
                    }
                }
                Input::Move(source, p, force) => {
                    if let Some(stroke) = self.pencil.as_mut().filter(|s| s.source == source) {
                        if let Some(pressure) = &mut stroke.pressure {
                            pressure.set_force(force);
                        }
                        stroke.push(view.world(p));
                    }
                }
                Input::Release(source, pos, force) => {
                    if let Some(mut stroke) = self.pencil.take_if(|s| s.source == source) {
                        if let Some(pressure) = &mut stroke.pressure {
                            pressure.set_force(force);
                        }
                        stroke.push(view.world(pos));
                        if let Some((nodes, closed, profile)) = stroke.finish() {
                            let path = FinishedStroke {
                                nodes,
                                closed,
                                style: style.clone(),
                                profile,
                            };
                            if committed {
                                self.extra_pencil_strokes.push(path);
                                ui.ctx().request_repaint();
                            } else {
                                committed = path.apply(Arc::make_mut(doc), selection);
                            }
                        }
                    }
                }
                Input::Cancel(source) => {
                    self.pencil
                        .take_if(|s| source.is_none_or(|source| source == s.source));
                }
            }
        }
        if !ui.input(|i| i.focused) {
            self.pencil = None;
        }
        committed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn curve_points(nodes: &[PathNode]) -> impl Iterator<Item = Pos2> + '_ {
        nodes.windows(2).flat_map(|w| {
            let [p, a, b, q] = [w[0].point, w[0].outgoing, w[1].incoming, w[1].point]
                .map(|point| Pos2::from(point).to_vec2());
            (0..=64).map(move |i| {
                let t = i as f32 / 64.;
                let s = 1. - t;
                (s.powi(3) * p + 3. * s * t * (s * a + t * b) + t.powi(3) * q).to_pos2()
            })
        })
    }

    #[test]
    fn closing_fits_the_seam_as_part_of_the_curve_and_keeps_drawn_corners() {
        for smoothing in [0, 50, 100, 200] {
            let radius = 28.;
            let start = pos2(radius, 0.);
            let mut circle = Stroke::new(start, 1., smoothing);
            for i in 1..=128 {
                let angle = std::f32::consts::TAU * i as f32 / 128.;
                circle.push(pos2(radius * angle.cos(), radius * angle.sin()));
            }
            let (nodes, closed, _) = circle.finish().unwrap();
            assert!(closed);
            let seam = nodes[0];
            assert_eq!(seam.point, <[f32; 2]>::from(start));
            assert_eq!(seam.handle_style, HandleStyle::Smooth);
            assert!(seam.incoming[1] < 0. && seam.outgoing[1] > 0.);
            assert!(nodes[1..].iter().all(|n| n.point != seam.point));

            let mut square = Stroke::new(Pos2::ZERO, 1., smoothing);
            for point in [pos2(0., 60.), pos2(60., 60.), pos2(60., 0.), pos2(3., 0.)] {
                square.push(point);
            }
            let (nodes, closed, _) = square.finish().unwrap();
            assert!(closed);
            assert_eq!(nodes[0], PathNode::corner([0., 0.]));
        }
    }

    #[test]
    fn strongest_pencil_fit_keeps_both_lobes_of_a_shallow_s_at_different_sampling_rates() {
        use std::f32::consts::TAU;
        for steps in [16, 128, 640] {
            let mut stroke = Stroke::new(Pos2::ZERO, 1., 200);
            for i in 1..steps {
                let t = i as f32 / steps as f32;
                stroke.push(pos2(160. * t, 8. * (TAU * t).sin()));
            }
            stroke.push(pos2(160., 0.));
            let (nodes, closed, _) = stroke.finish().unwrap();
            assert!(!closed);
            assert_eq!(nodes.first().unwrap().point, [0., 0.]);
            assert_eq!(nodes.last().unwrap().point, [160., 0.]);
            let high = curve_points(&nodes).map(|p| p.y).fold(0., f32::max);
            let low = curve_points(&nodes).map(|p| p.y).fold(0., f32::min);
            assert!(
                (6. ..10.).contains(&high) && (-10. ..-6.).contains(&low),
                "{steps} samples: {low}..{high}"
            );
        }
    }

    #[test]
    fn sparse_pencil_samples_bound_the_curve_between_measured_points() {
        let points = [
            pos2(0., 0.),
            pos2(20., 12.),
            pos2(180., 12.),
            pos2(202., -2.),
            pos2(230., 26.),
            pos2(230., 70.),
        ];
        let tolerance = 3.4;
        let nodes = fit::nodes(&points, false, tolerance, 1.).unwrap();
        for point in curve_points(&nodes) {
            let nearest = points
                .windows(2)
                .map(|edge| {
                    let d = edge[1] - edge[0];
                    let along = ((point - edge[0]).dot(d) / d.length_sq()).clamp(0., 1.);
                    point.distance(edge[0] + along * d)
                })
                .fold(f32::INFINITY, f32::min);
            assert!(
                nearest <= tolerance + 0.001,
                "fitted point {point:?} was {nearest} px from the sparse trace"
            );
        }
    }

    #[test]
    fn pencil_smoothing_reduces_nodes_and_tracks_the_same_screen_tolerance_at_any_zoom() {
        let fit = |zoom, amount| {
            let mut stroke = Stroke::new(Pos2::ZERO, zoom, amount);
            for i in 1..250 {
                let x = i as f32;
                let y = (x * 0.08).sin() * 14. + (x * 1.7).sin() * 1.4;
                stroke.push(pos2(x / zoom, y / zoom));
            }
            stroke.finish().unwrap().0
        };
        let faithful = fit(1., 0);
        let smooth = fit(1., 100);
        let strongest = fit(1., 200);
        assert!(strongest.len() < smooth.len());
        assert_eq!(
            strongest.last().unwrap().point,
            smooth.last().unwrap().point
        );
        assert!(smooth.len() * 3 < faithful.len());
        assert!(smooth.iter().any(|n| n.handle_style == HandleStyle::Smooth));
        for zoom in [0.25, 16.] {
            let scaled = fit(zoom, 100);
            assert_eq!(scaled.len(), smooth.len());
            for (a, b) in smooth.iter().zip(scaled) {
                for (pa, pb) in [a.point, a.incoming, a.outgoing]
                    .into_iter()
                    .zip([b.point, b.incoming, b.outgoing])
                {
                    assert!(
                        Pos2::from(pa).distance((Pos2::from(pb).to_vec2() * zoom).to_pos2())
                            < 0.001
                    );
                }
            }
        }
    }

    #[test]
    fn uneven_subpixel_jitter_does_not_split_a_straight_default_pencil_stroke() {
        use std::f32::consts::TAU;

        for scale in [0.25, 1., 16.] {
            let fit = |amount| {
                let mut stroke = Stroke::new(Pos2::ZERO, scale, amount);
                for i in 1..=480 {
                    let x = i as f32;
                    let y = if i < 240 {
                        0.93 * (x * 1.6).sin() * (TAU * x / 480.).sin().abs()
                    } else {
                        0.
                    };
                    stroke.push(pos2(x / scale, y / scale));
                }
                let (nodes, closed, _) = stroke.finish().unwrap();
                assert!(!closed);
                assert_eq!(nodes[0].point, [0.; 2]);
                assert_eq!(nodes.last().unwrap().point, [480. / scale, 0.]);
                nodes
            };
            assert_eq!(fit(crate::tool_keys::DEFAULT_PENCIL_SMOOTHING).len(), 2);
            assert!(fit(0).len() > 2);
        }
    }

    #[test]
    fn pencil_long_strokes_stay_bounded_and_keep_the_exact_release_point() {
        let mut stroke = Stroke::new(Pos2::ZERO, 1., 30);
        let mut end = Pos2::ZERO;
        for i in 1..90_000 {
            let x = i as f32;
            end = pos2(x, 12. * (x * 0.012).sin());
            stroke.push(end);
            assert!(stroke.points.len() <= MAX_SAMPLES);
        }
        let (nodes, closed, _) = stroke.finish().unwrap();
        assert!(!closed);
        assert!(nodes.len() < MAX_SAMPLES / 2);
        assert_eq!(nodes[0].point, [0., 0.]);
        assert_eq!(nodes.last().unwrap().point, <[f32; 2]>::from(end));
    }
}

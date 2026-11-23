use super::workspace::{INK, MATTE, MINT};
use super::{MorphDialog, MorphGuide, MorphPoint};
use document_core::{Shape, VectorObject};
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Sense, Stroke, Ui};

const DIRECT_PREVIEW_NODES: usize = 64;

#[derive(Default)]
pub(super) struct OutlineCache {
    bounds: Option<Rect>,
    texture: Option<([u32; 2], egui::TextureHandle)>,
    hover: Option<(Rect, Pos2, Option<MorphPoint>)>,
}

impl MorphDialog {
    pub(super) fn match_points(&mut self, ui: &mut Ui, object: &VectorObject, height: f32) {
        let Some(target) = self.to.as_ref() else {
            return;
        };
        let start_transform = object.local_to_world_at(self.start);
        let end_transform = object.local_to_world_at(self.end);
        let start_bounds = *self.previews[0]
            .bounds
            .get_or_insert_with(|| bounds(&self.from, &start_transform));
        let end_bounds = *self.previews[1]
            .bounds
            .get_or_insert_with(|| bounds(target, &end_transform));
        let mut picks = [None, None];
        ui.columns(2, |cols| {
            for (i, (shape, transform, bounds)) in [
                (&self.from, &start_transform, start_bounds),
                (target, &end_transform, end_bounds),
            ]
            .into_iter()
            .enumerate()
            {
                let col = &mut cols[i];
                let (heading, tick) = if i == 0 {
                    ("Start", self.start)
                } else {
                    ("End", self.end)
                };
                col.strong(format!("{heading} · Frame {}", tick + 1));
                let (rect, response) = col.allocate_exact_size(
                    vec2(col.available_width(), (height - 80.).max(30.)),
                    Sense::click(),
                );
                let screen = |p| fit(rect, bounds, transform(p));
                let cache = &mut self.previews[i];
                paint_outline(
                    col,
                    rect,
                    shape,
                    &screen,
                    cache,
                    object.has_fill_at(tick).then(|| object.fill_at(tick)),
                );
                for (index, pair) in self.guides.iter().enumerate() {
                    let mark = if i == 0 { pair.from } else { pair.to };
                    if let Some(point) = mark.position(shape) {
                        marker(col, screen(point), index);
                    }
                }
                if i == 0 {
                    if let Some(point) = self.pending.and_then(|p| p.position(shape)) {
                        marker(col, screen(point), self.guides.len());
                    }
                }
                if i == 1 && self.pending.is_none() {
                    continue;
                }
                // Dense paths only need an exact segment lookup when the artist
                // actually places a match. Moving across the preview stays cheap.
                if shape.contours()[0].len() > DIRECT_PREVIEW_NODES && !response.clicked() {
                    response.on_hover_cursor(egui::CursorIcon::Crosshair);
                    continue;
                }
                if let Some(point) = response.hover_pos() {
                    let hit = match cache.hover {
                        Some((last_rect, last_point, hit))
                            if last_rect == rect && last_point == point =>
                        {
                            hit
                        }
                        _ => {
                            let hit = pick(shape, &screen, point);
                            cache.hover = Some((rect, point, hit));
                            hit
                        }
                    };
                    if let Some(hit) = hit {
                        if let Some(p) = hit.position(shape) {
                            col.painter_at(rect).circle_stroke(
                                screen(p),
                                6.,
                                Stroke::new(2_f32, INK),
                            );
                        }
                        if response
                            .on_hover_cursor(egui::CursorIcon::Crosshair)
                            .clicked()
                        {
                            picks[i] = Some(hit);
                        }
                    }
                }
            }
        });
        if let Some(from) = picks[0] {
            self.pending = Some(from);
        }
        if let (Some(from), Some(to)) = (self.pending, picks[1]) {
            self.guides.push(MorphGuide { from, to });
            self.pending = None;
            self.rebuild();
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(if self.pending.is_some() {
                "Click the matching point on the end pose."
            } else {
                "Click a point on the start pose to match it."
            });
            if self.pending.is_some() && ui.small_button("Cancel point").clicked() {
                self.pending = None;
            }
        });
        if !self.guides.is_empty() {
            egui::ScrollArea::horizontal()
                .id_salt("morph-matches")
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Matches");
                        let mut remove = None;
                        for i in 0..self.guides.len() {
                            if ui
                                .small_button(format!("{} ×", i + 1))
                                .on_hover_text("Remove this match")
                                .clicked()
                            {
                                remove = Some(i)
                            }
                        }
                        if let Some(i) = remove {
                            self.guides.remove(i);
                            self.rebuild();
                        }
                    })
                });
        }
    }
}

fn bounds(shape: &Shape, world: &impl Fn([f32; 2]) -> [f32; 2]) -> Rect {
    let mut bounds = Rect::NOTHING;
    for n in shape.contours().iter().flatten() {
        for p in [n.incoming, n.point, n.outgoing] {
            bounds.extend_with(Pos2::from(world(p)));
        }
    }
    bounds
}

fn fit(rect: Rect, bounds: Rect, point: [f32; 2]) -> Pos2 {
    let inner = rect.shrink((rect.width().min(rect.height()) * 0.12).clamp(6., 60.));
    let scale =
        (inner.width() / bounds.width().max(0.01)).min(inner.height() / bounds.height().max(0.01));
    inner.center() + (Pos2::from(point) - bounds.center()) * scale
}

fn paint_outline(
    ui: &Ui,
    rect: Rect,
    shape: &Shape,
    screen: &impl Fn([f32; 2]) -> Pos2,
    cache: &mut OutlineCache,
    fill: Option<[u8; 4]>,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3., MATTE);
    let Shape::Path { nodes, closed } = shape else {
        return;
    };
    // Repainting the dialog should submit a single quad for a detailed outline.
    // Picking always runs against its original cubic segments.
    let dense = nodes.len() > DIRECT_PREVIEW_NODES;
    if dense || fill.is_some() {
        let ppp = ui
            .ctx()
            .pixels_per_point()
            .min(1024. / rect.size().max_elem());
        let size = [rect.width(), rect.height()].map(|n| (n * ppp).ceil().max(1.) as u32);
        if cache.texture.as_ref().map(|(old, _)| *old) != Some(size) {
            let pixels = |point| (screen(point) - rect.min) * ppp;
            cache.texture =
                raster_outline(nodes, *closed, size, ppp, pixels, fill, dense).map(|image| {
                    (
                        size,
                        ui.ctx()
                            .load_texture("morph-outline", image, egui::TextureOptions::LINEAR),
                    )
                });
        }
        if let Some((_, texture)) = &cache.texture {
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                Color32::WHITE,
            );
            if dense {
                return;
            }
        }
    }
    let stroke = Stroke::new(1.3_f32, INK);
    for controls in segments(nodes, *closed) {
        let controls = controls.map(screen);
        if controls.iter().all(|p| p.is_finite()) {
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                controls,
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
        }
    }
}

fn segments(
    nodes: &[document_core::PathNode],
    closed: bool,
) -> impl Iterator<Item = [[f32; 2]; 4]> + '_ {
    nodes
        .iter()
        .zip(nodes.iter().cycle().skip(1))
        .take(nodes.len().saturating_sub(usize::from(!closed)))
        .map(|(a, b)| [a.point, a.outgoing, b.incoming, b.point])
}

fn raster_outline(
    nodes: &[document_core::PathNode],
    closed: bool,
    size: [u32; 2],
    ppp: f32,
    pixels: impl Fn([f32; 2]) -> egui::Vec2,
    fill: Option<[u8; 4]>,
    outline: bool,
) -> Option<egui::ColorImage> {
    let mut path = tiny_skia::PathBuilder::new();
    let first = pixels(nodes.first()?.point);
    path.move_to(first.x, first.y);
    for [_, h1, h2, end] in segments(nodes, closed) {
        let [h1, h2, end] = [h1, h2, end].map(&pixels);
        path.cubic_to(h1.x, h1.y, h2.x, h2.y, end.x, end.y);
    }
    if closed {
        path.close();
    }
    let mut bitmap = tiny_skia::Pixmap::new(size[0], size[1])?;
    let mut paint = tiny_skia::Paint::default();
    let path = path.finish()?;
    if let Some([r, g, b, a]) = fill {
        paint.set_color_rgba8(r, g, b, a);
        bitmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    if outline {
        paint.set_color_rgba8(INK.r(), INK.g(), INK.b(), 255);
        bitmap.stroke_path(
            &path,
            &paint,
            &tiny_skia::Stroke {
                width: 2. * ppp,
                ..Default::default()
            },
            tiny_skia::Transform::identity(),
            None,
        );
    }
    Some(egui::ColorImage::from_rgba_premultiplied(
        size.map(|n| n as usize),
        bitmap.data(),
    ))
}

fn pick(shape: &Shape, screen: &impl Fn([f32; 2]) -> Pos2, mouse: Pos2) -> Option<MorphPoint> {
    let nodes = shape.contours().first()?;
    let near = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            let d = screen(n.point).distance_sq(mouse);
            (d <= 25.).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, _)) = near {
        if i + 1 == nodes.len() && matches!(shape, Shape::Path { closed: false, .. }) {
            return Some(MorphPoint {
                segment: i - 1,
                t: 1.,
            });
        }
        return Some(MorphPoint { segment: i, t: 0. });
    }
    crate::canvas::nearest_morph_point(shape, |p| screen(p).into(), mouse, 9.)
}

fn marker(ui: &Ui, center: Pos2, index: usize) {
    let painter = ui.painter();
    let badge = center + vec2(11., -13.);
    painter.line_segment([center, badge], (1.3, MINT));
    painter.circle_filled(center, 3.5, MINT);
    painter.circle_filled(badge, 8., MINT);
    painter.text(
        badge,
        egui::Align2::CENTER_CENTER,
        (index + 1).to_string(),
        egui::FontId::monospace(11.),
        INK,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_outline_reuses_preview_texture_while_idle() {
        let ctx = egui::Context::default();
        let shape = Shape::Path {
            closed: true,
            nodes: (0..1024)
                .map(|i| {
                    let (y, x) = (std::f32::consts::TAU * i as f32 / 1024.).sin_cos();
                    document_core::PathNode::corner([60. + x * 40., 60. + y * 40.])
                })
                .collect(),
        };
        let mut cache = OutlineCache::default();
        let mut draw = || {
            let out = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    paint_outline(
                        ui,
                        Rect::from_min_size(Pos2::ZERO, vec2(120., 120.)),
                        &shape,
                        &Pos2::from,
                        &mut cache,
                        None,
                    );
                });
            });
            let texture = cache.texture.as_ref().unwrap().1.id();
            assert_eq!(
                out.shapes
                    .iter()
                    .filter(|s| matches!(&s.shape,egui::Shape::Mesh(m) if m.texture_id == texture))
                    .count(),
                1
            );
            out.textures_delta
                .set
                .iter()
                .filter(|(id, _)| *id == texture)
                .count()
        };
        assert_eq!(draw(), 1);
        assert_eq!(draw(), 0);
    }
}

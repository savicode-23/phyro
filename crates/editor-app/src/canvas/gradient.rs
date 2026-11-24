use super::*;
use document_core::{Gradient, GradientKind, VectorObject};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Handle {
    Draw,
    Origin,
    End,
    VerticalRadius,
}

pub(super) fn editable(
    doc: &Document,
    object: &VectorObject,
    frame: u32,
    style: &CanvasStyle,
) -> bool {
    crate::commands::has_vector_paint(object)
        && object.visible_at(frame)
        && document_core::arrange::is_object_editable(doc, object)
        && style
            .key_mode
            .allows(object, frame, style.gradient_channel.property())
}

impl CanvasStyle {
    pub(crate) fn gradient_controls(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label("Edit gradient");
        for channel in [PaintChannel::Fill, PaintChannel::Stroke] {
            ui.selectable_value(&mut self.gradient_channel, channel, channel.name());
        }
    }
}

// Match the renderer's paint bounds, including the stroke width on a straight line.
fn mapping(
    object: &VectorObject,
    frame: u32,
    channel: PaintChannel,
    inverse: bool,
) -> Option<impl Fn(Pos2) -> Pos2> {
    let space = match channel {
        PaintChannel::Fill => object.paint_space(frame),
        PaintChannel::Stroke => object.stroke_paint_space(frame),
    }?;
    let m = if inverse { space.invert()? } else { space };
    Some(move |p: Pos2| {
        egui::pos2(
            m.sx * p.x + m.kx * p.y + m.tx,
            m.ky * p.x + m.sy * p.y + m.ty,
        )
    })
}

fn handles(kind: &GradientKind) -> impl Iterator<Item = (Handle, Pos2)> {
    let (origin, end, vertical) = match *kind {
        GradientKind::Linear { start, end } => (start, end, None),
        GradientKind::Radial { center, radius } | GradientKind::Rectangular { center, radius } => (
            center,
            [center[0] + radius, center[1]],
            Some([center[0], center[1] + radius]),
        ),
    };
    [(Handle::Origin, origin.into()), (Handle::End, end.into())]
        .into_iter()
        .chain(vertical.map(|p| (Handle::VerticalRadius, p.into())))
}

pub(super) fn begin(
    doc: &Document,
    frame: u32,
    point: Pos2,
    selection: &mut Vec<u64>,
    scale: f32,
    style: &CanvasStyle,
) -> Option<Gesture> {
    let channel = style.gradient_channel;
    if let [id] = selection.as_slice() {
        if let Some(object) = doc
            .objects
            .iter()
            .find(|o| o.id == *id && editable(doc, o, frame, style))
        {
            // An unpainted object selected in Layers can still receive a
            // gradient. Its dormant gradient has no draggable handles.
            if channel.is_none(object) {
                let local = object.world_to_local(point.into(), frame);
                if local.iter().all(|axis| (0.0..=1.0).contains(axis))
                    && object.stencil_contains(point.into(), frame)
                {
                    return Some(Gesture::Gradient {
                        id: *id,
                        start: point,
                        handle: Handle::Draw,
                        channel,
                    });
                }
            } else if let (Some(g), Some(to_world)) = (
                channel.gradient(object),
                mapping(object, frame, channel, false),
            ) {
                if let Some((handle, _)) = handles(&g.kind)
                    .map(|(h, p)| (h, to_world(p).distance(point) * scale))
                    .filter(|(_, d)| *d <= 7.)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                {
                    return Some(Gesture::Gradient {
                        id: *id,
                        start: point,
                        handle,
                        channel,
                    });
                }
            }
        }
    }
    let scope = style.group_scope();
    let Some(id) = doc.hit_test_in(point.into(), frame, scope) else {
        selection.clear();
        return None;
    };
    *selection = scope.resolve(doc, &[id]);
    let [id] = selection.as_slice() else {
        return None;
    };
    doc.objects
        .iter()
        .find(|o| o.id == *id && editable(doc, o, frame, style))
        .map(|_| Gesture::Gradient {
            id: *id,
            start: point,
            handle: Handle::Draw,
            channel,
        })
}

pub(super) fn apply(
    object: &mut VectorObject,
    frame: u32,
    start: Pos2,
    point: Pos2,
    handle: Handle,
    channel: PaintChannel,
    style: &CanvasStyle,
) -> bool {
    if !style.key_mode.allows(object, frame, channel.property()) {
        return false;
    }
    let Some((a, b)) =
        mapping(object, frame, channel, true).map(|local| (local(start), local(point)))
    else {
        return false;
    };
    let delta = b - a;
    if !a.is_finite() || !b.is_finite() || delta == Vec2::ZERO {
        return false;
    }
    let prior = channel.gradient(object).map(|g| &g.kind);
    let next = match (prior, handle) {
        (Some(GradientKind::Radial { .. }), Handle::Draw) => GradientKind::Radial {
            center: a.into(),
            radius: delta.length(),
        },
        (Some(GradientKind::Rectangular { .. }), Handle::Draw) => GradientKind::Rectangular {
            center: a.into(),
            radius: delta.abs().max_elem(),
        },
        (_, Handle::Draw) => GradientKind::Linear {
            start: a.into(),
            end: b.into(),
        },
        (Some(GradientKind::Linear { start, end }), Handle::Origin) => GradientKind::Linear {
            start: (Pos2::from(*start) + delta).into(),
            end: *end,
        },
        (Some(GradientKind::Linear { start, end }), Handle::End) => GradientKind::Linear {
            start: *start,
            end: (Pos2::from(*end) + delta).into(),
        },
        (
            Some(
                GradientKind::Radial { center, radius }
                | GradientKind::Rectangular { center, radius },
            ),
            _,
        ) => {
            let (mut center, mut radius) = (*center, *radius);
            match handle {
                Handle::Origin => center = (Pos2::from(center) + delta).into(),
                Handle::End => radius = (radius + delta.x).max(0.0001),
                Handle::VerticalRadius => radius = (radius + delta.y).max(0.0001),
                Handle::Draw => return false,
            };
            if matches!(prior, Some(GradientKind::Rectangular { .. })) {
                GradientKind::Rectangular { center, radius }
            } else {
                GradientKind::Radial { center, radius }
            }
        }
        _ => return false,
    };
    if (Some(&next) == prior && !channel.is_none(object)) || !next.valid() {
        return false;
    }
    if channel == PaintChannel::Stroke
        && (prior.is_none() || channel.is_none(object))
        && object.stroke_width_at(frame) == 0.
        && !style.key_mode.set(
            object,
            frame,
            document_core::Property::StrokeWidth,
            document_core::KeyValue::Scalar(if style.stroke_width > 0. {
                style.stroke_width
            } else {
                1.
            }),
        )
    {
        return false;
    }
    if let Some(g) = channel.gradient_mut(object) {
        g.kind = next;
    } else {
        let color = channel.color_at(object, frame);
        *channel.gradient_mut(object) = Some(Gradient::from_color(next, color));
    }
    channel.set_none(object, false);
    true
}

pub(super) fn paint(
    painter: &egui::Painter,
    view: View,
    object: &VectorObject,
    frame: u32,
    pointer: Option<Pos2>,
    channel: PaintChannel,
) -> bool {
    let Some(g) = channel
        .gradient(object)
        .filter(|_| !channel.is_none(object))
    else {
        return false;
    };
    let Some(world) = mapping(object, frame, channel, false) else {
        return false;
    };
    let mut points = handles(&g.kind).map(|(h, p)| (h, view.screen(world(p))));
    let Some((_, origin)) = points.next() else {
        return false;
    };
    let ink = match channel {
        PaintChannel::Fill => Color32::from_rgb(24, 70, 125),
        PaintChannel::Stroke => Color32::from_rgb(129, 64, 23),
    };
    for (_, p) in points {
        painter.line_segment([origin, p], (2.5, Color32::WHITE));
        painter.line_segment([origin, p], (1., ink));
    }
    let mut hover = false;
    for (handle, pos) in handles(&g.kind).map(|(h, p)| (h, view.screen(world(p)))) {
        let is_origin = handle == Handle::Origin;
        let stop = if is_origin {
            g.stops.first()
        } else {
            g.stops.last()
        };
        let [r, g, b, _] = stop.map_or([255; 4], |s| s.color);
        let color = Color32::from_rgb(r, g, b);
        let over = pointer.is_some_and(|p| p.distance(pos) <= 7.);
        hover |= over;
        let radius = if over { 5.5 } else { 4.5 };
        if is_origin {
            let rect = Rect::from_center_size(pos, Vec2::splat(2. * radius));
            painter.rect_filled(rect.expand(1.5), 1., Color32::WHITE);
            painter.rect(rect, 0., color, (1.5, ink), egui::StrokeKind::Inside);
        } else {
            painter.circle_filled(pos, radius + 1.5, Color32::WHITE);
            painter.circle(pos, radius, color, (1.5, ink));
        }
    }
    hover
}

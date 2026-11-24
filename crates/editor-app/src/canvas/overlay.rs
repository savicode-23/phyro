use super::*;
use document_core::{GridSettings, GridVisibility};
use eframe::egui::epaint::CubicBezierShape;
use gesture::{handles, rotation_handle};

#[cfg(test)]
mod group_tests;

const ACCENT: Color32 = Color32::from_rgb(38, 137, 236);
const GUIDE: Color32 = Color32::from_rgb(197, 95, 174);

pub(super) fn guides(p: &egui::Painter, view: View, doc: &Document, snap: SnapSettings) {
    let visible = view.board.intersect(p.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let show_grid = match snap.grid_settings.visibility {
        GridVisibility::Visible => true,
        GridVisibility::WhenSnapping => snap.enabled && snap.grid,
        GridVisibility::Hidden => false,
    };
    if show_grid {
        grid(p, view, visible, snap.grid_settings);
    }
    for &x in &doc.guides_x {
        let screen = view.screen(egui::pos2(x, 0.)).x;
        if visible.x_range().contains(screen) {
            p.vline(screen, visible.y_range(), (1., GUIDE));
        }
    }
    for &y in &doc.guides_y {
        let screen = view.screen(egui::pos2(0., y)).y;
        if visible.y_range().contains(screen) {
            p.hline(visible.x_range(), screen, (1., GUIDE));
        }
    }
}

fn grid(p: &egui::Painter, view: View, visible: Rect, settings: GridSettings) {
    let grid = settings.geometry();
    let scale = f64::from(view.scale);
    let column_spacing = grid.column_spacing() * scale;
    let row_spacing = f64::from(settings.spacing) * scale;
    // A screen-space floor keeps both dense grids and zoomed-out canvases legible.
    if column_spacing < 8.
        || grid
            .row_slopes()
            .any(|slope| row_spacing / slope.hypot(1.) < 8.)
    {
        return;
    }

    let relative_origin = (visible.min - view.board.min).to_pos2();
    let [ox, oy] = settings.origin.map(|value| f64::from(value) * scale);
    let (ox, oy) = (
        ox - f64::from(relative_origin.x),
        oy - f64::from(relative_origin.y),
    );
    let (width, height) = (f64::from(visible.width()), f64::from(visible.height()));
    let stroke = (1., Color32::from_black_alpha(22));
    let start = ox.rem_euclid(column_spacing);
    for column in 0..=((width - start) / column_spacing).floor() as i32 {
        let x = start + f64::from(column) * column_spacing;
        p.vline(visible.left() + x as f32, visible.y_range(), stroke);
    }
    for slope in grid.row_slopes() {
        let across = slope * width;
        let offset = (oy - slope * ox).rem_euclid(row_spacing);
        let first = ((-across.max(0.) - offset) / row_spacing).ceil() as i32;
        let last = ((height - across.min(0.) - offset) / row_spacing).floor() as i32;
        for row in first..=last {
            let y = offset + f64::from(row) * row_spacing;
            let mut limits = [0., width];
            if slope != 0. {
                let (top, bottom) = (-y / slope, (height - y) / slope);
                limits = [top.min(bottom).max(0.), top.max(bottom).min(width)];
            }
            p.line_segment(
                limits.map(|x| {
                    visible.min + egui::vec2(x as f32, (y + slope * x).clamp(0., height) as f32)
                }),
                stroke,
            );
        }
    }
}

mod nodes;
pub(super) use nodes::NodeOverlay;

fn visible_selection<'a>(
    doc: &'a Document,
    ids: &'a [u64],
) -> impl Iterator<Item = &'a document_core::VectorObject> + 'a {
    doc.objects
        .iter()
        .filter(|o| ids.contains(&o.id) && doc.layers.iter().any(|l| l.id == o.layer && l.visible))
}

pub(super) fn selection(
    p: &egui::Painter,
    view: View,
    doc: &Document,
    frame: u32,
    ids: &[u64],
    tool: Tool,
) {
    for object in visible_selection(doc, ids) {
        frame_outline(
            p,
            view,
            object,
            frame,
            tool == Tool::Select && ids.len() == 1,
        );
    }
}

pub(super) fn frame_outline(
    p: &egui::Painter,
    view: View,
    object: &document_core::VectorObject,
    frame: u32,
    single_select: bool,
) {
    let world = object.local_to_world_at(frame);
    let screen = |local| view.screen(world(local).into());
    let corners = handles(object, frame).map(|p| screen(p.into()));
    if !corners.iter().all(|c| c.is_finite()) {
        return;
    }
    outline(p, corners);
    if single_select {
        for (_, handle, _) in gesture::resize_handles(corners, 1.) {
            draw_corner(p, handle);
        }
        let top = corners[0] + 0.5 * (corners[1] - corners[0]);
        let rotate = view.screen(rotation_handle(object, frame, view.world(top), view.scale));
        p.line_segment([top, rotate], (1., ACCENT));
        p.circle(rotate, 4.0, Color32::WHITE, (1., ACCENT));
    }
}

pub(super) fn group_selection(
    painter: &egui::Painter,
    view: View,
    doc: &Document,
    frame: u32,
    ids: &[u64],
    rotation: Option<(Rect, f32)>,
) {
    // Keep short selections allocation-free. Larger marquees otherwise scan
    // the selected IDs once for each object in the document.
    let index = (ids.len() > 16).then(|| {
        ids.iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
    });
    let selected = |id| {
        index
            .as_ref()
            .map_or_else(|| ids.contains(&id), |set| set.contains(&id))
    };
    let mut bounds = Rect::NOTHING;
    let clip = painter
        .clip_rect()
        .expand(1. + 1. / painter.ctx().pixels_per_point());
    for object in doc.objects.iter().filter(|o| selected(o.id)) {
        let Some(layer) = doc
            .layers
            .iter()
            .find(|l| l.id == object.layer && l.visible)
        else {
            continue;
        };
        let to_world = object.local_to_world_at(frame);
        let world = handles(object, frame).map(|p| Pos2::from(to_world(p.into())));
        if rotation.is_none() && !layer.locked {
            for point in world {
                bounds.extend_with(point);
            }
        }
        let corners = world.map(|p| view.screen(p));
        if corners.iter().all(|p| p.is_finite()) && clip.intersects(Rect::from_points(&corners)) {
            outline(painter, corners);
        }
    }
    // Offscreen objects still determine the group handles and rotation pivot.
    if let Some((bounds, angle)) =
        rotation.or_else(|| (bounds.is_finite() && bounds.is_positive()).then_some((bounds, 0.)))
    {
        group_handles(painter, view, bounds, angle);
    }
}

fn outline(painter: &egui::Painter, corners: [Pos2; 4]) {
    for (i, corner) in corners.iter().enumerate() {
        painter.line_segment([*corner, corners[(i + 1) % 4]], (1., ACCENT));
    }
}

fn draw_corner(p: &egui::Painter, pos: Pos2) {
    let rect = Rect::from_center_size(pos, Vec2::splat(7.));
    p.rect(
        rect,
        0.,
        Color32::WHITE,
        (1., ACCENT),
        egui::StrokeKind::Inside,
    );
}

pub(super) fn group_handles(p: &egui::Painter, view: View, bounds: Rect, angle: f32) {
    let screen = |world| view.screen(gesture::rotate_around(world, bounds.center(), angle));
    let corners = box_corners(bounds).map(screen);
    for i in 0..4 {
        p.line_segment([corners[i], corners[(i + 1) % 4]], (1.5, ACCENT));
    }
    for (point, _) in gesture::group_resize_handles(bounds, view.scale) {
        draw_corner(p, screen(point));
    }
    let top = bounds.center_top();
    let rotate = screen(top - egui::vec2(0., 18. / view.scale));
    p.line_segment([screen(top), rotate], (1., ACCENT));
    p.circle(rotate, 4.0, Color32::WHITE, (1., ACCENT));
}

pub(super) fn marquee(p: &egui::Painter, view: View, start: Pos2, end: Pos2) {
    let rect = Rect::from_two_pos(view.screen(start), view.screen(end));
    p.rect(
        rect,
        0.,
        Color32::from_rgba_unmultiplied(45, 146, 240, 24),
        (1., ACCENT),
        egui::StrokeKind::Inside,
    );
}

pub(super) enum PenPreview {
    Extend(PathNode),
    Close {
        first: PathNode,
        guide: Option<Pos2>,
    },
}

pub(super) fn pen_ends(
    p: &egui::Painter,
    view: View,
    ends: paths::Endpoints,
    hovered: Option<document_core::path_edit::PathEnd>,
) {
    use document_core::path_edit::PathEnd;
    for (point, end) in ends
        .points()
        .into_iter()
        .zip([PathEnd::Start, PathEnd::End])
    {
        let at = view.screen(point);
        draw_corner(p, at);
        if hovered == Some(end) {
            close_ring(p, at);
            p.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }
}

pub(super) fn pen(
    p: &egui::Painter,
    view: View,
    nodes: &[PathNode],
    preview: Option<PenPreview>,
    opposite: Option<Pos2>,
) {
    let curve = |a: &PathNode, b: &PathNode| {
        let points = [a.point, a.outgoing, b.incoming, b.point].map(|p| view.screen(p.into()));
        p.add(CubicBezierShape::from_points_stroke(
            points,
            false,
            Color32::TRANSPARENT,
            (1.5, ACCENT),
        ));
    };
    for pair in nodes.windows(2) {
        curve(&pair[0], &pair[1]);
    }
    let (cursor, closing, guide) = match &preview {
        Some(PenPreview::Extend(next)) => (Some(next), None, None),
        Some(PenPreview::Close { first, guide }) => (None, Some(first), *guide),
        None => (None, None, None),
    };
    if let (Some(last), Some(next)) = (nodes.last(), closing.or(cursor)) {
        curve(last, next);
    }
    if let (Some(first), Some(guide)) = (closing, guide) {
        let anchor = view.screen(first.point.into());
        let tip = view.screen(guide);
        if anchor.distance_sq(tip) > 4. {
            p.line_segment([anchor, tip], (1., ACCENT.gamma_multiply(0.55)));
            p.circle_stroke(tip, 3., (1., ACCENT));
        }
    }
    if let Some(point) = opposite.filter(|_| closing.is_none()) {
        draw_corner(p, view.screen(point));
    }
    let first = if opposite.is_some() {
        nodes.first()
    } else {
        closing.or(nodes.first())
    };
    for n in first
        .into_iter()
        .chain(nodes.iter().skip(1))
        .chain(cursor)
        .chain(closing.filter(|_| opposite.is_some()))
    {
        let anchor = view.screen(n.point.into());
        for tip in [n.incoming, n.outgoing] {
            let handle = view.screen(tip.into());
            if anchor.distance_sq(handle) > 4. {
                p.line_segment([anchor, handle], (1., ACCENT));
                p.circle_filled(handle, 3., ACCENT);
            }
        }
        draw_corner(p, anchor);
    }
    if let Some(first) = closing {
        close_ring(p, view.screen(first.point.into()));
    }
}

pub(super) fn close_ring(p: &egui::Painter, start: Pos2) {
    p.circle_stroke(start, 9., (4., Color32::WHITE));
    p.circle_stroke(start, 9., (2., ACCENT));
}
pub(super) fn editing_group(
    painter: &egui::Painter,
    view: View,
    doc: &Document,
    frame: u32,
    group: document_core::arrange::Group,
    preview: Option<&document_core::VectorObject>,
) {
    let mut bounds = Rect::NOTHING;
    for object in doc.objects.iter().filter(|o| group.contains(o)) {
        let object = preview.filter(|o| o.id == object.id).unwrap_or(object);
        if object.visible_at(frame) {
            let [left, top, right, bottom] = object.bounds(frame);
            bounds = bounds.union(Rect::from_min_max(
                view.screen(egui::pos2(left, top)),
                view.screen(egui::pos2(right, bottom)),
            ));
        }
    }
    if !bounds.is_finite() {
        return;
    }
    let bounds = bounds.expand(9.);
    let clip = painter.clip_rect().expand(2.);
    let stroke = egui::Stroke::new(1.5_f32, crate::theme::MINT);
    for y in [bounds.top(), bounds.bottom()] {
        if (clip.top()..=clip.bottom()).contains(&y) {
            let left = bounds.left().max(clip.left());
            let right = bounds.right().min(clip.right());
            if left < right {
                painter.extend(egui::Shape::dashed_line(
                    &[egui::pos2(left, y), egui::pos2(right, y)],
                    stroke,
                    5.,
                    4.,
                ));
            }
        }
    }
    for x in [bounds.left(), bounds.right()] {
        if (clip.left()..=clip.right()).contains(&x) {
            let top = bounds.top().max(clip.top());
            let bottom = bounds.bottom().min(clip.bottom());
            if top < bottom {
                painter.extend(egui::Shape::dashed_line(
                    &[egui::pos2(x, top), egui::pos2(x, bottom)],
                    stroke,
                    5.,
                    4.,
                ));
            }
        }
    }
}

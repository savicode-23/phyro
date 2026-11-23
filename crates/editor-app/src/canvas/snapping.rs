use super::{gesture::GestureIds, Document, Pos2, Rect, Vec2};
use crate::path_editor::NodeSet;
use document_core::{GridKind, GridSettings, VectorObject};
use eframe::egui;
use std::sync::{Arc, Weak};

#[cfg(test)]
mod pen_tests;

#[derive(Clone, Copy)]
pub struct SnapSettings {
    pub enabled: bool,
    pub grid: bool,
    pub guides: bool,
    pub bounds: bool,
    pub anchors: bool,
    pub grid_settings: GridSettings,
    pub tolerance: f32,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            grid: true,
            guides: true,
            bounds: true,
            anchors: true,
            grid_settings: GridSettings::default(),
            tolerance: 6.,
        }
    }
}

pub(super) struct Hit {
    pub point: Pos2,
    pub lines: [Option<f32>; 2],
    pub anchor: Option<Pos2>,
}

// Identify the coordinate to hold fixed for the nearest cardinal direction.
pub(super) fn fixed_axis(origin: Pos2, point: Pos2) -> usize {
    let delta = (point - origin).abs();
    usize::from(delta.x >= delta.y)
}

impl Hit {
    pub fn constrain(&mut self, axis: usize, value: f32) {
        self.point[axis] = value;
        self.lines[axis] = None;
        self.anchor = self.anchor.filter(|anchor| *anchor == self.point);
    }

    pub fn free(point: Pos2) -> Self {
        Self {
            point,
            lines: [None; 2],
            anchor: None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Subject<'a> {
    Point,
    Object(u64),
    Objects(&'a GestureIds),
    Moving(&'a GestureIds, Vec2),
    Nodes { id: u64, nodes: &'a NodeSet },
}

/// For a move, each side and the centre of the moving selection can align to
/// a target. For a pen point, node or handle, only the point under edit moves.
pub(super) fn resolve(
    point: Pos2,
    doc: &Document,
    frame: u32,
    settings: SnapSettings,
    scale: f32,
    subject: Subject<'_>,
) -> Hit {
    if !settings.enabled {
        return Hit::free(point);
    }
    let radius = settings.tolerance / scale;
    let moving = match subject {
        Subject::Moving(_, delta) => Some(delta),
        _ => None,
    };
    let mut nearest = Nearest::new(radius);
    let mut targets = guide_axes(doc, settings);
    let moving_bounds = gather(
        &snap_objects(doc, settings, subject),
        frame,
        settings,
        subject,
        None,
        &mut targets,
        |a| nearest.visit(point, a),
    );
    let source = moving
        .filter(|_| moving_bounds.is_finite())
        .map(|delta| moving_bounds.translate(delta))
        .unwrap_or(Rect::from_min_max(point, point));
    finish(point, settings, radius, source, &targets, nearest.anchor)
}

struct Nearest {
    anchor: Option<Pos2>,
    distance_sq: f32,
}

impl Nearest {
    fn new(radius: f32) -> Self {
        Self {
            anchor: None,
            distance_sq: radius * radius,
        }
    }

    fn visit(&mut self, point: Pos2, anchor: Pos2) {
        let distance = point.distance_sq(anchor);
        if distance <= self.distance_sq {
            self.anchor = Some(anchor);
            self.distance_sq = distance;
        }
    }
}

fn guide_axes(doc: &Document, settings: SnapSettings) -> [Vec<f32>; 2] {
    if settings.guides {
        [doc.guides_x.clone(), doc.guides_y.clone()]
    } else {
        Default::default()
    }
}

fn snap_objects<'a>(
    doc: &'a Document,
    settings: SnapSettings,
    subject: Subject<'_>,
) -> Vec<&'a VectorObject> {
    if settings.bounds
        || settings.anchors
        || settings.guides && matches!(subject, Subject::Moving(..))
    {
        doc.paint_order()
    } else {
        Vec::new()
    }
}

fn gather(
    objects: &[&VectorObject],
    frame: u32,
    settings: SnapSettings,
    subject: Subject<'_>,
    bounds_region: Option<Rect>,
    targets: &mut [Vec<f32>; 2],
    mut visit_anchor: impl FnMut(Pos2),
) -> Rect {
    let moving = matches!(subject, Subject::Moving(..));
    let mut moving_bounds = Rect::NOTHING;
    let align_selection = moving && (settings.bounds || settings.guides);
    for object in objects {
        if !object.visible_at(frame) || object.opacity_at(frame) <= 0. {
            continue;
        }
        let excluded = match subject {
            Subject::Objects(ids) | Subject::Moving(ids, _) => ids.contains(object.id),
            Subject::Object(id) | Subject::Nodes { id, .. } => object.id == id,
            Subject::Point => false,
        };
        // The conservative bounds also enclose the unskinned path's anchors.
        // Animated and deformed paths pass this filter for exact evaluation.
        // Leave excluded objects available for moving bounds and sibling nodes.
        if settings.bounds
            && !excluded
            && bounds_region.is_some_and(|r| {
                !renderer::bounds_may_align(
                    object,
                    frame,
                    [r.left(), r.top(), r.right(), r.bottom()],
                )
            })
        {
            continue;
        }
        if (align_selection && excluded) || (settings.bounds && !excluded) {
            let [x0, y0, x1, y1] = object.bounds(frame);
            if excluded {
                moving_bounds.extend_with(egui::pos2(x0, y0));
                moving_bounds.extend_with(egui::pos2(x1, y1));
            } else {
                targets[0].extend([x0, (x0 + x1) * 0.5, x1]);
                targets[1].extend([y0, (y0 + y1) * 0.5, y1]);
            }
        }
        let sibling = matches!(subject, Subject::Nodes {id, ..} if id == object.id);
        if settings.anchors && (!excluded || sibling) && !moving {
            let shape = object.skinned_shape_at(frame);
            let contours = shape.contours();
            if contours.is_empty() {
                continue;
            }
            let world = object.local_to_world_at(frame);
            for (contour, nodes) in contours.iter().enumerate() {
                for (index, node) in nodes.iter().enumerate() {
                    if matches!(subject, Subject::Nodes { id, nodes } if id == object.id && nodes.contains(contour,index))
                    {
                        continue;
                    }
                    let anchor = Pos2::from(world(node.point));
                    visit_anchor(anchor);
                }
            }
        }
    }
    moving_bounds
}

fn finish(
    point: Pos2,
    settings: SnapSettings,
    radius: f32,
    source: Rect,
    targets: &[Vec<f32>; 2],
    nearest_anchor: Option<Pos2>,
) -> Hit {
    if let Some(anchor) = nearest_anchor {
        return Hit {
            point: anchor,
            anchor: Some(anchor),
            lines: [None; 2],
        };
    }
    let mut result = Hit::free(point);
    for (axis, lines) in targets.iter().enumerate() {
        let mut best = radius;
        for &line in lines {
            for value in [source.min[axis], source.center()[axis], source.max[axis]] {
                let delta = line - value;
                if delta.abs() <= radius && (result.lines[axis].is_none() || delta.abs() < best) {
                    best = delta.abs();
                    result.point[axis] = point[axis] + delta;
                    result.lines[axis] = Some(line);
                }
            }
        }
    }
    if settings.grid {
        match settings.grid_settings.kind {
            GridKind::Rectangular => {
                let snapped = settings
                    .grid_settings
                    .geometry()
                    .nearest_intersection(point.into());
                for (axis, value) in snapped.into_iter().enumerate() {
                    if result.lines[axis].is_none() {
                        result.point[axis] = value;
                    }
                }
            }
            GridKind::Axonometric if result.lines == [None; 2] => {
                result.point = settings
                    .grid_settings
                    .geometry()
                    .nearest_intersection(point.into())
                    .into();
            }
            GridKind::Axonometric => {}
        }
    }
    result
}

pub(super) fn point(
    point: Pos2,
    doc: &Document,
    frame: u32,
    settings: SnapSettings,
    scale: f32,
) -> Hit {
    resolve(point, doc, frame, settings, scale, Subject::Point)
}

/// Keep exact evaluated targets in visible paint order for Pen hover and input.
pub(super) struct PenTargets {
    source: Weak<Document>,
    frame: u32,
    modes: [bool; 3],
    bounds_region: Option<Rect>,
    geometry: Targets,
}

struct Targets {
    axes: [Vec<f32>; 2],
    anchors: Vec<Pos2>,
    moving_bounds: Rect,
}

impl Targets {
    fn new(
        doc: &Document,
        frame: u32,
        settings: SnapSettings,
        subject: Subject<'_>,
        bounds_region: Option<Rect>,
    ) -> Self {
        let objects = snap_objects(doc, settings, subject);
        let collect = |objects: &[&VectorObject], mut axes| {
            let mut anchors = Vec::new();
            let moving_bounds = gather(
                objects,
                frame,
                settings,
                subject,
                bounds_region,
                &mut axes,
                |p| {
                    if bounds_region.is_none_or(|r| r.contains(p)) {
                        anchors.push(p);
                    }
                },
            );
            Self {
                axes,
                anchors,
                moving_bounds,
            }
        };
        let axes = guide_axes(doc, settings);
        const MIN_BATCH: usize = 1024;
        if objects.len() < 2 * MIN_BATCH {
            return collect(&objects, axes);
        }
        let workers = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(4)
            .min(objects.len() / MIN_BATCH);
        if workers < 2 {
            return collect(&objects, axes);
        }
        std::thread::scope(|scope| {
            let mut chunks = objects.chunks(objects.len().div_ceil(workers));
            let first = chunks.next().unwrap();
            let jobs: Vec<_> = chunks
                .map(|chunk| {
                    let job = std::thread::Builder::new()
                        .spawn_scoped(scope, move || collect(chunk, Default::default()));
                    (chunk, job)
                })
                .collect();
            let mut targets = collect(first, axes);
            // Paint order breaks equal-distance ties. Merge in that same order.
            for (chunk, job) in jobs {
                let part = match job {
                    Ok(job) => job.join().expect("snap target worker panicked"),
                    Err(_) => collect(chunk, Default::default()),
                };
                targets.moving_bounds = targets.moving_bounds.union(part.moving_bounds);
                targets.anchors.extend(part.anchors);
                for (axis, values) in targets.axes.iter_mut().zip(part.axes) {
                    axis.extend(values);
                }
            }
            targets
        })
    }

    fn source(&self, point: Pos2, delta: Option<Vec2>) -> Rect {
        delta
            .filter(|_| self.moving_bounds.is_finite())
            .map_or(Rect::from_min_max(point, point), |d| {
                self.moving_bounds.translate(d)
            })
    }

    fn hit(&self, point: Pos2, settings: SnapSettings, scale: f32, delta: Option<Vec2>) -> Hit {
        let radius = settings.tolerance / scale;
        let mut nearest = Nearest::new(radius);
        for &anchor in &self.anchors {
            nearest.visit(point, anchor);
        }
        let source = self.source(point, delta);
        finish(point, settings, radius, source, &self.axes, nearest.anchor)
    }
}

fn modes(settings: SnapSettings) -> [bool; 3] {
    [settings.guides, settings.bounds, settings.anchors]
}

fn same_drawing(a: &Document, b: &Document) -> bool {
    std::ptr::eq(a, b) || a == b
}

/// Cover the visible pasteboard or give a captured drag outside it room to move.
fn bounds_region(settings: SnapSettings, radius: f32, visible: Rect, source: Rect) -> Option<Rect> {
    ((settings.bounds || settings.anchors) && radius.is_finite()).then(|| {
        let region = if visible.contains_rect(source) {
            visible
        } else {
            source.expand2(visible.size().max(Vec2::ZERO) * 0.5)
        };
        region.expand(2. * radius)
    })
}

impl PenTargets {
    fn covers(&self, p: Pos2, radius: f32) -> bool {
        self.bounds_region
            .is_none_or(|r| r.shrink(radius).contains(p))
    }

    /// Press/release happens before texture validation. An inspector edit may
    /// have changed the drawing since the last time it was painted.
    pub(super) fn before_paint(
        cache: Option<&Self>,
        p: Pos2,
        doc: &Document,
        frame: u32,
        settings: SnapSettings,
        scale: f32,
    ) -> Hit {
        if settings.enabled {
            if let Some(targets) = cache.filter(|c| {
                c.frame == frame
                    && c.modes == modes(settings)
                    && c.covers(p, settings.tolerance / scale)
                    && c.source
                        .upgrade()
                        .is_some_and(|source| same_drawing(&source, doc))
            }) {
                return targets.geometry.hit(p, settings, scale, None);
            }
        }
        point(p, doc, frame, settings, scale)
    }

    pub(super) fn after_paint(
        cache: &mut Option<Self>,
        p: Pos2,
        snapshot: &Arc<Document>,
        frame: u32,
        settings: SnapSettings,
        scale: f32,
        visible: Rect,
    ) -> Hit {
        if !settings.enabled || !(settings.bounds || settings.anchors) {
            return point(p, snapshot, frame, settings, scale);
        }
        let modes = modes(settings);
        let radius = settings.tolerance / scale;
        if cache.as_ref().is_some_and(|c| {
            c.frame != frame
                || c.modes != modes
                || c.source.as_ptr() != Arc::as_ptr(snapshot)
                || !c.covers(p, radius)
        }) {
            *cache = None;
        }
        cache
            .get_or_insert_with(|| {
                let bounds_region =
                    bounds_region(settings, radius, visible, Rect::from_min_max(p, p));
                Self {
                    source: Arc::downgrade(snapshot),
                    frame,
                    modes,
                    bounds_region,
                    geometry: Targets::new(
                        snapshot,
                        frame,
                        settings,
                        Subject::Point,
                        bounds_region,
                    ),
                }
            })
            .geometry
            .hit(p, settings, scale, None)
    }
}

/// The gesture owns its subject until release. The original drawing stays
/// available while the canvas paints a succession of temporary previews.
pub(super) struct DragTargets {
    original: Arc<Document>,
    frame: u32,
    modes: [bool; 3],
    bounds_region: Option<Rect>,
    targets: Targets,
}

#[cfg(test)]
mod drag_tests;

impl DragTargets {
    pub(super) fn original(&self) -> &Arc<Document> {
        &self.original
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve(
        cache: &mut Option<Self>,
        painted: Option<&Arc<Document>>,
        p: Pos2,
        doc: &Document,
        frame: u32,
        settings: SnapSettings,
        scale: f32,
        subject: Subject<'_>,
        visible: Rect,
    ) -> Hit {
        if !settings.enabled || !(settings.bounds || settings.anchors) {
            *cache = None;
            return resolve(p, doc, frame, settings, scale, subject);
        }
        let modes = modes(settings);
        let delta = match subject {
            Subject::Moving(_, d) => Some(d),
            _ => None,
        };
        if cache.as_ref().is_some_and(|c| {
            c.frame != frame || c.modes != modes || !same_drawing(&c.original, doc)
        }) {
            *cache = None;
        }
        let radius = settings.tolerance / scale;
        let cached = cache.get_or_insert_with(|| {
            let bounds_region = bounds_region(settings, radius, visible, Rect::from_min_max(p, p));
            Self {
                original: painted
                    .filter(|s| same_drawing(s, doc))
                    .cloned()
                    .unwrap_or_else(|| Arc::new(doc.clone())),
                frame,
                modes,
                bounds_region,
                targets: Targets::new(doc, frame, settings, subject, bounds_region),
            }
        });
        // The pointer can leave the viewport, and a moving group's far edge can
        // align with an offscreen target. Cover every edge as well as the center.
        let source = cached.targets.source(p, delta);
        if cached
            .bounds_region
            .is_some_and(|r| !r.shrink(radius).contains_rect(source))
        {
            cached.bounds_region = bounds_region(settings, radius, visible, source);
            cached.targets = Targets::new(doc, frame, settings, subject, cached.bounds_region);
        }
        cached.targets.hit(p, settings, scale, delta)
    }
}

pub(super) fn feedback(painter: &egui::Painter, view: super::View, hit: &Hit) {
    let color = egui::Color32::from_rgb(255, 179, 76);
    let visible = view.board.intersect(painter.clip_rect());
    if let Some(x) = hit.lines[0] {
        painter.vline(
            view.screen(egui::pos2(x, 0.)).x,
            visible.y_range(),
            (1., color),
        );
    }
    if let Some(y) = hit.lines[1] {
        painter.hline(
            visible.x_range(),
            view.screen(egui::pos2(0., y)).y,
            (1., color),
        );
    }
    if let Some(anchor) = hit.anchor {
        painter.circle_stroke(view.screen(anchor), 5., (1.5, color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::{PathNode, Shape};

    #[test]
    fn drag_aligns_a_moving_edge_to_the_other_objects_edge_in_screen_space() {
        let mut doc = Document::default();
        let moving = doc.add_rectangle([10., 10.], [10., 8.], [255; 4]);
        doc.add_rectangle([28., 40.], [12., 12.], [255; 4]);
        let settings = SnapSettings {
            enabled: true,
            grid: false,
            ..Default::default()
        };
        let raw = egui::pos2(17.1, 10.3);
        let delta = raw - egui::pos2(10., 10.);
        let ids = GestureIds::new(vec![moving]);
        let hit = resolve(raw, &doc, 0, settings, 4., Subject::Moving(&ids, delta));
        assert_eq!(hit.point, egui::pos2(18., 10.3));
        assert_eq!(hit.lines, [Some(28.), None]);
        assert_eq!(
            resolve(raw, &doc, 0, settings, 12., Subject::Moving(&ids, delta)).point,
            raw
        );
    }

    #[test]
    fn node_drag_excludes_selected_nodes_and_still_targets_siblings() {
        let mut doc = Document::default();
        let nodes = [[0., 0.], [0.5, 0.5], [1., 1.]]
            .map(PathNode::corner)
            .to_vec();
        let id = doc.add_shape(
            [8., 8.],
            [16., 16.],
            [255; 4],
            Shape::Path {
                nodes,
                closed: false,
            },
            1,
        );
        let settings = SnapSettings {
            enabled: true,
            bounds: false,
            grid: false,
            ..Default::default()
        };
        let target = |p| {
            resolve(
                p,
                &doc,
                0,
                settings,
                4.,
                Subject::Nodes {
                    id,
                    nodes: &NodeSet::range(0, 0, 1),
                },
            )
        };
        assert!(target(egui::pos2(16.1, 16.)).anchor.is_none());
        assert_eq!(
            target(egui::pos2(24.9, 24.1)).anchor,
            Some(egui::pos2(24., 24.))
        );
        doc.layers[0].visible = false;
        assert!(point(egui::pos2(24.9, 24.1), &doc, 0, settings, 4.)
            .anchor
            .is_none());
    }

    #[test]
    fn guide_uses_screen_radius_and_grid_uses_artboard_spacing() {
        let mut doc = Document::default();
        doc.guides_x.push(17.5);
        let settings = SnapSettings {
            enabled: true,
            grid_settings: GridSettings {
                spacing: 8.,
                ..Default::default()
            },
            ..Default::default()
        };
        let p = egui::pos2(18.6, 10.);
        assert_eq!(point(p, &doc, 0, settings, 4.).point, egui::pos2(17.5, 8.));
        assert_eq!(point(p, &doc, 0, settings, 8.).point, egui::pos2(16., 8.));
    }
}

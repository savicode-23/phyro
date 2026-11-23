use super::*;
use crate::timeline::KeyMode;
use document_core::{path_edit::PathEnd, Property};

#[derive(Clone, Copy, PartialEq)]
pub(in crate::canvas) struct Endpoints {
    id: u64,
    nodes: [PathNode; 2],
    len: usize,
}

impl Endpoints {
    pub(in crate::canvas) fn for_object(
        doc: &Document,
        object: &VectorObject,
        frame: u32,
        mode: KeyMode,
    ) -> Option<Self> {
        if !object.visible_at(frame)
            || !document_core::arrange::is_object_editable(doc, object)
            || mode == KeyMode::Auto
            || Property::Geometry.is_muted(object)
        {
            return None;
        }
        let nodes = object.pen_endpoints_at(frame)?;
        Some(Self {
            id: object.id,
            nodes,
            len: object.shape.contours().first()?.len(),
        })
    }

    pub(in crate::canvas) fn points(self) -> [Pos2; 2] {
        self.nodes.map(|node| Pos2::from(node.point))
    }

    pub(in crate::canvas) fn hit(self, view: View, pointer: Pos2) -> Option<PathEnd> {
        let [first, last] = self
            .points()
            .map(|point| view.screen(point).distance(pointer));
        if first.min(last) > CLOSE_RADIUS {
            return None;
        }
        Some(if first < last {
            PathEnd::Start
        } else {
            PathEnd::End
        })
    }
}

#[derive(Clone, Copy)]
pub(in crate::canvas) struct Continuation {
    source: Endpoints,
    from: PathEnd,
    index: usize,
}

impl Continuation {
    fn pick(
        doc: &Document,
        selection: &[u64],
        frame: u32,
        mode: KeyMode,
        view: View,
        pointer: Pos2,
    ) -> Option<Self> {
        let [id] = selection else {
            return None;
        };
        let index = doc.objects.iter().position(|o| o.id == *id)?;
        let source = Endpoints::for_object(doc, &doc.objects[index], frame, mode)?;
        let from = source.hit(view, pointer)?;
        Some(Self {
            source,
            from,
            index,
        })
    }

    /// Opposite and attached endpoints in the direction the artist is drawing.
    pub(in crate::canvas) fn oriented_ends(self) -> [PathNode; 2] {
        let mut ends = self.source.nodes;
        if self.from == PathEnd::Start {
            document_core::path_edit::reverse(&mut ends);
        }
        ends
    }

    fn close_target(self, draft: &[PathNode]) -> Option<PathNode> {
        let (_, added) = draft.split_first()?;
        (self.source.len + added.len() >= 3).then(|| self.oriented_ends()[0])
    }

    pub(in crate::canvas) fn target_index(
        self,
        doc: &Document,
        selection: &[u64],
        frame: u32,
        mode: KeyMode,
    ) -> Option<usize> {
        if selection != [self.source.id] {
            return None;
        }
        let index = if doc
            .objects
            .get(self.index)
            .is_some_and(|o| o.id == self.source.id)
        {
            self.index
        } else {
            doc.objects.iter().position(|o| o.id == self.source.id)?
        };
        (Endpoints::for_object(doc, &doc.objects[index], frame, mode)? == self.source)
            .then_some(index)
    }

    fn finish(
        self,
        doc: &mut Document,
        selection: &[u64],
        frame: u32,
        mode: KeyMode,
        draft: &[PathNode],
        close: Option<PathNode>,
    ) -> Option<u64> {
        let (join, added) = draft.split_first()?;
        if added.is_empty() && close.is_none() {
            return None;
        }
        let index = self.target_index(doc, selection, frame, mode)?;
        let object = &mut doc.objects[index];
        object
            .extend_open_path(
                frame,
                self.from,
                join.outgoing,
                added,
                close.map(|n| n.incoming),
            )
            .then_some(object.id)
    }
}

impl Canvas {
    pub(in crate::canvas) fn pen_angle_target(&self, point: Pos2, ctrl: bool) -> Option<Pos2> {
        let origin = Pos2::from(self.pen.last().filter(|_| ctrl)?.point);
        let delta = point - origin;
        let step = 15_f32.to_radians();
        let nearest = (delta.angle() / step).round();
        let direction = match (nearest as i32).rem_euclid(24) {
            0 => egui::vec2(1., 0.),
            6 => egui::vec2(0., 1.),
            12 => egui::vec2(-1., 0.),
            18 => egui::vec2(0., -1.),
            _ => Vec2::angled(nearest * step),
        };
        Some(origin + direction * delta.dot(direction))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::canvas) fn start_pen(
        &mut self,
        doc: &Document,
        selection: &mut Vec<u64>,
        frame: u32,
        style: &CanvasStyle,
        view: View,
        pointer: Pos2,
        ctrl: bool,
    ) -> Option<PenDrag> {
        let world = view.world(pointer);
        if self.pen.is_empty() {
            if let Some(path) =
                Continuation::pick(doc, selection, frame, style.key_mode, view, pointer)
            {
                self.clear_pen_redo();
                self.continuation = Some(path);
                return Some(PenDrag::resume(path.oriented_ends()[1], world));
            }
        }
        if self.continuation.is_none() && !is_editable(doc, style.layer) {
            return None;
        }
        if let Some(first) = self.close_pen_target(view, pointer) {
            return Some(PenDrag::close(first, world));
        }
        if let Some(path) = self.continuation {
            if path.source.len + self.pen.len().saturating_sub(1) >= document_core::MAX_PATH_NODES {
                return None;
            }
        } else if self.pen.is_empty() {
            self.clear_pen_redo();
            selection.clear();
        }
        let anchor = self.pen_angle_target(world, ctrl).unwrap_or_else(|| {
            snapping::PenTargets::before_paint(
                self.pen_targets.as_ref(),
                world,
                doc,
                frame,
                style.snap,
                view.scale,
            )
            .point
        });
        Some(PenDrag::new(anchor, world))
    }

    pub(in crate::canvas) fn close_pen_target(
        &self,
        view: View,
        pointer: Pos2,
    ) -> Option<PathNode> {
        let node = if let Some(path) = self.continuation {
            path.close_target(&self.pen)?
        } else {
            if self.pen.len() < 3 {
                return None;
            }
            self.pen[0]
        };
        (view.screen(node.point.into()).distance(pointer) <= CLOSE_RADIUS).then_some(node)
    }

    pub(in crate::canvas) fn finish_pen_draft(
        &mut self,
        doc: &mut Document,
        frame: u32,
        selection: &[u64],
        style: &CanvasStyle,
        close: Option<PathNode>,
    ) -> Option<u64> {
        if let Some(path) = self.continuation.take() {
            let id = path.finish(doc, selection, frame, style.key_mode, &self.pen, close);
            self.pen.clear();
            self.clear_pen_redo();
            return id;
        }
        if let Some(node) = close {
            *self.pen.first_mut()? = node;
        }
        let id = finish_pen(&mut self.pen, close.is_some(), doc, style);
        if self.pen.is_empty() {
            self.clear_pen_redo();
        }
        id
    }
}

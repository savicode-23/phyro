use super::*;

impl Canvas {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_node_marquee(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        frame: u32,
        view: View,
        pointer: Pos2,
        release: bool,
        selection: &mut Vec<u64>,
        scope: SelectionScope,
    ) {
        let Some(Gesture::NodeMarquee {
            id,
            start,
            previous,
            preview,
        }) = &mut self.gesture
        else {
            return;
        };
        let point = view.world(pointer);
        if point.distance(*start) * view.scale < 2. && preview.is_none() {
            if release {
                *selection = doc
                    .hit_test_in(point.into(), frame, scope)
                    .into_iter()
                    .collect();
                self.gesture = None;
            }
            return;
        }
        let rect = Rect::from_two_pos(*start, point);
        if !release && preview.as_ref().is_some_and(|(last, _)| *last == rect) {
            return;
        }
        let Some(object) = doc.objects.iter().find(|o| {
            o.id == *id && o.visible_at(frame) && document_core::arrange::is_object_editable(doc, o)
        }) else {
            self.gesture = None;
            return;
        };
        let inside = crate::path_editor::NodeSet::in_box(object, frame, rect, previous.as_ref());
        if release {
            crate::path_editor::set_nodes(ctx, *id, frame, inside);
        } else {
            *preview = Some((rect, inside));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn node_click(
        &mut self,
        response: &egui::Response,
        doc: &mut Arc<Document>,
        frame: u32,
        view: View,
        selection: &mut Vec<u64>,
        mods: egui::Modifiers,
        mode: crate::timeline::KeyMode,
        scope: SelectionScope,
    ) -> bool {
        if let Some(Gesture::Node {
            id,
            edit,
            cycle_on_click: Some(index),
            ..
        }) = &self.gesture
        {
            if !edit.has_dragged()
                && response
                    .ctx
                    .input(|i| primary_button(i, false))
                    .is_some_and(|(at, _)| response.interact_rect.contains(at))
            {
                let (id, index) = (*id, *index);
                self.gesture = None;
                self.last_node_click = None;
                return crate::path_editor::styles::cycle_at(
                    Arc::make_mut(doc),
                    id,
                    frame,
                    mode,
                    index.contour,
                    index.node,
                );
            }
        }
        if matches!(
            self.gesture,
            Some(Gesture::Node {
                edit: paths::NodeDrag::Segment { dragged: false, .. },
                ..
            })
        ) {
            if let Some((at, _)) = response.ctx.input(|i| primary_button(i, false)) {
                // A click still picks painted artwork. Only a drag claims the nearby curve.
                *selection = doc
                    .hit_test_in(view.world(at).into(), frame, scope)
                    .into_iter()
                    .collect();
                self.gesture = None;
            }
        }
        let previous = self.last_node_click.take();
        if self.gesture.is_some()
            || mods != egui::Modifiers::NONE
            || !response.clicked_by(egui::PointerButton::Primary)
        {
            return false;
        }
        let Some(at) = response
            .interact_pointer_pos()
            .filter(|p| response.interact_rect.contains(*p))
        else {
            return false;
        };
        let ctx = &response.ctx;
        let time = ctx.input(|i| i.time);
        let id = selection.last().copied();
        // egui counts clicks by time across all widgets and positions. Pair
        // only nearby primary clicks on this canvas, using its configured thresholds.
        let Some((_, _, first_id)) = previous.filter(|(first, then, target)| {
            target.zip(id).is_none_or(|(a, b)| a == b)
                && ctx.options(|o| {
                    at.distance(*first) <= o.input_options.max_click_dist
                        && time - then < o.input_options.max_double_click_delay
                })
        }) else {
            self.last_node_click = Some((at, time, id));
            return false;
        };
        let point = view.world(at);
        let radius = 7. / view.scale;
        let picked = id.or(first_id);
        let Some((id, node)) = self.split_edge(doc, frame, point, radius, picked, scope) else {
            return false;
        };
        *selection = vec![id];
        crate::path_editor::select(ctx, id, frame, node.contour, node.node);
        true
    }

    fn split_edge(
        &mut self,
        doc: &mut Arc<Document>,
        frame: u32,
        point: Pos2,
        radius: f32,
        picked: Option<u64>,
        scope: SelectionScope,
    ) -> Option<(u64, paths::NodeIndex)> {
        let mut floor = None;
        if let Some(id) = picked {
            let index = doc.objects.iter().position(|o| o.id == id)?;
            let layer = doc.objects[index].layer;
            let order = doc
                .layers
                .iter()
                .position(|l| l.id == layer && l.visible && !l.locked)?;
            if let Some(hit) = self.split_visible(doc, frame, point, radius, index) {
                return Some(hit);
            }
            // Near a thin foreground path, an ordinary click may pick the
            // backdrop. Search above those painted pixels, never behind them.
            floor = Some((order, index));
        }
        let layers: std::collections::BTreeMap<_, _> = doc
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.visible && !l.locked)
            .map(|(order, l)| (l.id, order))
            .collect();
        let mut candidates: Vec<_> = doc
            .objects
            .iter()
            .enumerate()
            .filter_map(|(i, o)| {
                if !scope.contains(o) {
                    return None;
                }
                let order = (*layers.get(&o.layer)?, i);
                if floor.is_some_and(|covered| order <= covered) {
                    return None;
                }
                if matches!(o.shape, Shape::Rectangle | Shape::Ellipse) {
                    // Even after skew and rotation every canonical primitive control
                    // lies inside the transformed unit box. Authored paths can extend outside it.
                    let world = o.local_to_world_at(frame);
                    let corners =
                        [[0., 0.], [1., 0.], [1., 1.], [0., 1.]].map(|p| Pos2::from(world(p)));
                    let bounds = Rect::from_points(&corners);
                    // Leave rounding room for the separately transformed controls.
                    let largest = bounds
                        .min
                        .to_vec2()
                        .abs()
                        .max(bounds.max.to_vec2().abs())
                        .max_elem();
                    if bounds.is_finite()
                        && !bounds
                            .expand(radius + largest * f32::EPSILON * 8.)
                            .contains(point)
                    {
                        return None;
                    }
                }
                Some(order)
            })
            .collect();
        candidates.sort_unstable();
        candidates
            .into_iter()
            .rev()
            .find_map(|(_, index)| self.split_visible(doc, frame, point, radius, index))
    }

    fn split_visible(
        &mut self,
        doc: &mut Arc<Document>,
        frame: u32,
        point: Pos2,
        radius: f32,
        index: usize,
    ) -> Option<(u64, paths::NodeIndex)> {
        let o = &doc.objects[index];
        if o.locked
            || !o.visible_at(frame)
            || o.opacity_at(frame) <= 0.
            || (!o.has_fill_at(frame) && !o.has_stroke_at(frame))
            || !o.effects.is_empty()
            || o.rigid_bone.is_some()
            || !o.stencil_contains(point.into(), frame)
        {
            return None;
        }
        let id = o.id;
        self.insert_node(doc, frame, index, point, radius, true, true)
            .map(|n| (id, n))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn insert_node(
        &mut self,
        doc: &mut Arc<Document>,
        frame: u32,
        index: usize,
        point: Pos2,
        radius: f32,
        allow_primitive: bool,
        single: bool,
    ) -> Option<paths::NodeIndex> {
        let painted = self.texture.as_mut().filter(|cached| {
            single
                && self.refinement.is_none()
                && cached.can_refine_split(doc, frame, &doc.objects[index])
        });
        let node = paths::insert_near(
            &mut Arc::make_mut(doc).objects[index],
            frame,
            point,
            radius,
            allow_primitive,
        )?;
        if let Some(cached) = painted {
            cached.pending_split = Some(Arc::clone(doc));
        }
        Some(node)
    }
}

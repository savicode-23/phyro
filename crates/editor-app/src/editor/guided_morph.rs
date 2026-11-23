use super::Editor;
use document_core::morph::{MorphGuide, MorphPlan, MorphPoint};
use document_core::{
    path_edit::primitive_path, Clip, Document, Easing, Property, Shape, VectorObject, MAX_FRAMES,
};
use eframe::egui::{self, Ui};
use std::sync::Arc;

mod preview;
mod workspace;

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Key,
    Artwork(u64),
}

pub(super) struct MorphDialog {
    object: u64,
    start: u32,
    end: u32,
    exposure: (u32, u32),
    editing: bool,
    pair: Option<[u64; 2]>,
    hide_end_shape: bool,
    target: Option<Target>,
    artwork: Vec<(u64, String)>,
    from: Shape,
    to: Option<Shape>,
    guides: Vec<MorphGuide>,
    pending: Option<MorphPoint>,
    plan: Option<Result<Arc<MorphPlan>, String>>,
    saved_plan: Option<(Vec<MorphGuide>, Arc<MorphPlan>)>,
    progress: f32,
    between: Option<Shape>,
    previews: [preview::OutlineCache; 3],
    easing: Easing,
    error: Option<String>,
    workspace: workspace::Workspace,
    background: Option<(egui::Rect, f32, egui::layers::PaintList)>,
}

fn outline_closure(object: &VectorObject) -> Option<bool> {
    if object.can_guided_morph() {
        if let Shape::Path { closed, .. } = &object.shape {
            return Some(*closed);
        }
    }
    (matches!(object.shape, Shape::Rectangle | Shape::Ellipse)
        && object.text.is_none()
        && object.image.is_none()
        && !object.is_instance()
        && object.skin.is_none()
        && object.stroke_profile.is_empty()
        && object.effects.is_empty()
        && object.animated.geometry_keys.is_empty())
    .then_some(true)
}

pub(super) fn pair_eligible(doc: &Document, source: u64, target: u64) -> bool {
    let closure = |id| {
        doc.objects
            .iter()
            .find(|o| o.id == id)
            .filter(|o| crate::commands::is_editable(doc, o.layer))
            .and_then(outline_closure)
    };
    source != target
        && closure(source)
            .zip(closure(target))
            .is_some_and(|(a, b)| a == b)
}

impl Editor {
    pub(super) fn open_guided_morph(&mut self, id: u64) {
        self.open_morph(id, None);
    }

    pub(super) fn open_selection_morph(&mut self, source: u64, target: u64) {
        self.open_morph(source, Some(target));
    }

    fn open_morph(&mut self, source: u64, target: Option<u64>) {
        self.finish_edit();
        self.timeline.stop();
        self.canvas.cancel_drag();
        let clip = &self.project.clips[self.timeline.clip];
        match MorphDialog::new(clip, self.timeline.frame, source, target) {
            Ok(dialog) => self.guided_morph = Some(dialog),
            Err(error) => self.report_error("Could not open guided morph", error),
        }
    }

    pub(super) fn show_morph_background(&mut self, ctx: &egui::Context) {
        // The document stays still during the preview. Replay the panels and
        // artboard without laying out the timeline or visiting every object.
        let screen = ctx.screen_rect();
        let scale = ctx.pixels_per_point();
        let layer = egui::LayerId::background();
        if let Some((rect, ppp, paint)) = self
            .guided_morph
            .as_ref()
            .and_then(|m| m.background.as_ref())
        {
            if *rect == screen && *ppp == scale {
                ctx.graphics_mut(|graphics| graphics.entry(layer).clone_from(paint));
                return;
            }
        }

        self.show_passive_workspace(ctx);
        if let Some(dialog) = &mut self.guided_morph {
            dialog.background = ctx.graphics(|graphics| {
                graphics
                    .get(layer)
                    .cloned()
                    .map(|paint| (screen, scale, paint))
            });
        }
    }

    pub(super) fn show_guided_morph(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.guided_morph.as_mut() else {
            return;
        };
        let clip = &self.project.clips[self.timeline.clip];
        let doc = clip.drawing_at(dialog.start);
        let Some(object) = doc.objects.iter().find(|o| o.id == dialog.object) else {
            self.guided_morph = None;
            return;
        };
        let popup_open = ctx.memory(|m| m.any_popup_open());
        let (done, cancel) = dialog.window(ctx, doc, object, clip.fps);
        if done {
            let result = dialog.commit(&mut self.project.clips[self.timeline.clip]);
            match result {
                Ok(()) => {
                    self.status = format!(
                        "Applied guided morph on frames {}–{}.",
                        dialog.start + 1,
                        dialog.end + 1
                    );
                    self.timeline.frame = self.timeline.frame.clamp(dialog.start, dialog.end);
                    let object = dialog.object;
                    self.guided_morph = None;
                    self.select_artwork(vec![object]);
                    self.sync_paint();
                    self.record_synced_edit(Ok(()));
                }
                Err(error) => dialog.error = Some(error),
            }
        } else if cancel
            || (!popup_open
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)))
        {
            self.guided_morph = None;
        }
    }
}

impl MorphDialog {
    fn new(clip: &Clip, frame: u32, id: u64, donor: Option<u64>) -> Result<Self, String> {
        let doc = clip.drawing_at(frame);
        if donor.is_some_and(|to| !pair_eligible(doc, id, to)) {
            return Err("Select two editable outlines, both open or both closed.".into());
        }
        let o = doc
            .objects
            .iter()
            .find(|o| {
                o.id == id
                    && outline_closure(o).is_some()
                    && crate::commands::is_editable(doc, o.layer)
            })
            .ok_or("Select one editable single-outline path.")?;
        let (mut first, duration) = clip.layer_exposure(frame, o.layer);
        let mut last = first + duration - 1;
        if let Some(other) = donor.and_then(|id| doc.objects.iter().find(|obj| obj.id == id)) {
            let (start, length) = clip.layer_exposure(frame, other.layer);
            first = first.max(start);
            last = last.min(start + length - 1);
        }
        if donor.is_some() && frame == last && frame + 1 == clip.frames {
            last = (last + clip.fps).min(MAX_FRAMES - 1);
        }
        if last <= first {
            return Err(
                "Extend this drawing’s Hold to at least two frames to create a morph.".into(),
            );
        }
        let keys = &o.animated.geometry_keys;
        let existing = o
            .guided_morph_span(frame)
            .filter(|&(a, b)| donor.is_none() && a >= first && b <= last);
        let (start, end) = if donor.is_some() && frame < last {
            let limit = (frame + clip.fps).min(last);
            let end = keys
                .range(frame + 1..=limit)
                .next()
                .map_or(limit, |(&t, _)| t);
            (frame, end)
        } else if donor.is_some() {
            let earliest = frame.saturating_sub(clip.fps).max(first);
            (
                keys.range(earliest..frame)
                    .next_back()
                    .map_or(earliest, |(&t, _)| t),
                frame,
            )
        } else {
            existing.unwrap_or_else(|| {
                if frame == first {
                    let end = keys
                        .range((first + 1)..=last)
                        .next()
                        .map(|(&f, _)| f)
                        .unwrap_or((first + (clip.fps / 2).max(1)).min(last));
                    (first, end)
                } else {
                    let start = keys
                        .range(first..frame)
                        .next_back()
                        .map_or(first, |(&f, _)| f);
                    (start, frame)
                }
            })
        };
        let closed = outline_closure(o);
        let artwork: Vec<_> = if donor.is_some() {
            Vec::new()
        } else {
            doc.objects
                .iter()
                .filter(|other| other.id != id && outline_closure(other) == closed)
                .map(|other| (other.id, other.display_name()))
                .collect()
        };
        let target = if let Some(target) = donor {
            Some(Target::Artwork(target))
        } else if keys.contains_key(&end) {
            Some(Target::Key)
        } else if artwork.len() == 1 {
            Some(Target::Artwork(artwork[0].0))
        } else {
            None
        };
        let mut dialog = Self {
            object: id,
            start,
            end,
            exposure: (first, last),
            editing: existing.is_some(),
            pair: donor.map(|to| [id, to]),
            hide_end_shape: donor.is_some_and(|to| {
                doc.objects
                    .iter()
                    .find(|o| o.id == to)
                    .is_some_and(VectorObject::frame_independent)
            }),
            target,
            artwork,
            from: Shape::default(),
            to: None,
            pending: None,
            guides: if existing.is_some() {
                o.morph_guides(start, end).unwrap_or_default().to_vec()
            } else {
                Vec::new()
            },
            plan: None,
            saved_plan: None,
            progress: 0.,
            between: None,
            previews: Default::default(),
            easing: Easing::Linear,
            error: None,
            workspace: Default::default(),
            background: None,
        };
        dialog.reload(doc, o);
        Ok(dialog)
    }

    fn reload(&mut self, doc: &Document, object: &VectorObject) {
        self.previews = Default::default();
        self.easing = Property::Geometry.easing_at(object, self.start);
        self.from = object
            .animated
            .geometry_keys
            .get(&self.start)
            .cloned()
            .unwrap_or_else(|| object.shape_at(self.start).into_owned());
        if let Some(path) = primitive_path(&self.from) {
            self.from = path;
        }
        self.to = match self.target {
            Some(Target::Key) => object.animated.geometry_keys.get(&self.end).cloned(),
            Some(Target::Artwork(id)) => doc
                .objects
                .iter()
                .find(|o| o.id == id)
                .map(|other| capture_target(object, other, self.end)),
            None => None,
        };
        self.pending = None;
        self.saved_plan = if self.target == Some(Target::Key) {
            object
                .guided_morph_plan(self.start, self.end)
                .map(|plan| (self.guides.clone(), plan))
        } else {
            None
        };
        self.error = None;
        self.rebuild();
        if object
            .animated
            .geometry_keys
            .range((self.start + 1)..self.end)
            .next()
            .is_some()
        {
            self.error =
                Some("A Geometry key lies between these frames. Choose adjacent poses.".into());
        }
        if Property::Geometry.is_muted(object) {
            self.error = Some(
                "Unmute the source’s Geometry track in the timeline to use this morph.".into(),
            );
        }
    }

    fn rebuild(&mut self) {
        self.workspace.reset_previews();
        self.plan = self.to.as_ref().map(|to| {
            if let Some((guides, plan)) = &self.saved_plan {
                if *guides == self.guides {
                    return Ok(plan.clone());
                }
            }
            MorphPlan::build(&self.from, to, &self.guides).map(Arc::new)
        });
        self.sample_preview();
    }

    fn sample_preview(&mut self) {
        self.previews[2] = Default::default();
        let t = self.easing.sample(f64::from(self.progress));
        self.between = match self.plan.as_ref().and_then(|result| result.as_ref().ok()) {
            Some(_) if t == 0. => Some(self.from.clone()),
            Some(_) if t == 1. => self.to.clone(),
            Some(plan) => Some(plan.sample(t)),
            None => None,
        };
    }

    fn ready(&self) -> bool {
        self.error.is_none() && self.pending.is_none() && matches!(self.plan, Some(Ok(_)))
    }

    fn apply(&self, object: &mut VectorObject) -> Result<(), String> {
        if self.editing && self.target == Some(Target::Key) {
            object.set_morph_guides(self.start, self.end, self.guides.clone())
        } else {
            let target = self.to.as_ref().ok_or("Choose the end pose.")?;
            if let Some(path) = primitive_path(&object.shape) {
                object.shape = path;
            }
            object.set_guided_morph(
                self.start,
                self.end,
                self.from.clone(),
                target.clone(),
                self.guides.clone(),
            )
        }
    }

    fn commit(&self, clip: &mut Clip) -> Result<(), String> {
        // Stage length, keys, primitive conversion, and donor visibility in one undo step.
        let mut staged = clip.clone();
        if self.end >= staged.frames {
            staged.set_length(self.end + 1)?;
        }
        let before = (!staged.layer_starts.is_empty()).then(|| staged.clone());
        let drawing = staged.drawing_at_mut(self.start);
        let source = drawing
            .objects
            .iter_mut()
            .find(|o| o.id == self.object)
            .ok_or("The selected shape is missing.")?;
        self.apply(source)?;
        if let (Some([_, id]), true) = (self.pair, self.hide_end_shape) {
            let donor = drawing
                .objects
                .iter_mut()
                .find(|o| o.id == id)
                .ok_or("The end shape is missing.")?;
            if donor.animated.visible_keys.is_empty() {
                donor.animated.visible = false;
            }
        }
        if let Some(before) = before {
            staged.sync_layer_edits(&before, self.start)?;
        }
        *clip = staged;
        Ok(())
    }

    fn swap_poses(&mut self, doc: &Document) {
        let Some([old_source, new_source]) = self.pair else {
            return;
        };
        let Some(source) = doc.objects.iter().find(|o| o.id == new_source) else {
            return;
        };
        self.object = new_source;
        self.pair = Some([new_source, old_source]);
        self.target = Some(Target::Artwork(old_source));
        for guide in &mut self.guides {
            std::mem::swap(&mut guide.from, &mut guide.to);
        }
        self.hide_end_shape = doc
            .objects
            .iter()
            .find(|o| o.id == old_source)
            .is_some_and(VectorObject::frame_independent);
        self.reload(doc, source);
    }

    fn settings(&mut self, ui: &mut Ui, doc: &Document, object: &VectorObject) {
        let old = (self.start, self.end, self.target);
        let (first, last) = self.exposure;
        ui.horizontal_wrapped(|ui| {
            if self.editing {
                ui.label(format!("Frames {}–{}", self.start + 1, self.end + 1));
            } else {
                ui.label("From frame");
                let mut start = self.start + 1;
                ui.add(
                    egui::DragValue::new(&mut start)
                        .range((first + 1)..=self.end)
                        .update_while_editing(false),
                );
                self.start = start - 1;
                ui.label("to");
                let mut end = self.end + 1;
                ui.add(
                    egui::DragValue::new(&mut end)
                        .range((self.start + 2)..=(last + 1))
                        .update_while_editing(false),
                );
                self.end = end - 1;
            }
        });
        if let Some([_, id]) = self.pair {
            if let Some(donor) = doc.objects.iter().find(|o| o.id == id) {
                ui.add_enabled(
                    donor.animated.visible_keys.is_empty(),
                    egui::Checkbox::new(&mut self.hide_end_shape, "Hide end shape"),
                )
                .on_hover_text(
                    "Keep the original shape in Layers and hide it in this drawing after Done.",
                )
                .on_disabled_hover_text("This shape already has Visibility keys.");
            }
        } else {
            self.choose_target(ui, object);
        }
        if old != (self.start, self.end, self.target) {
            if old.2 != self.target || self.target == Some(Target::Key) {
                self.guides.clear();
                if self.target == Some(Target::Key) {
                    self.guides.extend_from_slice(
                        object
                            .morph_guides(self.start, self.end)
                            .unwrap_or_default(),
                    );
                }
            }
            self.reload(doc, object);
        }
        if !object.effects.is_empty() {
            ui.small("Match points on the authored outline. Its live path effects apply after the morph.");
        }
    }

    fn choose_target(&mut self, ui: &mut Ui, object: &VectorObject) {
        let has_key = object.animated.geometry_keys.contains_key(&self.end);
        if self.target == Some(Target::Key) && !has_key {
            self.target = None;
        }
        let target_name = match self.target {
            Some(Target::Key) => "Existing Geometry key",
            Some(Target::Artwork(id)) => self
                .artwork
                .iter()
                .find(|(other, _)| *other == id)
                .map_or("Choose a path", |(_, name)| name.as_str()),
            None => "Choose a path",
        };
        ui.horizontal_wrapped(|ui| {
            ui.label("End pose");
            egui::ComboBox::from_id_salt("morph-target")
                .selected_text(target_name)
                .show_ui(ui, |ui| {
                    if has_key {
                        ui.selectable_value(
                            &mut self.target,
                            Some(Target::Key),
                            "Existing Geometry key",
                        );
                    }
                    for (id, name) in &self.artwork {
                        ui.selectable_value(&mut self.target, Some(Target::Artwork(*id)), name);
                    }
                });
        });
        if self.to.is_none() {
            ui.small("Choose another open or closed outline of the same kind.");
        }
    }
}

/// The destination may have a different origin, size, flip, rotation and skew.
/// Capture all three Bezier points in the source's local space at the end frame.
fn capture_target(source: &VectorObject, donor: &VectorObject, frame: u32) -> Shape {
    let mut shape = donor.evaluated_shape_at(frame).into_owned();
    if let Some(path) = primitive_path(&shape) {
        shape = path;
    }
    let world = donor.local_to_world_at(frame);
    let local = source.world_to_local_at(frame);
    for nodes in shape.contours_mut() {
        for node in nodes {
            for point in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                *point = local(world(*point));
            }
        }
    }
    shape
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::AnimationProject;

    fn two_shapes(frames: u32) -> (Clip, u64, u64) {
        let mut scene = Document {
            frames,
            fps: 12,
            ..Default::default()
        };
        let left = scene.add_rectangle([4., 4.], [16., 20.], [255; 4]);
        let right = scene.add_shape([12., 6.], [23., 20.], [255; 4], Shape::Ellipse, 1);
        (
            AnimationProject::from_document(scene).clips.pop().unwrap(),
            left,
            right,
        )
    }

    #[test]
    fn morph_plays_across_an_unrelated_layers_drawings() {
        let (mut clip, from, to) = two_shapes(24);
        let doc = clip.drawing_at_mut(0);
        doc.layers.push(document_core::Layer {
            id: 2,
            name: "Sparks".into(),
            ..Default::default()
        });
        doc.add_shape([2.; 2], [5.; 2], [255; 4], Shape::Ellipse, 2);
        clip.blank_layer_frame(1, 2).unwrap();
        let dialog = MorphDialog::new(&clip, 0, from, Some(to)).unwrap();
        assert_eq!((dialog.start, dialog.end), (0, 12));
        dialog.commit(&mut clip).unwrap();
        for tick in [0, 1, 6, 12] {
            assert!(clip.drawing_at(tick).objects[0]
                .morph_guides(0, 12)
                .is_some());
        }
        assert!(clip.layer_frame_is_blank(1, 2));
    }

    #[test]
    fn two_shape_timing_respects_the_next_drawing_and_preserves_donor_animation() {
        let (mut clip, start, end) = two_shapes(24);
        let donor = &mut clip.drawing_at_mut(0).objects[1];
        donor.keys.insert(8, [34., 28.]);
        donor.animated.visible_keys.insert(2, true);
        donor.animated.visible_keys.insert(8, false);
        let original_donor = donor.clone();
        let other_cel = clip.cels[&0].clone();
        clip.cels.insert(12, other_cel.clone());
        let dialog = MorphDialog::new(&clip, 8, start, Some(end)).unwrap();
        assert_eq!((dialog.start, dialog.end), (8, 11));
        assert!(!dialog.hide_end_shape);
        dialog.commit(&mut clip).unwrap();
        assert!(Arc::ptr_eq(&clip.cels[&12], &other_cel));
        assert_eq!(clip.drawing_at(8).objects[1], original_donor);
        assert_eq!(clip.frames, 24);
    }

    #[test]
    fn reversing_a_pair_keeps_its_matches_and_hiding_the_donor_is_optional() {
        let (mut clip, rectangle, ellipse) = two_shapes(25);
        let mut dialog = MorphDialog::new(&clip, 0, rectangle, Some(ellipse)).unwrap();
        let pair = MorphGuide {
            from: MorphPoint { segment: 0, t: 0.3 },
            to: MorphPoint { segment: 1, t: 0.6 },
        };
        dialog.guides.push(pair);
        dialog.swap_poses(clip.drawing_at(0));
        assert_eq!(
            dialog.guides,
            [MorphGuide {
                from: pair.to,
                to: pair.from
            }]
        );
        dialog.hide_end_shape = false;
        dialog.commit(&mut clip).unwrap();
        let doc = clip.drawing_at(0);
        assert!(doc.objects[0].visible_at(6));
        assert_eq!(doc.objects[0].shape, Shape::Rectangle);
        assert!(doc.objects[1].morph_guides(0, 12).is_some());
    }
}

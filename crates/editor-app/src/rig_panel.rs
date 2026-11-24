mod handles;
mod ik;
mod skin;

use crate::timeline::KeyMode;
use document_core::{Clip, Document, Interpolation};
use eframe::egui::{self, Color32, Pos2, Ui};
use std::sync::Arc;

#[derive(Clone, Copy)]
enum Pick {
    Head,
    Tip,
}

struct Draft {
    parent: Option<u64>,
    name: String,
    head: [f32; 2],
    tail: [f32; 2],
    pick: Pick,
}

pub(super) struct RigPanel {
    drawing: Option<(u64, u32)>,
    selected: Option<u64>,
    draft: Option<Draft>,
    ik_target: Option<u64>,
    ik_flip: bool,
    pose_handles: bool,
    handle_drag: Option<handles::HandleDrag>,
    weight_path: Option<u64>,
    inspector_open: bool,
    visible: bool,
    error: Option<String>,
}

impl Default for RigPanel {
    fn default() -> Self {
        Self {
            drawing: None,
            selected: None,
            draft: None,
            ik_target: None,
            ik_flip: false,
            pose_handles: true,
            handle_drag: None,
            weight_path: None,
            inspector_open: false,
            visible: true,
            error: None,
        }
    }
}

impl RigPanel {
    pub fn selected_bone(&self) -> Option<(u64, u32, u64)> {
        let (session, exposure) = self.drawing?;
        Some((session, exposure, self.selected?))
    }

    pub fn set_drawing(&mut self, clip_session: u64, exposure: u32) {
        let key = Some((clip_session, exposure));
        if self.drawing != key {
            self.drawing = key;
            self.selected = None;
            self.cancel_placement();
        }
    }

    pub fn cancel_placement(&mut self) {
        self.draft = None;
        self.ik_target = None;
        self.cancel_handle_drag();
        self.error = None;
    }

    pub fn leave_inspector(&mut self) {
        self.inspector_open = false;
        self.weight_path = None;
        self.cancel_placement();
    }

    pub fn placing(&self) -> bool {
        self.draft.is_some() || self.ik_target.is_some()
    }

    fn begin(&mut self, doc: &Document, parent: Option<u64>) {
        self.cancel_placement();
        let (head, tail) = if let Some(id) = parent {
            let Ok((a, b)) = doc.bone_segment(id, false) else {
                return;
            };
            (b, [b[0] + b[0] - a[0], b[1] + b[1] - a[1]])
        } else {
            let x = doc.width as f32 * 0.5;
            let y = doc.height as f32 * 0.5;
            ([x, y], [x + (doc.width as f32 * 0.2).max(1.), y])
        };
        let base = if parent.is_none() { "Root" } else { "Bone" };
        let existing = |name: &str| {
            doc.skeleton
                .bones
                .iter()
                .any(|b| b.name.eq_ignore_ascii_case(name))
        };
        let name = if existing(base) {
            (2..)
                .map(|i| format!("{base} {i}"))
                .find(|n| !existing(n))
                .unwrap()
        } else {
            base.to_owned()
        };
        self.draft = Some(Draft {
            parent,
            name,
            head,
            tail,
            pick: if parent.is_some() {
                Pick::Tip
            } else {
                Pick::Head
            },
        });
        self.visible = true;
        self.error = None;
    }

    fn create(&mut self, doc: &mut Document) {
        let Some(d) = &self.draft else { return };
        match doc.add_bone(d.name.trim(), d.parent, d.head, d.tail) {
            Ok(id) => {
                self.selected = Some(id);
                self.cancel_placement();
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Use the canvas' normal world-coordinate mapping after pan and zoom.
    pub fn placement_input(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        point: Option<[f32; 2]>,
        doc: &mut Arc<Document>,
        frame: u32,
    ) {
        if response.secondary_clicked()
            || (!ui.ctx().wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Escape)))
        {
            self.cancel_placement();
            return;
        }
        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        if let Some(id) = self.ik_target {
            if let Some(point) = point {
                self.apply_ik(Arc::make_mut(doc), id, frame, point);
            }
            return;
        }
        let (Some(point), Some(draft)) = (point, self.draft.as_mut()) else {
            return;
        };
        self.error = None;
        let point = match draft.parent {
            Some(id) => match doc.bone_rest_point_at(id, point, frame) {
                Ok(rest) => rest,
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            },
            None => point,
        };
        match draft.pick {
            Pick::Head => {
                let delta = [draft.tail[0] - draft.head[0], draft.tail[1] - draft.head[1]];
                draft.head = point;
                draft.tail = [point[0] + delta[0], point[1] + delta[1]];
                draft.pick = Pick::Tip;
            }
            Pick::Tip => {
                draft.tail = point;
                self.create(Arc::make_mut(doc));
            }
        }
    }

    fn bind(&mut self, doc: &mut Arc<Document>, selection: &[u64], bone: Option<u64>, frame: u32) {
        // A selection can span layers. Commit all links together only after every object is editable.
        let mut staged = (**doc).clone();
        for &id in selection {
            if let Err(e) = staged.bind_rigid_at(id, bone, frame) {
                self.error = Some(e);
                return;
            }
        }
        *doc = Arc::new(staged);
        self.error = None;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        clip: &mut Clip,
        selection: &[u64],
        frame: u32,
        mode: KeyMode,
    ) {
        let doc = clip.drawing_snapshot_mut(frame);
        let mut foot_contact = None;
        self.weight_path = None;
        if self
            .selected
            .is_some_and(|id| !doc.skeleton.bones.iter().any(|b| b.id == id))
        {
            self.selected = None;
            self.cancel_placement();
        }
        if self
            .draft
            .as_ref()
            .and_then(|d| d.parent)
            .is_some_and(|id| !doc.skeleton.bones.iter().any(|b| b.id == id))
        {
            self.cancel_placement();
        }
        let header = egui::CollapsingHeader::new("Skeleton")
            .default_open(true)
            .show(ui, |ui| {
                if !doc.skeleton.bones.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.visible, "Show bones");
                        ui.add_enabled(
                            self.visible,
                            egui::Checkbox::new(&mut self.pose_handles, "Pose handles"),
                        )
                        .on_hover_text("Drag a joint’s tip to pose its two-bone chain. Shift-drag reverses the bend.");
                    });
                }
                if self.draft.is_some() {
                    self.show_draft(ui, doc);
                } else {
                    if !doc.skeleton.bones.is_empty() {
                        foot_contact = self.show_pose(ui, doc, selection, frame, mode);
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .button("Add root bone")
                            .on_hover_text("Place a bone with no parent")
                            .clicked()
                        {
                            self.begin(doc, None);
                        }
                        if ui
                            .add_enabled(
                                self.selected.is_some(),
                                egui::Button::new("Add child bone"),
                            )
                            .on_hover_text("Start a child at the selected bone's tip")
                            .clicked()
                        {
                            self.begin(doc, self.selected);
                        }
                    });
                }
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
        if let Some((bone, plant)) = foot_contact {
            self.error = clip.set_foot_contact(bone, frame, plant).err();
        }
        self.inspector_open = header.body_returned.is_some();
        if !self.visible || !self.pose_handles || !self.inspector_open {
            self.cancel_handle_drag();
        }
        if header.body_returned.is_none() && self.placing() {
            self.cancel_placement();
        }
    }

    fn show_draft(&mut self, ui: &mut Ui, doc: &mut Arc<Document>) {
        let draft = self.draft.as_mut().unwrap();
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(
                egui::TextEdit::singleline(&mut draft.name)
                    .desired_width(110.)
                    .char_limit(80),
            );
        });
        ui.label(match draft.pick {
            Pick::Head => "Click the bone’s start on the artboard.",
            Pick::Tip => "Click the bone’s tip on the artboard.",
        });
        egui::Grid::new("rest-coordinates").show(ui, |ui| {
            for (label, point, enabled) in [
                ("Rest start", &mut draft.head, draft.parent.is_none()),
                ("Rest tip", &mut draft.tail, true),
            ] {
                ui.label(label);
                for (component, n) in ["X", "Y"].into_iter().zip(point.iter_mut()) {
                    ui.add_enabled(
                        enabled,
                        egui::DragValue::new(n)
                            .prefix(format!("{component} "))
                            .speed(0.25),
                    );
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .button("Create bone")
                .on_hover_text("Use these rest coordinates")
                .clicked()
            {
                self.create(Arc::make_mut(doc));
            }
            if ui.button("Cancel").clicked() {
                self.cancel_placement();
            }
        });
    }

    fn show_pose(
        &mut self,
        ui: &mut Ui,
        doc: &mut Arc<Document>,
        selection: &[u64],
        frame: u32,
        mode: KeyMode,
    ) -> Option<(u64, bool)> {
        let current = self
            .selected
            .and_then(|id| doc.skeleton.bones.iter().find(|b| b.id == id))
            .or_else(|| doc.skeleton.bones.first())
            .unwrap();
        self.selected = Some(current.id);
        egui::ComboBox::from_label("Bone")
            .selected_text(&current.name)
            .show_ui(ui, |ui| {
                for b in &doc.skeleton.bones {
                    ui.selectable_value(&mut self.selected, Some(b.id), &b.name);
                }
            });
        let index = self
            .selected
            .and_then(|id| doc.skeleton.bones.iter().position(|b| b.id == id))?;
        let b = &doc.skeleton.bones[index];
        let id = b.id;
        if self.ik_target.is_some_and(|target| target != id) {
            self.cancel_placement();
        }
        if let Some(parent) = b
            .parent
            .and_then(|p| doc.skeleton.bones.iter().find(|b| b.id == p))
        {
            ui.small(format!("Child of {}", parent.name));
        }
        self.rotation_controls(ui, doc, index, frame, mode);
        let foot_contact = self.ik_controls(ui, doc, id, frame);
        let any_bound = doc
            .objects
            .iter()
            .any(|o| selection.contains(&o.id) && (o.rigid_bone.is_some() || o.skin.is_some()));
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!selection.is_empty(), egui::Button::new("Bind selection"))
                .on_hover_text("Attach the selected artwork rigidly to this bone")
                .clicked()
            {
                self.bind(doc, selection, Some(id), frame);
            }
            if ui
                .add_enabled(any_bound, egui::Button::new("Detach"))
                .on_hover_text("Keep this pose and release the selected artwork from its bones")
                .clicked()
            {
                self.bind(doc, selection, None, frame);
            }
        });
        self.skin_controls(ui, doc, selection, id, frame);
        if selection.len() == 1 {
            let binding = doc
                .objects
                .iter()
                .find(|o| o.id == selection[0])
                .and_then(|o| o.rigid_bone)
                .and_then(|id| doc.skeleton.bones.iter().find(|b| b.id == id));
            if let Some(b) = binding {
                ui.small(format!("Bound to {}", b.name));
            }
        }
        foot_contact.map(|plant| (id, plant))
    }

    fn rotation_controls(
        &mut self,
        ui: &mut Ui,
        doc: &mut Arc<Document>,
        index: usize,
        frame: u32,
        mode: KeyMode,
    ) {
        let b = &doc.skeleton.bones[index];
        let id = b.id;
        let mut angle = b.rotation_at(frame);
        let keyed = b.rotation_keys.contains_key(&frame);
        let tracked = !b.rotation_keys.is_empty();
        let interpolation = b.interpolation;
        let editable = mode != KeyMode::Manual || !tracked || keyed;
        ui.horizontal_wrapped(|ui| {
            ui.label("Pose angle");
            let angle_changed = ui
                .add_enabled(
                    editable,
                    egui::DragValue::new(&mut angle)
                        .suffix("°")
                        .speed(0.5)
                        .fixed_decimals(1),
                )
                .on_hover_text(mode.hint())
                .on_disabled_hover_text(
                    "Add a pose key on this frame to change its angle in Manual mode",
                )
                .changed();
            let reset = ui
                .add_enabled(editable && angle != 0., egui::Button::new("Reset"))
                .on_hover_text("Set the displayed pose angle to zero")
                .clicked();
            if angle_changed || reset {
                self.error = Self::set_pose_angle(
                    Arc::make_mut(doc),
                    id,
                    frame,
                    mode,
                    if reset { 0. } else { angle },
                )
                .err();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.small(format!("Frame {}", frame + 1));
            if keyed {
                if ui
                    .button("Remove pose key")
                    .on_hover_text("Remove this bone's rotation key at the playhead")
                    .clicked()
                {
                    self.error = Arc::make_mut(doc).remove_bone_rotation_key(id, frame).err();
                }
            } else if ui
                .button("Key pose")
                .on_hover_text("Store this bone's current angle at the playhead")
                .clicked()
            {
                self.error = Arc::make_mut(doc)
                    .set_bone_rotation_key(id, frame, angle)
                    .err();
            }
            if tracked {
                let mut chosen = interpolation;
                egui::ComboBox::from_id_salt(("bone-interpolation", id))
                    .width(52.)
                    .selected_text(match chosen {
                        Interpolation::Linear => "Linear",
                        Interpolation::Hold => "Hold",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut chosen, Interpolation::Linear, "Linear");
                        ui.selectable_value(&mut chosen, Interpolation::Hold, "Hold");
                    })
                    .response
                    .on_hover_text("How this bone rotates between keys");
                if chosen != interpolation {
                    self.error = Arc::make_mut(doc).set_bone_interpolation(id, chosen).err();
                }
            }
        });
    }

    fn set_pose_angle(
        doc: &mut Document,
        id: u64,
        frame: u32,
        mode: KeyMode,
        degrees: f32,
    ) -> Result<(), String> {
        let bone = doc
            .skeleton
            .bones
            .iter()
            .find(|b| b.id == id)
            .ok_or("Choose an existing bone.")?;
        if bone.rotation_keys.is_empty() {
            if mode != KeyMode::Auto {
                return doc.set_bone_rotation(id, degrees);
            }
            if frame > 0 {
                let base = bone.rotation;
                let mut staged = doc.clone();
                staged.set_bone_rotation_key(id, 0, base)?;
                staged.set_bone_rotation_key(id, frame, degrees)?;
                *doc = staged;
                return Ok(());
            }
        } else if mode == KeyMode::Manual && !bone.rotation_keys.contains_key(&frame) {
            return Err("Add a pose key on this frame to change its angle in Manual mode.".into());
        }
        doc.set_bone_rotation_key(id, frame, degrees)
    }

    /// Draw joints over posed artwork. The draft itself stores rest coordinates.
    pub fn paint(
        &self,
        painter: &egui::Painter,
        doc: &Document,
        frame: u32,
        screen: impl Fn([f32; 2]) -> Pos2,
        cursor: Option<[f32; 2]>,
    ) {
        if !self.inspector_open
            || !self.visible
            || (doc.skeleton.bones.is_empty() && self.draft.is_none())
        {
            return;
        }
        if let Ok(segments) = doc.bone_segments_at(true, frame) {
            for bone in &segments {
                let (a, b) = (bone.head, bone.tail);
                let selected = self.selected == Some(bone.id);
                let color = if selected {
                    Color32::from_rgb(255, 196, 100)
                } else {
                    Color32::from_rgb(97, 218, 199)
                };
                let (a, b) = (screen(a), screen(b));
                if !a.is_finite() || !b.is_finite() {
                    continue;
                }
                let width = if selected { 3. } else { 2. };
                painter.line_segment([a, b], (width + 2., Color32::from_black_alpha(185)));
                painter.line_segment([a, b], (width, color));
                painter.circle_filled(a, 4., color);
                painter.circle_stroke(a, 4., (1., Color32::BLACK));
                painter.circle_filled(b, 2.5, color);
            }
            if self.handles_visible() {
                if let Ok(handles) = doc.ik_handles_at(frame) {
                    for bone in segments.iter().filter(|bone| handles.contains(&bone.id)) {
                        self.paint_handle(painter, doc, bone.id, frame, screen(bone.tail), &screen);
                    }
                }
            }
        }
        if let (Some(id), Some(target)) = (self.ik_target, cursor) {
            self.paint_ik_preview(painter, doc, id, frame, target, &screen);
        }
        self.paint_weight_node(painter, doc, frame, &screen);
        if let Some(d) = &self.draft {
            let color = Color32::from_rgb(255, 210, 135);
            if let (Pick::Head, Some(point)) = (d.pick, cursor) {
                painter.circle_stroke(screen(point), 5., (1.5, color));
                return;
            }
            let posed = |p| match d.parent {
                Some(id) => doc.bone_posed_point_at(id, p, frame).ok(),
                None => Some(p),
            };
            let Some(head) = posed(d.head) else { return };
            let Some(tip) = cursor.or_else(|| posed(d.tail)) else {
                return;
            };
            let a = screen(head);
            let b = screen(tip);
            if !a.is_finite() || !b.is_finite() {
                return;
            }
            painter.line_segment([a, b], (1.5, color));
            painter.circle_stroke(a, 4., (1.5, color));
            painter.circle_stroke(b, 3., (1.5, color));
        }
    }
}

#[cfg(test)]
mod tests;

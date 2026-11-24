mod asset_move;
mod eyedropper;
mod flip;
mod folder_drag;
pub(crate) use eyedropper::{paint_picker_icon, PaintChannel};
#[cfg(test)]
pub(crate) use flip::flip_selection;
pub(crate) use flip::flip_selection_in;
pub(crate) mod display_filter;
mod gesture;
mod gradient;
mod lettering;
mod motion;
mod node_click;
mod onion;
mod overlay;
mod pasteboard;
mod paths;
pub(crate) use paths::nearest_morph_point;
mod pencil;
mod pivot;
mod point_text;
mod raster;
mod selection_move;
pub(crate) use raster::placement::{PlacementPage, PlacementResolution};
mod snapping;
mod tool_paint;
pub(crate) use tool_paint::{DrawingPaint, PathPaintMemory};
#[cfg(test)]
mod tests;

#[cfg(test)]
use crate::commands::editable_selection;
use crate::commands::is_editable;
use crate::diagnostics::{Action, Diagnostics};
use document_core::arrange::{Group, SelectionScope};
use document_core::{Document, PathNode, Shape};
use eframe::egui::{self, Color32, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use gesture::{box_corners, group_bounds, Gesture, GestureIds};
pub use snapping::SnapSettings;
use std::sync::{Arc, Weak};

const ARTBOARD_COLOR: Color32 = Color32::from_rgb(179, 181, 183);

#[cfg(test)]
pub(crate) fn painted_artboard(shape: &egui::Shape) -> Option<Rect> {
    match shape {
        egui::Shape::Rect(rect) if rect.fill == ARTBOARD_COLOR => Some(rect.rect),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rectangle,
    Ellipse,
    Pen,
    Pencil,
    Node,
    Text,
    Eyedropper,
    Gradient,
}

impl From<Tool> for crate::diagnostics::Tool {
    fn from(tool: Tool) -> Self {
        match tool {
            Tool::Select => Self::Select,
            Tool::Node => Self::Node,
            Tool::Rectangle => Self::Rectangle,
            Tool::Ellipse => Self::Ellipse,
            Tool::Pen => Self::Pen,
            Tool::Pencil => Self::Pencil,
            Tool::Text => Self::Text,
            Tool::Eyedropper => Self::Eyedropper,
            Tool::Gradient => Self::Gradient,
        }
    }
}

#[derive(Clone)]
pub struct CanvasStyle {
    pub(crate) group_edit: Option<GroupEdit>,
    pub(crate) edit_motion: bool,
    pub(crate) key_mode: crate::timeline::KeyMode,
    pub fill: [u8; 4],
    pub no_fill: bool,
    pub gradient: Option<std::sync::Arc<document_core::Gradient>>,
    pub stroke_gradient: Option<std::sync::Arc<document_core::Gradient>>,
    pub(crate) gradient_channel: PaintChannel,
    pub stroke: [u8; 4],
    pub no_stroke: bool,
    pub stroke_width: f32,
    pub cap: document_core::LineCap,
    pub join: document_core::LineJoin,
    pub layer: u64,
    pub snap: SnapSettings,
}

#[derive(Clone, Copy)]
pub(crate) struct GroupEdit {
    pub group: Group,
    pub clip_session: u64,
    pub exposure_start: u32,
}

impl CanvasStyle {
    pub(crate) fn group_scope(&self) -> SelectionScope {
        self.group_edit.map_or(SelectionScope::Groups, |edit| {
            SelectionScope::InGroup(edit.group)
        })
    }

    pub(crate) fn read_paint(&mut self, object: &document_core::VectorObject, frame: u32) {
        if object.image.is_some() {
            return;
        }
        self.fill = object.fill_at(frame);
        self.no_fill = object.no_fill;
        if self.gradient.as_deref() != object.gradient.as_ref() {
            self.gradient = object.gradient.clone().map(std::sync::Arc::new);
        }
        self.stroke = object.stroke_at(frame);
        self.no_stroke = object.no_stroke;
        if self.stroke_gradient.as_deref() != object.stroke_gradient.as_ref() {
            self.stroke_gradient = object.stroke_gradient.clone().map(std::sync::Arc::new);
        }
        self.stroke_width = object.stroke_width_at(frame);
        self.cap = object.cap;
        self.join = object.join;
        self.layer = object.layer;
    }

    fn paint_new_object(&self, doc: &mut Document) {
        let group = self.group_scope().group().and_then(|group| {
            let name = group.member(doc)?.group_name.clone();
            Some((group, name))
        });
        let object = doc.objects.last_mut().expect("creation appends an object");
        if let Some((group, name)) = group {
            group.add(object, name);
        }
        object.fill = self.fill;
        object.no_fill = self.no_fill;
        object.gradient = self.gradient.as_deref().cloned();
        object.stroke = self.stroke;
        object.no_stroke = self.no_stroke;
        object.stroke_gradient = self.stroke_gradient.as_deref().cloned();
        object.stroke_width = self.stroke_width;
        object.cap = self.cap;
        object.join = self.join;
    }
}

#[derive(Clone, Copy)]
struct View {
    board: Rect,
    scale: f32,
}
impl View {
    fn world(self, pos: Pos2) -> Pos2 {
        ((pos - self.board.min) / self.scale).to_pos2()
    }
    fn screen(self, pos: Pos2) -> Pos2 {
        self.board.min + pos.to_vec2() * self.scale
    }
}

struct FrameTexture {
    sample_pixels: Option<std::sync::Arc<egui::ColorImage>>,
    backdrop: Option<renderer::EditBackdrop>,
    document: std::sync::Arc<Document>,
    pending_split: Option<std::sync::Arc<Document>>,
    failed_refinement: Option<raster::FailedRefinement>,
    frame_independent: std::cell::OnceCell<bool>,
    frame: u32,
    scale: f32,
    handle: TextureHandle,
}

#[derive(Default)]
pub struct Canvas {
    pub(crate) direct_selection: Option<u64>,
    direct_click: bool,
    context_surface: Option<(egui::Response, View)>,
    sampled: Option<eyedropper::PaintSample>,
    pub(crate) property_pick: Option<PaintChannel>,
    diagnostics: Option<Diagnostics>,
    point_text: Option<point_text::PointText>,
    text_defaults: document_core::typography::TextStyle,
    pasteboard: pasteboard::Pasteboard,
    detail: pasteboard::Pasteboard,
    nodes: overlay::NodeOverlay,
    pub(super) rig: crate::rig_panel::RigPanel,
    onions: onion::OnionSkins,
    zoom_steps: f32,
    viewport_size: Option<Vec2>,
    sharpen_at: Option<f64>,
    pan: Vec2,
    gesture: Option<Gesture>,
    editing_pivot: Option<u64>,
    last_node_click: Option<(Pos2, f64, Option<u64>)>,
    pen: Vec<PathNode>,
    continuation: Option<paths::Continuation>,
    pen_redo: Vec<PathNode>,
    // Redo may reattach the first point after undoing back to an empty draft.
    pen_redo_join: Option<paths::Continuation>,
    pencil: Option<pencil::Stroke>,
    extra_pencil_strokes: Vec<pencil::FinishedStroke>,
    pencil_smoothing: Option<u8>,
    pencil_pressure: bool,
    pen_targets: Option<snapping::PenTargets>,
    drag_targets: Option<snapping::DragTargets>,
    previous: Option<(Tool, u32, u64)>,
    texture: Option<FrameTexture>,
    refinement: Option<raster::Refinement>,
    drag_preview: Option<renderer::DragPreview>,
    asset_move: asset_move::AssetMove,
    selection_move: selection_move::SelectionMove,
    duplicated_placement: Option<(std::sync::Arc<Document>, u32, u64)>,
    page_drag: Option<raster::PageDrag>,
    previous_drag_document: Option<std::sync::Arc<Document>>,
    staged_transform: Option<StagedTransform>,
    patching_drag: bool,
    show_checkerboard: bool,
    checker: Option<TextureHandle>,
}

impl Canvas {
    pub(crate) fn selection_scope(&self, style: &CanvasStyle, selection: &[u64]) -> SelectionScope {
        let scope = style.group_scope();
        if scope == SelectionScope::Groups
            && self.direct_selection.is_some_and(|id| selection == [id])
        {
            SelectionScope::Objects
        } else {
            scope
        }
    }

    /// Retain the document's view and tools while releasing textures and jobs
    /// that only its visible canvas can use.
    pub(crate) fn park(&mut self) {
        self.context_surface = None;
        self.pasteboard = Default::default();
        self.detail = Default::default();
        self.nodes.clear();
        self.onions = Default::default();
        self.texture = None;
        self.refinement = None;
        self.drag_preview = None;
        self.asset_move = Default::default();
        self.selection_move = Default::default();
        self.duplicated_placement = None;
        self.page_drag = None;
        self.previous_drag_document = None;
        self.staged_transform = None;
        self.patching_drag = false;
        self.drag_targets = None;
        self.pen_targets = None;
        self.sharpen_at = None;
        self.checker = None;
    }

    pub(crate) fn take_folder_asset_drop(
        &self,
    ) -> Option<(std::sync::Arc<crate::library::AssetDrop>, [f32; 2])> {
        let (response, view) = self.context_surface.as_ref()?;
        let asset = response.dnd_release_payload::<crate::library::AssetDrop>()?;
        let position = response.ctx.pointer_interact_pos()?;
        Some((asset, view.world(position).into()))
    }

    pub(crate) fn take_context_surface(&mut self) -> Option<(egui::Response, Option<[f32; 2]>)> {
        let (response, view) = self.context_surface.take()?;
        let at = response
            .interact_pointer_pos()
            .map(|p| view.world(p).into());
        Some((response, at))
    }

    pub(crate) fn show_checkerboard(&self) -> bool {
        self.show_checkerboard
    }

    pub(crate) fn set_checkerboard(&mut self, enabled: bool) {
        self.show_checkerboard = enabled;
    }

    pub(crate) fn set_diagnostics(&mut self, store: Option<Diagnostics>) {
        self.diagnostics = store;
    }

    fn note(&self, action: Action) {
        if let Some(store) = &self.diagnostics {
            store.note(action);
        }
    }
    pub fn set_animation_overlay(
        &mut self,
        clip: &document_core::Clip,
        frame: u32,
        sides: [(u32, [u8; 4]); 2],
        editing: bool,
    ) {
        let painted = self.texture.as_ref().map(|texture| texture.authored());
        self.onions.set(clip, frame, sides, editing, painted);
    }
    pub(crate) fn pen_in_progress(&self) -> bool {
        !self.pen.is_empty() || matches!(self.gesture, Some(Gesture::Pen(_)))
    }
    pub(crate) fn pencil_in_progress(&self) -> bool {
        self.pencil.is_some()
    }
    pub(crate) fn path_in_progress(&self) -> bool {
        self.pen_in_progress() || self.pencil_in_progress()
    }
    /// A live outline owns Undo/Redo until its last point is removed. At that
    /// point Undo can reach the document again, while Redo can restart the draft.
    pub(crate) fn pen_history_available(&self, redo: bool) -> Option<bool> {
        if matches!(self.gesture, Some(Gesture::Pen(_))) {
            return Some(false);
        }
        if !self.pen.is_empty() || (redo && !self.pen_redo.is_empty()) {
            Some(!redo || !self.pen_redo.is_empty())
        } else {
            None
        }
    }

    pub(crate) fn step_pen_history(&mut self, redo: bool) -> bool {
        let Some(enabled) = self.pen_history_available(redo) else {
            return false;
        };
        if enabled {
            if redo {
                if self.pen.is_empty() {
                    self.continuation = self.pen_redo_join.take();
                }
                self.pen
                    .push(self.pen_redo.pop().expect("available Pen redo"));
            } else {
                self.pen_redo
                    .push(self.pen.pop().expect("available Pen undo"));
                if self.pen.is_empty() {
                    self.pen_redo_join = self.continuation.take();
                }
            }
        }
        true
    }

    fn clear_pen_redo(&mut self) {
        self.pen_redo.clear();
        self.pen_redo_join = None;
    }
    pub(crate) fn set_pencil_options(&mut self, smoothing: u8, pressure: bool) {
        self.pencil_smoothing = Some(smoothing.min(200));
        self.pencil_pressure = pressure;
    }
    pub fn cancel_drag(&mut self) {
        self.asset_move.cancel();
        self.rig.cancel_handle_drag();
        self.property_pick = None;
        self.previous_drag_document = None;
        self.staged_transform = None;
        self.gesture = None;
        self.direct_click = false;
        self.last_node_click = None;
        self.drag_targets = None;
        self.pen.clear();
        self.clear_pen_redo();
        self.continuation = None;
        self.pencil = None;
    }
    pub(crate) fn moving_selection(&self) -> bool {
        matches!(self.gesture, Some(Gesture::Move { .. }))
    }

    pub(crate) fn has_unfinished_edit(&self) -> bool {
        self.gesture.is_some()
            || self.path_in_progress()
            || self.text_active()
            || self.rig.placing()
    }
    pub(crate) fn cancel_placement(&mut self, ctx: &egui::Context) -> bool {
        if (self.property_pick.is_some()
            || self.editing_pivot.is_some()
            || matches!(
                self.gesture,
                Some(Gesture::Pivot { .. } | Gesture::Gradient { .. })
            ))
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.property_pick.take().is_none() {
                self.editing_pivot = None;
                self.cancel_drag();
            }
            return true;
        }
        false
    }
    pub fn fit(&mut self) {
        self.zoom_steps = 0.;
        self.sharpen_at = None;
        self.pan = Vec2::ZERO;
        self.cancel_drag();
    }
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        authored: &mut std::sync::Arc<Document>,
        frame: u32,
        tool: Tool,
        selection: &mut Vec<u64>,
        style: &CanvasStyle,
        editable: bool,
    ) -> Result<bool, String> {
        // The command may come from a shortcut or a menu after the canvas has
        // painted. Properties can take a mutable copy without changing any
        // artwork, so accept that copy as well as the original snapshot.
        let duplicate = self
            .duplicated_placement
            .take()
            .filter(|(drawing, tick, id)| {
                *tick == frame
                    && selection.as_slice() == [*id]
                    && (std::sync::Arc::ptr_eq(drawing, authored)
                        || drawing.as_ref() == authored.as_ref())
            });
        self.asset_move.poll_warm(ui.ctx(), editable);
        self.context_surface = None;
        // Compare the actual outgoing and incoming snapshots in this paint only.
        // Dropping either a failed or abandoned cache cannot retain a drag document.
        let previous_art = self.texture.as_ref().map(|t| t.authored().clone());
        // A hit in another layer changes the inspector's active layer. Only
        // creation gestures depend on that layer as their destination.
        let destination = match tool {
            Tool::Select | Tool::Node | Tool::Eyedropper | Tool::Gradient => 0,
            _ => style.layer,
        };
        if tool != Tool::Pen || !style.snap.enabled {
            self.pen_targets = None;
        }
        let state = (tool, frame, destination);
        if tool != Tool::Select
            || !editable
            || self
                .editing_pivot
                .is_some_and(|id| selection.as_slice() != [id])
        {
            self.editing_pivot = None;
        }
        if self.previous != Some(state) || !editable || !ui.is_enabled() {
            self.cancel_drag();
        }
        if self
            .continuation
            .or(self.pen_redo_join)
            .is_some_and(|path| {
                path.target_index(authored, selection, frame, style.key_mode)
                    .is_none()
            })
        {
            self.cancel_drag();
        }
        self.previous = Some(state);
        let (viewport, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let size = egui::vec2(authored.width as f32, authored.height as f32);
        // Resizing a dock or the native window can change Fit on every pointer
        // event. Scale the existing artwork during that gesture, as for wheel zoom.
        let mut view_changed = self
            .viewport_size
            .replace(viewport.size())
            .is_some_and(|previous| previous != viewport.size());
        let fit = ((viewport.width() - 64.) / size.x)
            .min((viewport.height() - 64.) / size.y)
            .max(0.01);
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let over_text = self.point_text.is_some() && ui.rect_contains_pointer(viewport);
        if (response.hovered() || over_text)
            && self.gesture.is_none()
            && !self.pencil_in_progress()
            && !self.rig.dragging_handle()
        {
            if let Some(pos) = pointer {
                let before = self.zoom_steps;
                // egui reports Ctrl+wheel and native pinch as a scale factor.
                // Plain wheel events are delivered separately as scroll distances.
                let delta = ui.input(|i| {
                    let factor = i.zoom_delta();
                    let pinch = if factor.is_finite() && factor > 0. {
                        factor.log2()
                    } else {
                        0.
                    };
                    i.smooth_scroll_delta.y / 400. + pinch
                });
                // Large artboards need more fit-relative steps to inspect the
                // same fine geometry as small drawings.
                let max_zoom = (64. / fit).log2().max(4.);
                self.zoom_steps = (before + delta).clamp(-3., max_zoom);
                if self.zoom_steps != before {
                    self.note(Action::Zoom);
                }
                let ratio = (self.zoom_steps - before).exp2();
                self.pan += (pos - viewport.center() - self.pan) * (1. - ratio);
                view_changed |= self.zoom_steps != before;
            }
        }
        if view_changed {
            self.last_node_click = None;
            self.sharpen_at = Some(ui.input(|i| i.time) + 0.08);
        }
        if response.drag_started_by(egui::PointerButton::Middle) {
            self.note(Action::Pan);
        }
        if response.dragged_by(egui::PointerButton::Middle) && !self.pencil_in_progress() {
            self.pan += ui.input(|i| i.pointer.delta());
        }
        let scale = fit * self.zoom_steps.exp2();
        let view = View {
            board: Rect::from_center_size(viewport.center() + self.pan, size * scale),
            scale,
        };
        let visible_world = Rect::from_two_pos(view.world(viewport.min), view.world(viewport.max));
        self.asset_move.set_view(visible_world);
        if let Some((drawing, _, id)) = duplicate {
            if let Some(object) = drawing.objects.iter().find(|o| o.id == id) {
                if self.asset_move.prepare(
                    ui.ctx(),
                    &drawing,
                    object,
                    frame,
                    scale * ui.ctx().pixels_per_point(),
                ) {
                    self.asset_move.release(&drawing, frame, id);
                }
            }
        }
        let board_point = pointer
            .filter(|p| view.board.contains(*p))
            .map(|p| view.world(p).into());
        let mut change = Edit::default();
        if editable && ui.is_enabled() {
            if self.rig.placing() {
                if response.hovered() && board_point.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    if ui.input(|i| i.pointer.primary_pressed()) {
                        self.note(Action::Rig);
                    }
                }
                self.cancel_drag();
                self.rig
                    .placement_input(ui, &response, board_point, authored, frame);
            } else if self.gesture.is_none()
                && self.property_pick.is_none()
                && self.point_text.is_none()
                && !self.path_in_progress()
                && self.rig.handle_input(
                    ui,
                    &response,
                    authored,
                    frame,
                    |p| view.screen(p.into()),
                    |p| view.world(p),
                )
            {
                if ui.input(|i| i.pointer.primary_pressed()) {
                    self.note(Action::Rig);
                }
                change.preview = self.rig.handle_preview();
            } else if tool == Tool::Eyedropper || self.property_pick.is_some() {
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
                if response.clicked_by(egui::PointerButton::Primary) {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let world = view.world(pos);
                        let sample = self.sample_paint(
                            authored,
                            frame,
                            world.into(),
                            view.scale * ui.ctx().pixels_per_point(),
                            self.property_pick.map_or_else(
                                || ui.input(|i| i.modifiers.shift),
                                |p| p == PaintChannel::Stroke,
                            ),
                        )?;
                        let painted = !selection.is_empty()
                            && sample.apply_to_selection(
                                Arc::make_mut(authored),
                                selection,
                                frame,
                                style,
                            );
                        change.committed = self.property_pick.take().is_none() && painted;
                        self.sampled = Some(sample);
                        self.note(Action::SamplePaint);
                    }
                }
            } else if tool == Tool::Pencil {
                change.committed =
                    self.pencil_input(ui, &response, authored, selection, style, view);
            } else if tool == Tool::Text {
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }
                if response.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
                    if let Some(pos) = pointer.filter(|p| response.interact_rect.contains(*p)) {
                        if self.point_text.is_none() {
                            self.note(Action::EditText);
                            self.start_text(authored, frame, view.world(pos), selection, style);
                        }
                    }
                }
            } else {
                change =
                    self.interact(ui, &response, authored, frame, tool, selection, style, view);
            }
        }
        if editable && tool == Tool::Text {
            if let Some(session) = &mut self.point_text {
                match session.show(ui, view, viewport, authored, frame) {
                    point_text::Outcome::Editing => {
                        if let Some(caret) = session.take_caret_reveal() {
                            let inner = viewport.shrink(8.);
                            let shift = (inner.min - caret.min).max(Vec2::ZERO)
                                + (inner.max - caret.max).min(Vec2::ZERO);
                            if inner.is_positive() && shift != Vec2::ZERO {
                                self.pan += shift;
                                ui.ctx().request_repaint();
                            }
                        }
                    }
                    point_text::Outcome::Cancel => {
                        session.surrender_focus(ui.ctx());
                        self.point_text = None;
                    }
                    point_text::Outcome::Commit => {
                        // A layout error stays with the draft so the artist can correct it.
                        change.committed |= self
                            .finish_text(ui.ctx(), Arc::make_mut(authored), selection)
                            .unwrap_or(false);
                    }
                }
            }
        }
        let doc = authored.as_ref();
        let lettering = self.point_text.as_ref().and_then(|s| s.preview().cloned());
        let drawing = lettering.as_ref().unwrap_or(authored);
        let scene = change.preview.as_deref().unwrap_or(drawing.as_ref());
        let painter = ui.painter_at(viewport);
        painter.rect_filled(viewport, 0., Color32::from_rgb(27, 30, 35));
        let margins = painter.add(egui::Shape::Noop);
        painter.rect(
            view.board,
            0.,
            ARTBOARD_COLOR,
            (1., Color32::from_rgb(97, 101, 106)),
            egui::StrokeKind::Outside,
        );
        if self.show_checkerboard {
            let checker = self.checker.get_or_insert_with(|| {
                let gray = Color32::from_rgb(213, 216, 219);
                let white = Color32::from_rgb(229, 231, 233);
                ui.ctx().load_texture(
                    "artboard transparency",
                    egui::ColorImage {
                        size: [2, 2],
                        pixels: vec![gray, white, white, gray],
                    },
                    TextureOptions {
                        wrap_mode: egui::TextureWrapMode::Repeat,
                        ..TextureOptions::NEAREST
                    },
                )
            });
            painter.image(
                checker.id(),
                view.board,
                Rect::from_min_max(Pos2::ZERO, (view.board.size() / 24.).to_pos2()),
                Color32::WHITE,
            );
        }
        let physical = view.scale * ui.ctx().pixels_per_point();
        if change.selection_commit {
            self.selection_move.commit(authored, view);
        }
        if editable && tool == Tool::Select && selection.len() >= 2 {
            // A single marquee can visit hundreds of different selections.
            // Prepare its final paint order once, after the button is released.
            if !matches!(self.gesture, Some(Gesture::Marquee { .. })) {
                let holding = self.gesture.as_ref().is_some_and(Gesture::transforms_group);
                self.selection_move.prepare(
                    ui.ctx(),
                    authored,
                    selection,
                    frame,
                    (physical, visible_world),
                    !holding && !change.selection_commit,
                );
            }
        } else {
            self.selection_move.reset();
        }
        let mut group_images = change
            .preview
            .as_ref()
            .filter(|_| self.gesture.as_ref().is_some_and(Gesture::transforms_group))
            .and_then(|posed| self.selection_move.drag_images(ui.ctx(), posed, view));
        if editable && !view_changed && self.texture.as_ref().is_none_or(|page| page.frame != frame)
        {
            // Start preparing the newly visible pose while this thread paints the page.
            // A grab immediately after scrubbing can then use the same previews as an idle grab.
            self.asset_move.warm_scene(
                ui.ctx(),
                authored,
                frame,
                physical,
                selection.first().copied(),
                visible_world,
            );
        }
        let page_limit = 2048. / size.x.max(size.y);
        let tiled = physical > page_limit;
        // The page texture supplies immediate, current pixels during pans and
        // edits. Once the zoom exceeds a 2048-pixel page, sharpen visible tiles.
        let mut raster_scale = if tiled {
            self.texture
                .as_ref()
                .filter(|t| (t.document.width, t.document.height) == (scene.width, scene.height))
                .map_or(fit * ui.ctx().pixels_per_point(), |t| t.scale)
                .min(page_limit)
        } else {
            physical
        };
        // During a wheel gesture the painter scales the existing artwork and onion
        // textures. Refine once the view settles, even if the pointer has stopped.
        let remaining = self
            .sharpen_at
            .map_or(0., |time| time - ui.input(|i| i.time));
        if remaining > 0. {
            if tiled {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
            }
            if let Some(cached) = &self.texture {
                if (cached.document.width, cached.document.height) == (scene.width, scene.height)
                    && cached.scale != raster_scale
                {
                    raster_scale = cached.scale;
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
                }
            }
        }
        // Keep live geometry under the pointer at one pixel per logical point.
        // Releasing or cancelling the gesture restores the display's full density.
        let previewing = change.preview.is_some() || change.object_preview.is_some();
        let artwork_scale = if previewing {
            self.sharpen_at = None;
            raster_scale.min(view.scale)
        } else {
            raster_scale
        };
        self.onions
            .paint_ghosts(ui.ctx(), &painter, view, raster_scale)?;
        // The incoming preview can replace the texture's document. Save the
        // outgoing allocation for the next move of the same transform target.
        if change.preview.is_some()
            && doc.objects.len() >= 512
            && self
                .gesture
                .as_ref()
                .and_then(Gesture::single_transform)
                .is_some()
        {
            self.previous_drag_document = self.texture.as_ref().map(|t| t.authored().clone());
        } else if self.previous_drag_document.is_some() {
            if ui.input(|i| i.pointer.primary_released()) {
                // Present the final pose before dropping a large retired allocation.
                ui.ctx().request_repaint();
            } else {
                self.previous_drag_document = None;
            }
        }
        if let Some(id) = change.object_commit {
            self.commit_object_page(ui.ctx(), authored, id, raster_scale)?;
        }
        if let Some(id) = change.asset_commit {
            self.asset_move.release(authored, frame, id);
        }
        let progress = if !previewing && lettering.is_none() {
            self.selection_move.advance(ui.ctx(), raster_scale)
        } else {
            selection_move::Progress::Inactive
        };
        let refining_tiles = matches!(
            progress,
            selection_move::Progress::Page(_) | selection_move::Progress::Tiles
        );
        if let selection_move::Progress::Page(image) = progress {
            self.upload_page(ui.ctx(), authored.clone(), frame, raster_scale, image);
        } else if matches!(progress, selection_move::Progress::Rendering) {
            group_images = Some(self.selection_move.committed_images(view));
        }
        let moving_layer = if change.asset_transform {
            true
        } else if previewing || lettering.is_some() {
            self.asset_move.cancel();
            false
        } else {
            match self.asset_move.settle(ui.ctx(), doc, frame, raster_scale) {
                asset_move::Settle::KeepLayers => true,
                asset_move::Settle::Finished(image) => {
                    self.upload_page(ui.ctx(), authored.clone(), frame, raster_scale, image);
                    false
                }
                asset_move::Settle::PaintNormally => false,
            }
        };
        // A scene made entirely of cached placements is drawn by the layer stack.
        // Keep the authored page and its tiles for cancellation instead of uploading
        // a full-size transparent replacement on the first pointer movement.
        if group_images.is_none()
            && (!moving_layer || !self.asset_move.backdrop().objects.is_empty())
        {
            let texture = if moving_layer {
                if let Some(cached) = self
                    .asset_move
                    .take_placement_background(frame, raster_scale)
                {
                    self.texture = Some(cached);
                }
                let backdrop = self.asset_move.backdrop();
                self.artwork_texture(ui.ctx(), &backdrop, frame, raster_scale, None)?
            } else if let Some(object) = &change.object_preview {
                self.object_page_texture(ui.ctx(), object, frame, artwork_scale)?
            } else {
                self.artwork_texture(
                    ui.ctx(),
                    drawing,
                    frame,
                    artwork_scale,
                    change.preview.as_ref(),
                )?
            };
            let texture_frame = self.texture.as_ref().unwrap();
            if editable && !moving_layer && !previewing && !view_changed && remaining <= 0. {
                self.asset_move.warm_scene(
                    ui.ctx(),
                    texture_frame.authored(),
                    frame,
                    physical,
                    selection.first().copied(),
                    visible_world,
                );
            }
            let tile_frame = if texture_frame.frame != frame
                && texture_frame.frame_independent.get() == Some(&true)
            {
                texture_frame.frame
            } else {
                frame
            };
            let margins_ready = self.pasteboard.paint(
                &painter,
                margins,
                view,
                tile_frame,
                texture_frame,
                previous_art.as_ref(),
                if remaining > 0. {
                    self.pasteboard.density().unwrap_or(raster_scale)
                } else {
                    physical
                },
                change.preview.is_some(),
                false,
                refining_tiles,
            );
            let art_slot = painter.add(display_filter::image(
                &painter,
                texture,
                view.board,
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
                Color32::WHITE,
            ));
            let detail_ready = if tiled && remaining > 0. {
                self.detail
                    .during_zoom(&painter, art_slot, view, tile_frame, texture_frame);
                true
            } else if tiled {
                self.detail.paint(
                    &painter,
                    art_slot,
                    view,
                    tile_frame,
                    texture_frame,
                    previous_art.as_ref(),
                    physical,
                    change.preview.is_some(),
                    true,
                    refining_tiles,
                )
            } else {
                self.detail.clear(ui.ctx());
                true
            };
            if refining_tiles {
                if margins_ready && detail_ready {
                    self.selection_move.finish_settlement(ui.ctx());
                } else {
                    painter.set(margins, egui::Shape::Noop);
                    painter.set(art_slot, egui::Shape::Noop);
                    group_images = Some(self.selection_move.committed_images(view));
                }
            }
        }
        if let Some(images) = group_images {
            painter.add(display_filter::batch(&painter, images));
        } else if moving_layer {
            self.asset_move
                .paint(&painter, view, change.object_preview.as_deref());
        }
        if let Some(group) = style.group_scope().group() {
            overlay::editing_group(
                &painter,
                view,
                scene,
                frame,
                group,
                change.object_preview.as_deref(),
            );
        }
        if !editable || tool != Tool::Node {
            self.nodes.clear();
        }
        if editable {
            self.onions.paint_origin(&painter, view);
            overlay::guides(&painter, view, doc, style.snap);
            if let Some(hit) = &change.snap {
                snapping::feedback(&painter, view, hit);
            }
            let single = match selection.as_slice() {
                [id] => change
                    .object_preview
                    .as_deref()
                    .filter(|o| o.id == *id)
                    .or_else(|| scene.objects.iter().find(|object| object.id == *id)),
                _ => None,
            };
            let pen_ends = if tool == Tool::Pen
                && ui.is_enabled()
                && !self.rig.placing()
                && self.pen.is_empty()
                && self.gesture.is_none()
            {
                single.and_then(|object| {
                    paths::Endpoints::for_object(scene, object, frame, style.key_mode)
                })
            } else {
                None
            };
            let resume_hover = pen_ends.and_then(|ends| {
                let pointer = pointer.filter(|_| response.hovered())?;
                ends.hit(view, pointer)
            });
            if let Some(object) = single {
                motion::paint(&painter, view, object, frame, style.edit_motion);
            }
            if tool == Tool::Gradient {
                overlay::selection(&painter, view, scene, frame, selection, tool);
                if let Some(object) = single.filter(|o| gradient::editable(scene, o, frame, style))
                {
                    let hovered = gradient::paint(
                        &painter,
                        view,
                        object,
                        frame,
                        pointer.filter(|_| response.hovered()),
                        style.gradient_channel,
                    );
                    if matches!(self.gesture, Some(Gesture::Gradient { .. })) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    } else if hovered {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                }
            } else if tool == Tool::Select && selection.len() > 1 {
                let rotation = match (&self.gesture, pointer) {
                    (Some(Gesture::GroupRotate { start, bounds, .. }), Some(pos)) => {
                        let angle = gesture::turn(
                            *start,
                            view.world(pos),
                            bounds.center(),
                            ui.input(|i| i.modifiers.shift),
                        );
                        Some((*bounds, angle))
                    }
                    _ => None,
                };
                overlay::group_selection(&painter, view, scene, frame, selection, rotation);
            } else if tool == Tool::Node {
                self.nodes.paint(
                    &painter,
                    view,
                    scene,
                    frame,
                    selection,
                    self.texture.as_ref().map(FrameTexture::authored),
                    match &self.gesture {
                        Some(Gesture::NodeMarquee {
                            id,
                            preview: Some((_, nodes)),
                            ..
                        }) => Some((*id, nodes)),
                        _ => None,
                    },
                );
            } else if let Some(ends) = pen_ends {
                overlay::pen_ends(&painter, view, ends, resume_hover);
            } else if let Some(object) = &change.object_preview {
                overlay::frame_outline(&painter, view, object, frame, true);
            } else if self.point_text.is_none()
                && !self.pencil_in_progress()
                && self.continuation.is_none()
            {
                overlay::selection(&painter, view, scene, frame, selection, tool);
            }
            if let Some(object) = single.filter(|object| {
                tool == Tool::Select
                    && !self.rig.placing()
                    && self.property_pick.is_none()
                    && ui.is_enabled()
                    && object.visible_at(frame)
                    && document_core::arrange::is_object_editable(scene, object)
            }) {
                let point = change
                    .pivot
                    .unwrap_or_else(|| object.pivot_at(frame).into());
                let dragging = matches!(self.gesture, Some(Gesture::Pivot { .. }));
                let hovered = self.gesture.is_none()
                    && self.editing_pivot.is_none()
                    && response.hovered()
                    && !ui.input(|i| i.modifiers.shift)
                    && pointer.is_some_and(|pos| {
                        // Most pointer movement only needs the screen-space distance check.
                        // Resolve overlapping transform and motion handles just at the marker.
                        pivot::over_marker(view.screen(point) - pos)
                            && matches!(
                                gesture::object_handle(
                                    object,
                                    frame,
                                    view.world(pos),
                                    view.scale,
                                    false
                                ),
                                Some(Gesture::Pivot { .. })
                            )
                            && !(style.edit_motion
                                && motion::hit(object, frame, view.world(pos), 7. / view.scale)
                                    .is_some())
                    });
                if self.editing_pivot.is_none() {
                    if dragging {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    } else if hovered {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                }
                pivot::marker(&painter, view.screen(point), dragging || hovered);
            }
            if let Some(
                Gesture::Marquee { start, .. }
                | Gesture::NodeMarquee {
                    start,
                    preview: Some(_),
                    ..
                },
            ) = &self.gesture
            {
                if let Some(p) = pointer {
                    overlay::marquee(&painter, view, *start, view.world(p));
                }
            }
            if let Some(pencil) = &self.pencil {
                pencil.paint(&painter, view, style);
            }
            if !self.rig.placing()
                && tool == Tool::Pen
                && (is_editable(doc, style.layer) || self.continuation.is_some())
            {
                let pointer = pointer.filter(|_| {
                    resume_hover.is_none() && (response.hovered() || self.gesture.is_some())
                });
                let preview = pointer.map(|position| {
                    if let Some(first) = self
                        .close_pen_target(view, position)
                        .filter(|_| self.gesture.is_none())
                    {
                        return overlay::PenPreview::Close { first, guide: None };
                    }
                    let snapshot = self
                        .texture
                        .as_ref()
                        .expect("artwork was painted")
                        .authored();
                    let visible = painter.clip_rect();
                    let point = view.world(position);
                    let angle_target = self
                        .gesture
                        .is_none()
                        .then(|| self.pen_angle_target(point, ui.input(|i| i.modifiers.ctrl)))
                        .flatten();
                    let hit = if let Some(anchor) = angle_target {
                        // An angle-constrained anchor takes precedence over magnetic snapping.
                        snapping::Hit::free(anchor)
                    } else {
                        let target = match self.gesture {
                            Some(Gesture::Pen(drag)) => drag.tangent_target(point),
                            _ => point,
                        };
                        snapping::PenTargets::after_paint(
                            &mut self.pen_targets,
                            target,
                            snapshot,
                            frame,
                            style.snap,
                            scale,
                            Rect::from_min_max(view.world(visible.min), view.world(visible.max)),
                        )
                    };
                    snapping::feedback(&painter, view, &hit);
                    match &mut self.gesture {
                        Some(Gesture::Pen(drag)) => {
                            let shift = ui.input(|i| {
                                i.modifiers.shift
                                    || (i.pointer.primary_pressed()
                                        && primary_button(i, true).is_some_and(|(_, m)| m.shift))
                            });
                            let node = drag.update(point, hit.point, scale, shift);
                            if drag.is_closing() {
                                overlay::PenPreview::Close {
                                    first: node,
                                    guide: drag.closing_guide(),
                                }
                            } else {
                                overlay::PenPreview::Extend(node)
                            }
                        }
                        _ => overlay::PenPreview::Extend(PathNode::corner(hit.point.into())),
                    }
                });
                overlay::pen(
                    &painter,
                    view,
                    &self.pen,
                    preview,
                    self.continuation
                        .map(|path| Pos2::from(path.oriented_ends()[0].point)),
                );
            }
            self.rig.paint(
                &painter,
                scene,
                frame,
                |p| view.screen(p.into()),
                board_point,
            );
        }
        if editable && tool == Tool::Text {
            if let Some(session) = &self.point_text {
                session.paint_feedback(&painter);
            }
        }
        self.context_surface = Some((response, view));
        Ok(change.committed)
    }

    #[allow(clippy::too_many_arguments)]
    fn interact(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        doc: &mut Arc<Document>,
        frame: u32,
        tool: Tool,
        selection: &mut Vec<u64>,
        style: &CanvasStyle,
        view: View,
    ) -> Edit {
        let mut edit = Edit::default();
        let staged_transform = self.staged_transform.take();
        let keys = ui.input(|i| {
            (
                i.focused,
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Enter),
                i.modifiers,
            )
        });
        let (focused, escape, enter, mods) = keys;
        let scope = style.group_scope();
        if !focused || escape {
            self.cancel_drag();
            if escape {
                self.editing_pivot = None;
            }
            return edit;
        }
        if tool == Tool::Pen
            && (response.secondary_clicked() || enter && !ui.ctx().wants_keyboard_input())
            && !ui.input(|i| i.pointer.primary_pressed())
            && !matches!(self.gesture, Some(Gesture::Pen(drag)) if self.continuation.is_some() || drag.is_closing())
            && (self.continuation.is_some() || is_editable(doc, style.layer))
        {
            if let Some(id) =
                self.finish_pen_draft(Arc::make_mut(doc), frame, selection, style, None)
            {
                *selection = vec![id];
                edit.committed = true;
            } else if response.secondary_clicked() {
                self.pen.clear();
                self.clear_pen_redo();
            }
            self.gesture = None;
            return edit;
        }
        let (press, release, down, origin, pointer) = ui.input(|i| {
            let p = &i.pointer;
            (
                p.primary_pressed(),
                p.primary_released(),
                p.primary_down(),
                p.press_origin(),
                p.interact_pos(),
            )
        });
        if self.editing_pivot.is_some() && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let primary_press = if press && matches!(tool, Tool::Select | Tool::Pen | Tool::Node) {
            ui.input(|i| primary_button(i, true))
        } else {
            None
        };
        let pen_press_on_canvas = tool == Tool::Pen
            && release
            && primary_press.is_some_and(|(pos, _)| {
                response.interact_rect.contains(pos)
                    && ui.ctx().layer_id_at(pos) == Some(response.layer_id)
            });
        if press && (response.contains_pointer() || pen_press_on_canvas) {
            let press_mods = primary_press.map_or(mods, |(_, modifiers)| modifiers);
            self.direct_click = tool == Tool::Select
                && (press_mods.ctrl || press_mods.command)
                && !press_mods.alt
                && !press_mods.shift;
            self.asset_move.begin_gesture();
            self.note(match tool {
                Tool::Select if self.editing_pivot.is_some() => Action::Transform,
                Tool::Select => Action::Select,
                Tool::Node => Action::EditPath,
                Tool::Rectangle | Tool::Ellipse | Tool::Pen | Tool::Pencil => Action::Draw,
                Tool::Text => Action::EditText,
                Tool::Eyedropper => Action::SamplePaint,
                Tool::Gradient => Action::Paint,
            });
            self.drag_targets = None;
            if let Some(start) = primary_press
                .map(|(position, _)| position)
                .or(origin)
                .or(pointer)
                .filter(|p| response.interact_rect.contains(*p))
            {
                let world = view.world(start);
                let guide_hit = if self.editing_pivot.is_none()
                    && style.edit_motion
                    && selection.len() == 1
                    && matches!(tool, Tool::Select | Tool::Node)
                {
                    doc.objects
                        .iter()
                        .find(|o| {
                            o.id == selection[0]
                                && document_core::arrange::is_object_editable(doc, o)
                        })
                        .and_then(|o| motion::hit(o, frame, world, 7. / view.scale))
                } else {
                    None
                };
                self.gesture = if let Some(id) = self.editing_pivot {
                    pivot::target(doc, selection, frame)
                        .is_some()
                        .then_some(Gesture::Pivot {
                            start: world,
                            id,
                            grab_offset: None,
                        })
                } else if let Some(mut hit) = guide_hit {
                    if mods.shift {
                        if let Gesture::Guide { part, .. } = &mut hit {
                            if *part == paths::Part::Anchor {
                                *part = paths::Part::SymmetricHandles;
                            }
                        }
                    }
                    Some(hit)
                } else {
                    match tool {
                        Tool::Gradient => {
                            gradient::begin(doc, frame, world, selection, view.scale, style)
                        }
                        Tool::Text | Tool::Eyedropper | Tool::Pencil => None,
                        Tool::Select => self.select(
                            doc,
                            frame,
                            world,
                            selection,
                            press_mods.shift,
                            view.scale,
                            scope,
                        ),
                        Tool::Node => {
                            let (gesture, changed) = self.start_node(
                                ui.ctx(),
                                doc,
                                frame,
                                world,
                                selection,
                                primary_press.map_or(mods, |(_, modifiers)| modifiers),
                                view.scale,
                                style.key_mode,
                                scope,
                            );
                            edit.committed = changed;
                            gesture
                        }
                        Tool::Pen => self
                            .start_pen(
                                doc,
                                selection,
                                frame,
                                style,
                                view,
                                start,
                                primary_press.map_or(mods, |(_, m)| m).ctrl,
                            )
                            .map(Gesture::Pen),
                        _ if !is_editable(doc, style.layer) => None,
                        Tool::Rectangle | Tool::Ellipse => Some(Gesture::Draw {
                            tool,
                            start: snapping::point(world, doc, frame, style.snap, view.scale).point,
                        }),
                    }
                };
            }
        }
        if let Some(Gesture::Move { flips, .. }) = &mut self.gesture {
            use crate::tool_keys::{selection_shortcuts, FLIPS};
            for horizontal in selection_shortcuts(ui.ctx(), &FLIPS, false) {
                let axis = usize::from(!horizontal);
                flips[axis] = !flips[axis];
            }
        }
        let mods = if release {
            ui.input(|i| primary_button(i, false))
                .map_or(mods, |(_, m)| m)
        } else {
            mods
        };
        let pointer = if release {
            ui.input(|i| primary_button(i, false))
                .map(|(pos, _)| pos)
                .or(pointer)
        } else {
            pointer
        };
        if release
            && std::mem::take(&mut self.direct_click)
            && response.clicked_by(egui::PointerButton::Primary)
        {
            if let Some(id) =
                pointer.and_then(|p| doc.hit_test_in(view.world(p).into(), frame, scope))
            {
                *selection = vec![id];
                self.direct_selection = Some(id);
                self.gesture = None;
                return edit;
            }
        }
        match &mut self.gesture {
            Some(Gesture::Node { start, edit, .. }) => edit.track_start(ui, view, *start, press),
            Some(Gesture::Pen(drag)) => drag.track_start(ui, view, press),
            _ => {}
        }
        if let (Some(Gesture::NodeMarquee { .. }), Some(pointer)) = (&self.gesture, pointer) {
            self.update_node_marquee(
                ui.ctx(),
                doc,
                frame,
                view,
                pointer,
                release,
                selection,
                scope,
            );
        }
        if tool == Tool::Node && ui.input(|i| i.pointer.any_released()) {
            edit.committed |= self.node_click(
                response,
                doc,
                frame,
                view,
                selection,
                mods,
                style.key_mode,
                scope,
            );
        }
        let (Some(gesture), Some(pointer)) = (&self.gesture, pointer) else {
            if !down {
                self.gesture = None;
                self.drag_targets = None;
            }
            return edit;
        };
        let point = view.world(pointer);
        match gesture {
            Gesture::Pivot {
                id,
                start,
                grab_offset,
            } if grab_offset.is_none() || point.distance(*start) * view.scale >= 2. => {
                let hit = snapping::DragTargets::resolve(
                    &mut self.drag_targets,
                    self.texture.as_ref().map(FrameTexture::authored),
                    point + grab_offset.unwrap_or_default(),
                    doc,
                    frame,
                    style.snap,
                    view.scale,
                    snapping::Subject::Point,
                    Rect::from_min_max(
                        view.world(response.interact_rect.min),
                        view.world(response.interact_rect.max),
                    ),
                );
                edit.pivot = Some(hit.point);
                if release {
                    if let Some(index) = pivot::target(doc, selection, frame) {
                        if doc.objects[index].id == *id {
                            let object = &mut Arc::make_mut(doc).objects[index];
                            edit.committed = object.set_pivot_at(frame, hit.point.into());
                        }
                    }
                }
                edit.snap = Some(hit);
            }
            Gesture::Pivot { .. } => {}
            Gesture::Pen(mut drag)
                if release && (self.continuation.is_some() || is_editable(doc, style.layer)) =>
            {
                // The pointer may already have moved beyond the release position
                // by the time the backend delivers this batch of events.
                let (end, end_mods) = ui
                    .input(|i| primary_button(i, false))
                    .unwrap_or((pointer, mods));
                let point = view.world(end);
                let hit = snapping::PenTargets::before_paint(
                    self.pen_targets.as_ref(),
                    drag.tangent_target(point),
                    doc,
                    frame,
                    style.snap,
                    view.scale,
                );
                let node = drag.update(point, hit.point, view.scale, end_mods.shift);
                if drag.is_closing() {
                    if let Some(id) = self.finish_pen_draft(
                        Arc::make_mut(doc),
                        frame,
                        selection,
                        style,
                        Some(node),
                    ) {
                        *selection = vec![id];
                        edit.committed = true;
                    }
                } else {
                    self.clear_pen_redo();
                    self.pen.push(node);
                }
            }
            Gesture::Pen(_) | Gesture::NodeMarquee { .. } => {}
            Gesture::Marquee { start, previous } => {
                if point.distance(*start) * view.scale >= 2. {
                    let rect = Rect::from_two_pos(*start, point);
                    let region = [rect.left(), rect.top(), rect.right(), rect.bottom()];
                    let mut ids = previous.clone();
                    for o in &doc.objects {
                        if scope.contains(o)
                            && document_core::arrange::is_object_editable(doc, o)
                            && o.bounds_within(region, frame)
                        {
                            ids.push(o.id);
                        }
                    }
                    *selection = scope.resolve(doc, &ids);
                }
            }
            _ if gesture.has_dragged(point, view.scale)
                || matches!(gesture, Gesture::Move { flips, .. } if flips.iter().any(|&flip| flip))
                || gesture
                    .placement_transform()
                    .is_some_and(|id| self.asset_move.can_release(doc, frame, id)) =>
            {
                if self.diagnostics.is_some() {
                    match gesture {
                        Gesture::Move { .. } => self.note(Action::Move),
                        Gesture::Resize { .. } | Gesture::GroupResize { .. } => {
                            self.note(Action::Resize)
                        }
                        Gesture::Rotate { .. } | Gesture::GroupRotate { .. } => {
                            self.note(Action::Rotate)
                        }
                        Gesture::Guide { .. } => self.note(Action::Transform),
                        _ => {}
                    }
                }
                let (point, constraint) = match gesture {
                    Gesture::Move {
                        start, reference, ..
                    } if mods.ctrl => {
                        let axis = snapping::fixed_axis(*start, point);
                        let mut point = point;
                        point[axis] = start[axis];
                        (point, Some((axis, reference[axis])))
                    }
                    _ => (point, None),
                };
                let mut hit = match gesture.snap_subject(point) {
                    Some((target, subject)) => snapping::DragTargets::resolve(
                        &mut self.drag_targets,
                        self.texture.as_ref().map(FrameTexture::authored),
                        target,
                        doc,
                        frame,
                        style.snap,
                        view.scale,
                        subject,
                        Rect::from_min_max(
                            view.world(response.interact_rect.min),
                            view.world(response.interact_rect.max),
                        ),
                    ),
                    None => snapping::Hit::free(point),
                };
                if let Some((axis, value)) = constraint {
                    hit.constrain(axis, value);
                }
                let target = hit.point;
                edit.snap = Some(hit);
                if release {
                    let selection_commit = gesture.transforms_group()
                        && self.selection_move.can_commit(doc, selection, frame);
                    let asset = gesture
                        .placement_transform()
                        .filter(|&id| self.asset_move.can_release(doc, frame, id));
                    let staged = staged_transform.filter(|staged| {
                        !press
                            && tool == Tool::Select
                            && gesture.single_transform().is_some()
                            && staged.frame == frame
                            && staged.target == target
                            && staged.mods == mods
                            && staged.key_mode == style.key_mode
                            && staged.original.as_ptr() == Arc::as_ptr(doc)
                    });
                    let (changed, new_id) =
                        if let Some(staged) = staged.and_then(|staged| staged.preview.upgrade()) {
                            // The cached full preview already contains the result of
                            // this exact gesture, including authored animation keys.
                            *doc = staged;
                            (true, None)
                        } else {
                            gesture.apply(Arc::make_mut(doc), frame, target, style, mods)
                        };
                    edit.committed |= changed;
                    edit.selection_commit = selection_commit && changed;
                    edit.asset_commit = asset;
                    if changed {
                        // Snap targets were revalidated against the authored drawing
                        // for this release, including its final pointer position.
                        edit.object_commit = self.page_commit_target(gesture, frame);
                    }
                    if let Some(id) = new_id {
                        *selection = vec![id];
                    }
                } else if down {
                    if let Some(object) = gesture
                        .placement_transform()
                        .and_then(|id| doc.objects.iter().find(|o| o.id == id))
                        .filter(|o| {
                            o.is_instance()
                                || (o.image.is_some()
                                    && (matches!(gesture, Gesture::Move { .. })
                                        || (!o.has_stroke_at(frame)
                                            && o.effects.is_empty()
                                            && o.skin.is_none())))
                        })
                    {
                        if self.asset_move.prepare(
                            ui.ctx(),
                            doc,
                            object,
                            frame,
                            view.scale * ui.ctx().pixels_per_point(),
                        ) {
                            // Preview the same authored edit that release will apply, including
                            // a zero-delta or snapped pose when the handle returns to its start.
                            let mut draft = Document {
                                objects: vec![object.clone()],
                                ..Default::default()
                            };
                            gesture.apply(&mut draft, frame, target, style, mods);
                            edit.object_preview = draft.objects.pop().map(Box::new);
                            edit.asset_transform = true;
                            return edit;
                        }
                    }
                    if let Some((object, density)) = self.page_transform_target(
                        gesture,
                        frame,
                        view,
                        ui.ctx().pixels_per_point(),
                        response.interact_rect,
                    ) {
                        // Single-object transforms depend only on the authored target.
                        // Apply the same gesture used for release to an isolated copy.
                        let mut draft = Document {
                            objects: vec![object.clone()],
                            ..Default::default()
                        };
                        if !gesture.apply(&mut draft, frame, target, style, mods).0 {
                            return edit;
                        }
                        let object = draft
                            .objects
                            .pop()
                            .expect("single transform keeps its target");
                        if renderer::wholly_on_artboard(
                            &object,
                            frame,
                            [doc.width, doc.height],
                            density,
                        ) {
                            edit.object_preview = Some(Box::new(object));
                            return edit;
                        }
                    }
                    let mut preview = gesture
                        .reuse_preview(doc, self.previous_drag_document.take())
                        .unwrap_or_else(|| doc.as_ref().clone());
                    if gesture.apply(&mut preview, frame, target, style, mods).0 {
                        let preview = Arc::new(preview);
                        if tool == Tool::Select && gesture.single_transform().is_some() {
                            self.staged_transform = Some(StagedTransform {
                                original: Arc::downgrade(doc),
                                preview: Arc::downgrade(&preview),
                                frame,
                                target,
                                mods,
                                key_mode: style.key_mode,
                            });
                        }
                        edit.preview = Some(preview);
                    }
                }
            }
            _ => {}
        }
        if release || !down {
            self.gesture = None;
            self.drag_targets = None;
        }
        edit
    }

    #[allow(clippy::too_many_arguments)]
    fn select(
        &mut self,
        doc: &Document,
        frame: u32,
        point: Pos2,
        selection: &mut Vec<u64>,
        shift: bool,
        scale: f32,
        scope: SelectionScope,
    ) -> Option<Gesture> {
        let within = |p: Pos2| p.distance(point) * scale <= gesture::HANDLE_HIT;
        if selection.len() > 1 {
            let ids = GestureIds::new(selection.clone());
            if let Some(bounds) = group_bounds(doc, frame, &ids) {
                let rotate = bounds.center_top() - egui::vec2(0., 18. / scale);
                if within(rotate) {
                    return Some(Gesture::GroupRotate {
                        start: point,
                        bounds,
                        ids,
                    });
                }
                if let Some((handle, fixed)) = gesture::group_resize_handles(bounds, scale)
                    .filter(|(handle, _)| gesture::over_resize_grip(point, *handle, scale, shift))
                    .min_by(|(a, _), (b, _)| a.distance_sq(point).total_cmp(&b.distance_sq(point)))
                {
                    return Some(Gesture::GroupResize {
                        start: point,
                        fixed,
                        handle,
                        ids,
                    });
                }
            }
        }
        if let Some(handle) = pivot::target(doc, selection, frame)
            .and_then(|i| gesture::object_handle(&doc.objects[i], frame, point, scale, shift))
        {
            return Some(handle);
        }
        let was_direct = self
            .direct_selection
            .take()
            .is_some_and(|id| selection.as_slice() == [id]);
        if was_direct && shift {
            *selection = scope.resolve(doc, selection);
        }
        if let Some(id) = doc.hit_test_in(point.into(), frame, scope) {
            let members = scope.resolve(doc, &[id]);
            if selection.contains(&id) {
                if shift {
                    let members = GestureIds::new(members);
                    selection.retain(|id| !members.contains(*id));
                    return None;
                } else if was_direct {
                    *selection = members;
                }
            } else if shift {
                selection.extend(members);
            } else {
                *selection = members;
            }
            let ids = GestureIds::new(scope.resolve(doc, selection));
            let object = doc.objects.iter().find(|o| o.id == id)?;
            return Some(Gesture::Move {
                start: point,
                ids,
                reference: object.position_at(frame).into(),
                flips: [false; 2],
            });
        }
        if !shift {
            selection.clear();
        }
        Some(Gesture::Marquee {
            start: point,
            previous: selection.clone(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn start_node(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Arc<Document>,
        frame: u32,
        point: Pos2,
        selection: &mut Vec<u64>,
        mods: egui::Modifiers,
        scale: f32,
        key_mode: crate::timeline::KeyMode,
        scope: SelectionScope,
    ) -> (Option<Gesture>, bool) {
        let radius = 7. / scale;
        let target = doc
            .objects
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, o)| {
                selection.contains(&o.id)
                    && o.visible_at(frame)
                    && document_core::arrange::is_object_editable(doc, o)
            })
            .find_map(|(i, o)| {
                paths::nodes_at(o, frame, point, gesture::HANDLE_HIT / scale, scale)
                    .map(|hit| (i, hit))
            });
        if let Some((i, (index, part, grip))) = target {
            let object = &doc.objects[i];
            let id = object.id;
            *selection = vec![id];
            if mods.alt && part == paths::Part::Anchor {
                let changed = crate::path_editor::delete_node_at(
                    ctx,
                    &mut Arc::make_mut(doc).objects[i],
                    frame,
                    key_mode,
                    index.contour,
                    index.node,
                );
                return (None, changed);
            }
            let anchor = part == paths::Part::Anchor;
            let cycle_on_click =
                ((mods.ctrl || mods.command) && !mods.shift && anchor).then_some(index);
            let nodes = crate::path_editor::selected_nodes(ctx, id, frame);
            let nodes = if mods.shift && anchor {
                let nodes = nodes.with_node(index.contour, index.node);
                crate::path_editor::set_nodes(ctx, id, frame, nodes.clone());
                nodes
            } else if !nodes.contains(index.contour, index.node) || part != paths::Part::Anchor {
                crate::path_editor::select(ctx, id, frame, index.contour, index.node);
                crate::path_editor::selected_nodes(ctx, id, frame)
            } else {
                nodes
            };
            let pull = if mods.ctrl && cycle_on_click.is_some() {
                paths::NodeDrag::pull(object, frame, index)
            } else {
                None
            };
            let edit = pull.unwrap_or(paths::NodeDrag::Control {
                index,
                part: if mods.shift && anchor {
                    paths::Part::SymmetricHandles
                } else {
                    part
                },
                nodes,
                offset: grip - point,
                dragged: false,
            });
            return (
                Some(Gesture::Node {
                    start: point,
                    id,
                    edit,
                    cycle_on_click,
                }),
                false,
            );
        }
        if mods.ctrl || mods.command {
            for &id in selection.iter() {
                let Some(index) = doc
                    .objects
                    .iter()
                    .position(|o| o.id == id && document_core::arrange::is_object_editable(doc, o))
                else {
                    continue;
                };
                if let Some(node) = self.insert_node(
                    doc,
                    frame,
                    index,
                    point,
                    radius,
                    false,
                    selection.len() == 1,
                ) {
                    crate::path_editor::select(ctx, id, frame, node.contour, node.node);
                    return (None, true);
                }
            }
        }
        if mods == egui::Modifiers::NONE {
            if let Some((id, edit)) = doc
                .objects
                .iter()
                .rev()
                .filter(|o| {
                    selection.contains(&o.id)
                        && o.text.is_none()
                        && crate::commands::has_vector_paint(o)
                        && o.visible_at(frame)
                        && document_core::arrange::is_object_editable(doc, o)
                })
                .find_map(|o| {
                    paths::NodeDrag::segment(o, frame, point, radius).map(|edit| (o.id, edit))
                })
            {
                *selection = vec![id];
                return (
                    Some(Gesture::Node {
                        start: point,
                        id,
                        edit,
                        cycle_on_click: None,
                    }),
                    false,
                );
            }
        }
        if let [id] = selection.as_slice() {
            if doc.objects.iter().any(|o| {
                o.id == *id
                    && o.text.is_none()
                    && crate::commands::has_vector_paint(o)
                    && !o.shape.contours().is_empty()
                    && document_core::arrange::is_object_editable(doc, o)
            }) {
                return (
                    Some(Gesture::NodeMarquee {
                        start: point,
                        id: *id,
                        previous: mods
                            .shift
                            .then(|| crate::path_editor::selected_nodes(ctx, *id, frame)),
                        preview: None,
                    }),
                    false,
                );
            }
        }
        *selection = doc
            .hit_test_in(point.into(), frame, scope)
            .into_iter()
            .collect();
        (None, false)
    }
}
fn primary_button(input: &egui::InputState, pressed: bool) -> Option<(Pos2, egui::Modifiers)> {
    input.events.iter().rev().find_map(|event| match event {
        egui::Event::PointerButton {
            button: egui::PointerButton::Primary,
            pressed: state,
            pos,
            modifiers,
        } if *state == pressed => Some((*pos, *modifiers)),
        _ => None,
    })
}

#[derive(Default)]
struct Edit {
    selection_commit: bool,
    committed: bool,
    asset_transform: bool,
    asset_commit: Option<u64>,
    pivot: Option<Pos2>,
    preview: Option<std::sync::Arc<Document>>,
    object_preview: Option<Box<document_core::VectorObject>>,
    object_commit: Option<u64>,
    snap: Option<snapping::Hit>,
}

struct StagedTransform {
    original: Weak<Document>,
    preview: Weak<Document>,
    frame: u32,
    target: Pos2,
    mods: egui::Modifiers,
    key_mode: crate::timeline::KeyMode,
}

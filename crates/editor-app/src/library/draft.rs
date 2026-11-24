use crate::canvas::{Canvas, CanvasStyle, Tool};
use crate::commands::{self, History};
use crate::keymap::{self, Action};
use document_core::{AnimationProject, Document};
use eframe::egui;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
enum Target {
    Symbol(u64),
    Copy {
        clip: usize,
        frame: u32,
        object: u64,
    },
}

/// The isolated draft has its own gesture history. Applying it is one outer project transaction.
pub(super) struct Draft {
    target: Target,
    child: Option<Box<Draft>>,
    name: String,
    temp: AnimationProject,
    previous: AnimationProject,
    history: History,
    canvas: Canvas,
    path_shortcuts: crate::path_ops::Shortcuts,
    style: CanvasStyle,
    path_paints: crate::canvas::PathPaintMemory,
    tool: Tool,
    selection: Vec<u64>,
    error: Option<String>,
}

impl Draft {
    pub(super) fn symbol(id: u64, project: &AnimationProject) -> Self {
        let definition = &project.symbols[&id];
        Self::new(
            Target::Symbol(id),
            definition.name.clone(),
            &definition.artwork,
            project,
        )
    }

    pub(super) fn edit_copy_button(
        ui: &mut egui::Ui,
        project: &AnimationProject,
        clip: usize,
        frame: u32,
        selection: &[u64],
    ) -> Option<Self> {
        let [id] = selection else { return None };
        let doc = project.clips[clip].drawing_at(frame);
        let object = doc.objects.iter().find(|o| o.id == *id)?;
        let art = object.embedded_artwork.as_ref()?;
        ui.add_enabled(
            commands::is_editable(doc, object.layer),
            egui::Button::new("Edit this copy…"),
        )
        .clicked()
        .then(|| {
            Self::new(
                Target::Copy {
                    clip,
                    frame,
                    object: *id,
                },
                object.display_name(),
                art,
                project,
            )
        })
    }

    fn symbol_id(&self) -> Option<u64> {
        match self.target {
            Target::Symbol(id) => Some(id),
            Target::Copy { .. } => None,
        }
    }

    fn new(target: Target, name: String, artwork: &Document, project: &AnimationProject) -> Self {
        let mut temp = AnimationProject::from_document(artwork.clone());
        temp.symbols.clone_from(&project.symbols);
        let mut draft = Self {
            target,
            child: None,
            name,
            previous: temp.clone(),
            temp,
            history: History::default(),
            canvas: Canvas::default(),
            path_shortcuts: Default::default(),
            style: CanvasStyle {
                group_edit: None,
                edit_motion: false,
                key_mode: Default::default(),
                layer: 1,
                fill: [88, 209, 167, 255],
                gradient: None,
                stroke_gradient: None,
                no_fill: false,
                no_stroke: false,
                gradient_channel: crate::canvas::PaintChannel::Fill,
                cap: Default::default(),
                join: Default::default(),
                stroke: [35, 40, 48, 255],
                stroke_width: 0.,
                snap: Default::default(),
            },
            path_paints: Default::default(),
            tool: Tool::Select,
            selection: Vec::new(),
            error: None,
        };
        draft.path_paints.remember(Tool::Pen, &draft.style);
        draft
    }

    fn record(&mut self) {
        if self.previous != self.temp {
            self.history.record(&self.previous, &self.temp, 0, 0);
            self.previous = self.temp.clone();
        }
    }

    fn finish_text(&mut self, ctx: &egui::Context) -> bool {
        if !self.canvas.text_active() {
            return true;
        }
        match self.canvas.finish_text(
            ctx,
            self.temp.clips[0].drawing_at_mut(0),
            &mut self.selection,
        ) {
            Ok(changed) => {
                if changed {
                    self.record();
                }
                self.error = None;
                true
            }
            Err(message) => {
                self.error = Some(message);
                false
            }
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        if self.tool == tool {
            return;
        }
        self.canvas.cancel_drag();
        self.path_paints.switch(self.tool, tool, &mut self.style);
        self.tool = tool;
    }

    fn selection_scope(&self) -> document_core::arrange::SelectionScope {
        self.canvas.selection_scope(&self.style, &self.selection)
    }

    fn shortcuts(&mut self, ui: &egui::Ui) {
        let ctx = ui.ctx();
        let available = ui.is_enabled()
            && !self.canvas.text_active()
            && !keymap::widget_owns_shortcuts(ctx)
            && ui.input(|i| i.focused && !i.pointer.any_down())
            && ctx.memory(|m| {
                m.top_modal_layer()
                    .is_none_or(|layer| layer == ui.layer_id())
            });
        if available && !ctx.memory(|m| m.any_popup_open()) && self.canvas.cancel_placement(ctx) {
            return;
        }
        let count = if available && !self.canvas.path_in_progress() {
            self.selection.len()
        } else {
            0
        };
        for op in self.path_shortcuts.take(ctx, count) {
            self.record();
            match self.temp.clips[0]
                .drawing_at_mut(0)
                .boolean(&self.selection, 0, op)
            {
                Ok(ids) => {
                    self.selection = ids;
                    self.set_tool(Tool::Select);
                    self.error = None;
                    self.record();
                }
                Err(error) => self.error = Some(error),
            }
        }
        if !available {
            return;
        }
        if !self.canvas.path_in_progress() {
            self.style_clipboard(ctx);
        }
        let scope = self.selection_scope();
        let whole_groups = scope == document_core::arrange::SelectionScope::Groups;
        if count > 0
            && !ctx.memory(|m| m.any_popup_open())
            && keymap::pressed(ctx, Action::SimplifyPath)
        {
            self.record();
            let doc = self.temp.clips[0].drawing_at_mut(0);
            if crate::path_editor::simplify::apply(
                ctx,
                doc,
                &self.selection,
                0,
                self.style.key_mode,
            )
            .is_some()
            {
                self.error = None;
                self.record();
            }
            return;
        }
        if count > 0
            && !ctx.memory(|m| m.any_popup_open())
            && keymap::pressed(ctx, Action::LockObjects)
        {
            let doc = self.temp.clips[0].drawing_at_mut(0);
            if whole_groups {
                document_core::arrange::set_object_locks(doc, &self.selection, true);
            } else {
                document_core::arrange::set_selected_locks(doc, &self.selection, true);
            }
            self.selection.clear();
            self.record();
            return;
        }
        if self.tool == Tool::Node && count > 0 && !ctx.memory(|m| m.any_popup_open()) {
            if keymap::pressed(ctx, Action::CombineNodes) {
                self.combine_nodes(ctx);
                return;
            }
            if keymap::pressed(ctx, Action::Delete) {
                self.record();
                if crate::path_editor::delete_selected_nodes(
                    ctx,
                    self.temp.clips[0].drawing_at_mut(0),
                    &self.selection,
                    0,
                    self.style.key_mode,
                ) {
                    self.error = None;
                    self.record();
                }
                return;
            }
            let styles = crate::path_editor::styles::shortcuts(ctx);
            if !styles.is_empty() {
                for style in styles {
                    self.style_nodes(ctx, style);
                }
                return;
            }
        }
        if count > 0 {
            if whole_groups {
                if let Some(action) = crate::editor::edit_commands::grouping_shortcut(ctx) {
                    self.record();
                    action.apply(self.temp.clips[0].drawing_at_mut(0), &mut self.selection);
                    self.record();
                    return;
                }
            }
            use crate::tool_keys::{selection_shortcuts, FLIPS, STACKING};
            if !self.canvas.moving_selection() && !ctx.memory(|m| m.any_popup_open()) {
                let flips = selection_shortcuts(ctx, &FLIPS, false);
                if !flips.is_empty() {
                    for horizontal in flips {
                        self.flip_selection(horizontal);
                    }
                    return;
                }
            }
            for order in selection_shortcuts(ctx, &STACKING, true) {
                commands::arrange_in(
                    self.temp.clips[0].drawing_at_mut(0),
                    &self.selection,
                    0,
                    order,
                    scope,
                );
                self.record();
            }
        }
        if count > 0 && keymap::pressed(ctx, Action::Duplicate) {
            self.record();
            let copies = commands::duplicate_in(
                self.temp.clips[0].drawing_at_mut(0),
                &self.selection,
                scope,
            );
            if !copies.is_empty() {
                self.selection = copies;
                self.set_tool(Tool::Select);
                self.error = None;
                self.record();
            }
            return;
        }
        let map = keymap::active(ctx);
        let command = ui.input(|i| {
            i.events.iter().find_map(|e| {
                let egui::Event::Key {
                    key,
                    modifiers,
                    pressed: true,
                    ..
                } = e
                else {
                    return None;
                };
                map.resolve(*key, *modifiers).filter(|action| {
                    matches!(
                        action,
                        Action::SelectAll
                            | Action::Deselect
                            | Action::Undo
                            | Action::Redo
                            | Action::Delete
                    )
                })
            })
        });
        match command {
            Some(Action::SelectAll) => {
                let doc = self.temp.clips[0].drawing_at(0);
                self.selection = doc
                    .paint_order()
                    .into_iter()
                    .filter(|o| document_core::arrange::is_object_editable(doc, o))
                    .map(|o| o.id)
                    .collect();
            }
            Some(Action::Deselect)
                if !self.canvas.path_in_progress() && !ctx.memory(|m| m.any_popup_open()) =>
            {
                self.selection.clear()
            }
            Some(Action::Undo) => self.restore(false),
            Some(Action::Redo) => self.restore(true),
            Some(Action::Delete) if self.tool != Tool::Node => {
                commands::delete_in(self.temp.clips[0].drawing_at_mut(0), &self.selection, scope);
                self.selection.clear();
            }
            _ => {
                if let Some(tool) = crate::tool_keys::pressed(ctx) {
                    self.set_tool(tool);
                }
            }
        }
    }

    fn combine_nodes(&mut self, ctx: &egui::Context) {
        if crate::path_editor::combine_nodes(
            ctx,
            self.temp.clips[0].drawing_at_mut(0),
            &self.selection,
            0,
            self.style.key_mode,
        ) {
            self.error = None;
            self.record();
        }
    }

    fn flip_selection(&mut self, horizontal: bool) {
        self.record();
        let scope = self.selection_scope();
        let doc = self.temp.clips[0].drawing_at_mut(0);
        if crate::canvas::flip_selection_in(
            doc,
            &self.selection,
            0,
            horizontal,
            self.style.key_mode,
            scope,
        ) {
            self.error = None;
            self.record();
        } else {
            self.error = Some("The selection cannot be flipped at this frame.".into());
        }
    }

    fn style_nodes(&mut self, ctx: &egui::Context, style: document_core::path_edit::HandleStyle) {
        self.record();
        let doc = self.temp.clips[0].drawing_at_mut(0);
        if crate::path_editor::styles::apply(
            ctx,
            doc,
            &self.selection,
            0,
            self.style.key_mode,
            style,
        ) {
            self.error = None;
            self.record();
        }
    }

    fn style_clipboard(&mut self, ctx: &egui::Context) {
        use crate::editor::edit_commands::{clipboard_input, request_paste, ClipboardInput};
        let scope = self.selection_scope();
        for action in clipboard_input(ctx, false) {
            match action {
                ClipboardInput::Copy if !self.selection.is_empty() => {
                    let mut source = commands::Clipboard::default();
                    source.copy_project_in(&self.temp, 0, 0, &self.selection, scope);
                    if !source.is_empty() {
                        source.publish(ctx);
                    }
                }
                ClipboardInput::RequestPaste { style: true } if !self.selection.is_empty() => {
                    request_paste(ctx, true);
                }
                ClipboardInput::Paste { text, style: true } if !self.selection.is_empty() => {
                    if let Some(source) = commands::Clipboard::from_system(ctx, &text) {
                        self.record();
                        let doc = self.temp.clips[0].drawing_at_mut(0);
                        if source.paste_style_in(
                            doc,
                            &self.selection,
                            0,
                            self.style.key_mode,
                            scope,
                        ) > 0
                        {
                            if let Some(object) = doc
                                .objects
                                .iter()
                                .find(|o| self.selection.last() == Some(&o.id) && !o.is_instance())
                            {
                                self.style.read_paint(object, 0);
                                self.path_paints.adopt_style(&self.style);
                            }
                            self.error = None;
                            self.record();
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn restore(&mut self, redo: bool) {
        if self.canvas.step_pen_history(redo) {
            return;
        }
        let changed = if redo {
            self.history.redo(&mut self.temp)
        } else {
            self.history.undo(&mut self.temp)
        };
        if changed {
            self.previous = self.temp.clone();
            self.selection.clear();
            self.canvas.cancel_drag();
        }
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        p: &mut AnimationProject,
        checkerboard: bool,
        pencil_smoothing: &mut u8,
        pencil_pressure: &mut bool,
    ) -> Option<Option<String>> {
        if let Some(child) = &mut self.child {
            if child
                .show(
                    ctx,
                    &mut self.temp,
                    checkerboard,
                    pencil_smoothing,
                    pencil_pressure,
                )
                .is_some()
            {
                self.child = None;
                self.record();
            }
            return None;
        }
        self.canvas.set_checkerboard(checkerboard);
        if crate::inspector::finish_rename_on_blur(ctx, &mut self.temp.clips[0], 0) {
            self.record();
        }
        let session = self.temp.clips[0].session_id();
        crate::path_editor::use_canvas(ctx, session);
        let mut finished = None;
        egui::Modal::new(egui::Id::new("edit-symbol-artwork")).show(ctx, |ui| {
            let screen = ctx.screen_rect().size();
            let width = (screen.x - 64.).clamp(280., 850.);
            let height = (screen.y - 170.).clamp(120., 470.);
            ui.set_width(width);
            let (heading, apply) = match self.target {
                Target::Symbol(_) => ("Edit symbol", "Apply to instances"),
                Target::Copy { .. } => ("Edit independent artwork", "Apply to this copy"),
            };
            ui.heading(format!("{heading} · {}", self.name));
            let old_selection = self.selection.clone();
            let old_tool = self.tool;
            if self.canvas.text_should_finish(ctx) {
                self.finish_text(ctx);
            }
            ui.horizontal(|ui| {
                if ui.button(apply).clicked() && self.finish_text(ctx) {
                    let artwork = self.temp.clips[0].drawing_at(0).clone();
                    let result = match self.target {
                        Target::Symbol(id) => p.replace_symbol_artwork(id, artwork),
                        Target::Copy {
                            clip,
                            frame,
                            object,
                        } => p.replace_embedded_artwork(clip, frame, object, artwork),
                    };
                    match result {
                        Ok(()) => {
                            finished = Some(Some(match self.target {
                                Target::Symbol(_) => {
                                    format!("Updated {} throughout the project.", self.name)
                                }
                                Target::Copy { .. } => format!("Updated {}.", self.name),
                            }))
                        }
                        Err(err) => self.error = Some(err),
                    }
                }
                if ui.button("Cancel").clicked() {
                    finished = Some(None);
                }
            });
            if finished.is_some() {
                return;
            }
            ui.horizontal_wrapped(|ui| {
                for tool in [
                    Tool::Select,
                    Tool::Rectangle,
                    Tool::Ellipse,
                    Tool::Pen,
                    Tool::Pencil,
                    Tool::Node,
                    Tool::Text,
                    Tool::Eyedropper,
                    Tool::Gradient,
                ] {
                    let (name, action) = tool.label_and_action();
                    let shortcut = keymap::label(ctx, action);
                    let label = if shortcut.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{name}  {shortcut}")
                    };
                    if ui.selectable_label(self.tool == tool, label).clicked()
                        && self.finish_text(ctx)
                    {
                        self.set_tool(tool);
                    }
                }
                ui.menu_button("Edit", |ui| {
                    for (action, redo) in [(Action::Undo, false), (Action::Redo, true)] {
                        let enabled =
                            self.canvas.pen_history_available(redo).unwrap_or_else(|| {
                                if redo {
                                    self.history.can_redo()
                                } else {
                                    self.history.can_undo()
                                }
                            });
                        if ui
                            .add_enabled(
                                enabled,
                                egui::Button::new(action.name())
                                    .shortcut_text(keymap::label(ctx, action)),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if self.finish_text(ctx) {
                                self.restore(action == Action::Redo);
                            }
                        }
                    }
                });
                let available = !self.canvas.path_in_progress() && !self.canvas.text_active();
                if self.tool == Tool::Gradient {
                    self.style.gradient_controls(ui);
                }
                if let Some(horizontal) = crate::tool_keys::flip_menu(
                    ui,
                    self.temp.clips[0].drawing_at(0),
                    &self.selection,
                    available,
                ) {
                    self.flip_selection(horizontal);
                }
                if self.tool == Tool::Select
                    && self.canvas.pivot_controls(
                        ui,
                        &mut self.temp.clips[0],
                        0,
                        &self.selection,
                        true,
                    )
                {
                    self.record();
                }
            });
            if self.tool == Tool::Node {
                ui.horizontal(|ui| {
                    if let Some(style) = crate::path_editor::styles::toolbar(
                        ui,
                        self.temp.clips[0].drawing_at(0),
                        &self.selection,
                        0,
                        self.style.key_mode,
                        true,
                    ) {
                        self.style_nodes(ctx, style);
                    }
                    if crate::path_editor::combine_button(
                        ui,
                        self.temp.clips[0].drawing_at(0),
                        &self.selection,
                        0,
                        self.style.key_mode,
                        true,
                    ) {
                        self.combine_nodes(ctx);
                    }
                });
            }
            if self.tool == Tool::Pencil {
                ui.horizontal(|ui| {
                    crate::tool_keys::pencil_controls(ui, pencil_smoothing, pencil_pressure)
                });
                self.canvas
                    .set_pencil_options(*pencil_smoothing, *pencil_pressure);
            }
            if p.symbols.keys().any(|&id| Some(id) != self.symbol_id()) {
                ui.menu_button("Place linked symbol…", |ui| {
                    for (&id, def) in &p.symbols {
                        if Some(id) == self.symbol_id() {
                            continue;
                        }
                        if ui.button(&def.name).clicked() && self.finish_text(ctx) {
                            let art = def.artwork.clone();
                            let doc = self.temp.clips[0].drawing_at_mut(0);
                            if commands::is_editable(doc, self.style.layer) {
                                let object = doc.add_shape(
                                    [0., 0.],
                                    [art.width as f32, art.height as f32],
                                    [0; 4],
                                    document_core::Shape::Rectangle,
                                    self.style.layer,
                                );
                                doc.objects.last_mut().unwrap().symbol =
                                    Some(document_core::SymbolInstance::new(id, art));
                                self.selection = vec![object];
                                self.set_tool(Tool::Select);
                            }
                            ui.close_menu();
                        }
                    }
                });
            }
            self.shortcuts(ui);
            if let Some(copy) = Self::edit_copy_button(ui, &self.temp, 0, 0, &self.selection) {
                self.child = Some(Box::new(copy));
            }
            let clip = &mut self.temp.clips[0];
            let mut finished_pencil = false;
            ui.horizontal_top(|ui| {
                let inspector = (width * 0.35).clamp(120., 235.);
                ui.allocate_ui_with_layout(
                    egui::vec2(width - inspector - 12., height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let editing_text = self.canvas.text_active();
                        match self.canvas.show(
                            ui,
                            clip.drawing_snapshot_mut(0),
                            0,
                            self.tool,
                            &mut self.selection,
                            &self.style,
                            true,
                        ) {
                            Err(e) => self.error = Some(e),
                            Ok(true) if self.tool == Tool::Pencil => finished_pencil = true,
                            Ok(true) if self.tool == Tool::Gradient => {
                                if let Some(object) = clip
                                    .drawing_at(0)
                                    .objects
                                    .iter()
                                    .find(|o| self.selection.as_slice() == [o.id])
                                {
                                    self.style.read_paint(object, 0);
                                }
                            }
                            Ok(_) if editing_text && !self.canvas.text_active() => {
                                self.error = None
                            }
                            Ok(_) => {}
                        }
                        if let Some(color) = self
                            .canvas
                            .update_sampled_paint(&mut self.style, &mut self.path_paints)
                        {
                            crate::color_picker::remember(ui.ctx(), color);
                        }
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(inspector, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(inspector);
                        if !crate::canvas::PathPaintMemory::is_drawing_tool(self.tool)
                            && (old_selection != self.selection
                                || old_tool != self.tool && self.tool == Tool::Select)
                        {
                            if let Some(object) = clip
                                .drawing_at(0)
                                .objects
                                .iter()
                                .find(|o| self.selection.last() == Some(&o.id))
                            {
                                self.style.read_paint(object, 0);
                            }
                        }
                        ui.set_max_height(height);
                        if self.tool == Tool::Text {
                            self.canvas.text_controls(ui);
                            if self.canvas.text_active() {
                                return;
                            }
                            ui.separator();
                        }
                        let before = crate::canvas::DrawingPaint::read(&self.style);
                        let selected = self.selection.clone();
                        if let Some(channel) = ui
                            .push_id(session, |ui| {
                                crate::inspector::show(
                                    ui,
                                    clip,
                                    &mut self.selection,
                                    0,
                                    &mut self.style,
                                    !matches!(self.tool, Tool::Pen | Tool::Pencil),
                                    &self.previous.clips[0].cels[&0],
                                )
                            })
                            .inner
                        {
                            self.canvas.property_pick = Some(channel);
                        }
                        if self.selection == selected {
                            self.path_paints.record_edit(
                                &before,
                                &crate::canvas::DrawingPaint::read(&self.style),
                            );
                        }
                    },
                );
            });
            if finished_pencil {
                self.record();
                for stroke in self.canvas.take_extra_pencil_strokes() {
                    if stroke.apply(self.temp.clips[0].drawing_at_mut(0), &mut self.selection) {
                        self.record();
                    }
                }
            } else if !ui.input(|i| i.pointer.primary_down()) {
                self.record();
            }
            if let Some(error) = self
                .error
                .as_deref()
                .or_else(|| self.canvas.refinement_warning())
            {
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
        });
        finished
    }
}

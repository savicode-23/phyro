use super::{commands, Arrange, CanvasStyle};
use document_core::{arrange::SelectionScope, Clip, Document, KeyValue, Property, Shape};
use eframe::egui::{self, pos2, vec2, Color32, Id, Pos2, Rect, Response, Sense, Stroke, Ui};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

const ROW: f32 = 24.;
use crate::theme::{MINT, SELECTED};
type Group = (u64, u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Row {
    Layer(usize),
    Group(Group),
    Object(usize),
}

#[derive(Clone, Default)]
struct State {
    // Missing entries follow the active layer; an explicit toggle is remembered.
    layers: HashMap<u64, bool>,
    groups: HashSet<Group>,
    layer_focus: Option<u64>,
    last_selection: Option<(u64, usize)>,
}
impl State {
    fn expanded(&self, layer: u64, active: u64) -> bool {
        self.layers.get(&layer).copied().unwrap_or(layer == active)
    }
}

#[derive(Clone, Copy)]
enum NameTarget {
    Group(Group),
    Object(u64),
}

#[derive(Clone)]
struct Rename {
    target: NameTarget,
    field: Id,
    frame: u32,
    exposure: u32,
    text: String,
    rect: Option<Rect>,
}

impl Rename {
    fn apply(mut self, doc: &mut Document) {
        self.text.retain(|c| !c.is_control());
        match self.target {
            NameTarget::Group((layer, group)) => {
                let _ = doc.rename_group(layer, group, &self.text);
            }
            NameTarget::Object(id) => {
                if let Some(index) = doc.objects.iter().position(|o| o.id == id) {
                    if commands::is_editable(doc, doc.objects[index].layer) {
                        doc.objects[index].name = self.text;
                    }
                }
            }
        }
    }
}

fn rename_id(clip: &Clip) -> Id {
    Id::new(("layer-tree-rename", clip.session_id()))
}

// The timeline and dock header process pointer presses before the Layers tab.
// Finish the edit on the outgoing drawing while its owner is still active.
pub(crate) fn finish_rename_on_blur(ctx: &egui::Context, clip: &mut Clip, frame: u32) -> bool {
    let id = rename_id(clip);
    let pending = ctx.data(|d| d.get_temp::<Option<Rename>>(id)).flatten();
    let Some(edit) = pending else {
        return false;
    };
    let leave = ctx.input(|i| {
        i.viewport().close_requested()
            || !i.raw.dropped_files.is_empty()
            || (i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| edit.rect.is_none_or(|r| !r.contains(p))))
    });
    if !leave {
        return false;
    }
    ctx.data_mut(|d| d.remove::<Option<Rename>>(id));
    ctx.memory_mut(|m| m.surrender_focus(edit.field));
    if edit.frame != frame || edit.exposure != clip.exposure(frame).0 {
        return false;
    }
    edit.apply(clip.drawing_at_mut(frame));
    true
}

// Cache hierarchy and paint order independently of names, artwork or animation
// frames. Only the visible rows need text layout, icons, and input responses.
struct Tree {
    layers: Vec<u64>,
    objects: Vec<(u64, u64, Option<u64>)>,
    members: HashMap<Group, Vec<usize>>,
    open_layers: Vec<u64>,
    open_groups: HashSet<Group>,
    rows: Vec<Row>,
}

impl Tree {
    fn matches(&self, doc: &Document, state: &State, active: u64) -> bool {
        self.layers
            .iter()
            .copied()
            .eq(doc.layers.iter().map(|l| l.id))
            && self.open_layers.iter().copied().eq(doc
                .layers
                .iter()
                .filter(|l| state.expanded(l.id, active))
                .map(|l| l.id))
            && self.open_groups == state.groups
            && self.objects.len() == doc.objects.len()
            && self
                .objects
                .iter()
                .zip(&doc.objects)
                .all(|(identity, o)| *identity == (o.id, o.layer, o.group))
    }

    fn build(doc: &Document, state: &State, active: u64) -> Self {
        let mut children = HashMap::<u64, Vec<Row>>::new();
        let mut members = HashMap::<Group, Vec<usize>>::new();
        for (i, o) in doc.objects.iter().enumerate().rev() {
            if !state.expanded(o.layer, active) {
                continue;
            }
            let row = if let Some(id) = o.group {
                let key = (o.layer, id);
                let members = members.entry(key).or_default();
                members.push(i);
                if members.len() != 1 {
                    continue;
                }
                Row::Group(key)
            } else {
                Row::Object(i)
            };
            children.entry(o.layer).or_default().push(row);
        }
        let mut rows = Vec::new();
        for (i, l) in doc.layers.iter().enumerate().rev() {
            rows.push(Row::Layer(i));
            for &row in children.get(&l.id).into_iter().flatten() {
                rows.push(row);
                if let Row::Group(key) = row {
                    if state.groups.contains(&key) {
                        rows.extend(members[&key].iter().copied().map(Row::Object));
                    }
                }
            }
        }
        Self {
            layers: doc.layers.iter().map(|l| l.id).collect(),
            objects: doc
                .objects
                .iter()
                .map(|o| (o.id, o.layer, o.group))
                .collect(),
            members,
            open_layers: doc
                .layers
                .iter()
                .filter(|l| state.expanded(l.id, active))
                .map(|l| l.id)
                .collect(),
            open_groups: state.groups.clone(),
            rows,
        }
    }
}

pub(crate) fn layers(
    ui: &mut Ui,
    clip: &mut Clip,
    selected: &mut Vec<u64>,
    frame: u32,
    style: &mut CanvasStyle,
) {
    let pending_id = rename_id(clip);
    let exposure = clip.exposure(frame).0;
    let id = ui.make_persistent_id("layer-tree");
    let mut state: State = ui.data_mut(|d| d.remove_temp(id)).unwrap_or_default();
    let can_move =
        commands::retain_editable_selection(clip.drawing_at(frame), selected, style.layer);
    let selection = selected.last().map(|&id| (id, selected.len()));
    let reveal = selection != state.last_selection;
    if reveal {
        state.layer_focus = None;
    }
    state.last_selection = selection;
    let mut add_layer = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Layers and objects").strong().size(15.));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let add = ui.button("+");
            add.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Add layer")
            });
            if add.on_hover_text("Add layer").clicked() {
                add_layer = true;
            }
            ui.menu_button("…", |ui| {
                layer_menu(ui, clip, selected, frame, style, can_move);
            });
        });
    });
    if add_layer {
        let id = clip.next_layer_id();
        style.layer = commands::add_layer_with_id(clip.drawing_at_mut(frame), id);
        state.layers.insert(style.layer, true);
        state.layer_focus = Some(style.layer);
        selected.clear();
    }
    let doc = clip.drawing_at(frame);
    ui.separator();
    let key = id.with("hierarchy");
    let cached = ui.data(|d| d.get_temp::<Arc<Tree>>(key));
    let tree = cached
        .filter(|t| t.matches(doc, &state, style.layer))
        .unwrap_or_else(|| {
            let tree = Arc::new(Tree::build(doc, &state, style.layer));
            ui.data_mut(|d| d.insert_temp(key, tree.clone()));
            tree
        });
    let selected_ids: HashSet<_> = selected.iter().copied().collect();
    let mut clicked = None;
    let mut rename = ui
        .data_mut(|d| d.remove_temp::<Option<Rename>>(pending_id))
        .flatten()
        .filter(|edit| edit.frame == frame && edit.exposure == exposure);
    if let Some(edit) = &mut rename {
        edit.rect = None;
    }
    let height = (ui.available_height() - 60.).max(32.);
    egui::ScrollArea::vertical()
        .id_salt("layer-tree-rows")
        .auto_shrink([false, false])
        .min_scrolled_height(0.)
        .max_height(height)
        .show_viewport(ui, |ui, viewport| {
            ui.spacing_mut().item_spacing.y = 0.;
            let total = tree.rows.len();
            if let Some(id) = selected.last().filter(|_| reveal) {
                let target = tree.rows.iter().position(|row| match *row {
                    Row::Object(slot) => doc.objects[slot].id == *id,
                    Row::Group(key) if !state.groups.contains(&key) => {
                        tree.members[&key].iter().any(|&i| doc.objects[i].id == *id)
                    }
                    _ => false,
                });
                if let Some(row) = target {
                    let top = ui.cursor().min + vec2(0., row as f32 * ROW);
                    ui.scroll_to_rect_animation(
                        Rect::from_min_size(top, vec2(1., ROW)),
                        None,
                        egui::style::ScrollAnimation::none(),
                    );
                }
            }
            let visible = if ui.ctx().options(|o| o.screen_reader) {
                0..total
            } else {
                ((viewport.top() / ROW).floor() as usize)
                    .saturating_sub(1)
                    .min(total)
                    ..((viewport.bottom() / ROW).ceil() as usize + 1).min(total)
            };
            ui.set_min_height(total as f32 * ROW);
            ui.add_space(visible.start as f32 * ROW);
            let view = Rows {
                doc,
                tree: &tree,
                state: &state,
                selected: &selected_ids,
                frame,
                exposure,
                active: style.layer,
            };
            for &row in &tree.rows[visible] {
                if let Some(action) = view.row(ui, id, row, &mut rename) {
                    clicked = Some((row, action));
                }
            }
        });
    if let Some(edit) = rename {
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let leave = edit.rect.is_none_or(|rect| {
            ui.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
            })
        });
        if escape || leave || !ui.memory(|m| m.has_focus(edit.field)) {
            ui.memory_mut(|m| m.surrender_focus(edit.field));
            if !escape {
                edit.apply(clip.drawing_at_mut(frame));
            }
        } else {
            ui.data_mut(|d| d.insert_temp(pending_id, Some(edit)));
        }
    }
    if let Some(click) = clicked {
        activate(click, clip, &tree, &mut state, selected, frame, style);
        // The rows have already painted the previous state on the release frame.
        ui.ctx().request_repaint();
    }
    ui.separator();
    footer(
        ui,
        clip,
        selected,
        frame,
        style.layer,
        state.layer_focus == Some(style.layer),
        style.group_scope(),
    );
    ui.data_mut(|d| d.insert_temp(id, state));
}

fn layer_menu(
    ui: &mut Ui,
    clip: &mut Clip,
    selected: &mut Vec<u64>,
    frame: u32,
    style: &mut CanvasStyle,
    can_move: bool,
) {
    let doc = clip.drawing_at(frame);
    let Some(i) = doc.layers.iter().position(|l| l.id == style.layer) else {
        return;
    };
    ui.menu_button("Rename layer…", |ui| {
        let mut name = clip.drawing_at(frame).layers[i].name.clone();
        let response = ui.add(
            egui::TextEdit::singleline(&mut name)
                .desired_width(140.)
                .hint_text("Layer name"),
        );
        if response.changed() {
            clip.drawing_at_mut(frame).layers[i].name = name;
        }
        if ui.button("Done").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            ui.close_menu();
        }
    });
    if ui
        .add_enabled(can_move, egui::Button::new("Move selection here"))
        .clicked()
    {
        commands::move_to_layer(clip.drawing_at_mut(frame), selected, style.layer);
        ui.close_menu();
    }
    let doc = clip.drawing_at(frame);
    ui.add_enabled_ui(doc.layers.len() > 1 && !doc.layers[i].locked, |ui| {
        ui.menu_button("Delete layer…", |ui| {
            let doc = clip.drawing_at(frame);
            let n = doc
                .objects
                .iter()
                .filter(|o| o.layer == style.layer)
                .count();
            ui.label(format!("{} · {n} objects", doc.layers[i].name));
            if ui.button("Delete layer and contents").clicked() {
                let doc = clip.drawing_at_mut(frame);
                doc.objects.retain(|o| o.layer != style.layer);
                doc.layers.remove(i);
                style.layer = doc.layers[i.min(doc.layers.len() - 1)].id;
                commands::retain_editable_selection(doc, selected, style.layer);
                ui.close_menu();
            }
        });
    });
}

enum Action {
    Select(bool),
    Expand,
    Visibility,
    Lock,
}

fn activate(
    (row, action): (Row, Action),
    clip: &mut Clip,
    tree: &Tree,
    state: &mut State,
    selected: &mut Vec<u64>,
    frame: u32,
    style: &mut CanvasStyle,
) {
    let doc = clip.drawing_at(frame);
    if let Row::Layer(i) = row {
        let id = doc.layers[i].id;
        match action {
            Action::Select(_) => {
                style.layer = id;
                state.layer_focus = Some(id);
            }
            Action::Expand => {
                state.layers.insert(id, !state.expanded(id, style.layer));
            }
            Action::Visibility => {
                let layer = &mut clip.drawing_at_mut(frame).layers[i];
                layer.visible = !layer.visible;
            }
            Action::Lock => {
                let layer = &mut clip.drawing_at_mut(frame).layers[i];
                layer.locked = !layer.locked;
            }
        }
        commands::retain_editable_selection(clip.drawing_at(frame), selected, style.layer);
        return;
    }
    let (layer, slots): (_, &[_]) = match row {
        Row::Group(key) => (key.0, &tree.members[&key]),
        Row::Object(ref i) => (doc.objects[*i].layer, std::slice::from_ref(i)),
        _ => unreachable!(),
    };
    match action {
        Action::Expand => {
            if let Row::Group(key) = row {
                if !state.groups.remove(&key) {
                    state.groups.insert(key);
                }
            }
        }
        Action::Lock if commands::is_editable(doc, layer) => {
            let locked = slots.iter().any(|&i| doc.objects[i].locked);
            let id = doc.objects[slots[0]].id;
            let doc = clip.drawing_at_mut(frame);
            if matches!(row, Row::Object(_))
                && style
                    .group_scope()
                    .group()
                    .is_some_and(|g| g.contains(&doc.objects[slots[0]]))
            {
                document_core::arrange::set_selected_locks(doc, &[id], !locked);
            } else {
                document_core::arrange::set_object_locks(doc, &[id], !locked);
            }
            commands::retain_editable_selection(doc, selected, style.layer);
        }
        Action::Select(toggle)
            if commands::is_editable(doc, layer)
                && slots.iter().any(|&i| !doc.objects[i].locked) =>
        {
            state.layer_focus = None;
            style.layer = layer;
            let scope = style.group_scope();
            let inside = scope
                .group()
                .is_some_and(|g| g.contains(&doc.objects[slots[0]]));
            if !inside && style.group_edit.take().is_some() {
                selected.clear();
            }
            let group = doc.objects[slots[0]].group;
            let slots = if inside {
                slots
            } else {
                group
                    .and_then(|g| tree.members.get(&(layer, g)))
                    .map_or(slots, Vec::as_slice)
            };
            let ids: Vec<_> = slots
                .iter()
                .rev()
                .filter(|&&i| !doc.objects[i].locked)
                .map(|&i| doc.objects[i].id)
                .collect();
            let Some(first) = ids.first() else { return };
            if !toggle {
                *selected = ids;
            } else if selected.contains(first) {
                let remove: HashSet<_> = ids.into_iter().collect();
                selected.retain(|id| !remove.contains(id));
            } else {
                let existing: HashSet<_> = selected.iter().copied().collect();
                selected.extend(ids.into_iter().filter(|id| !existing.contains(id)));
            }
        }
        Action::Visibility
            if commands::is_editable(doc, layer)
                && slots.iter().any(|&i| !doc.objects[i].locked) =>
        {
            let show = !slots.iter().any(|&i| {
                let object = &doc.objects[i];
                !object.locked && object.visible_at(frame)
            });
            let doc = clip.drawing_at_mut(frame);
            for &slot in slots {
                let object = &mut doc.objects[slot];
                if object.locked || object.visible_at(frame) == show {
                    continue;
                }
                style
                    .key_mode
                    .set(object, frame, Property::Visibility, KeyValue::Visible(show));
            }
        }
        _ => {}
    }
}

struct Rows<'a> {
    doc: &'a Document,
    tree: &'a Tree,
    state: &'a State,
    selected: &'a HashSet<u64>,
    frame: u32,
    exposure: u32,
    active: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Locks {
    Unlocked,
    Mixed,
    Locked,
}

impl Rows<'_> {
    fn row(
        &self,
        ui: &mut Ui,
        tree_id: Id,
        row: Row,
        rename: &mut Option<Rename>,
    ) -> Option<Action> {
        let (layer, name, icon, depth, expanded, selected, visible, locks) = match row {
            Row::Layer(i) => {
                let l = &self.doc.layers[i];
                (
                    l.id,
                    if l.name.trim().is_empty() {
                        "Unnamed layer".into()
                    } else {
                        l.name.clone()
                    },
                    Icon::Layer,
                    0,
                    Some(self.state.expanded(l.id, self.active)),
                    l.id == self.active
                        && (self.selected.is_empty() || self.state.layer_focus == Some(l.id)),
                    l.visible,
                    Locks::Unlocked,
                )
            }
            Row::Group(key) => {
                let members = &self.tree.members[&key];
                let mut unlocked = 0;
                let mut chosen = 0;
                let mut locked_visible = false;
                let mut unlocked_visible = false;
                for &index in members {
                    let member = &self.doc.objects[index];
                    if member.locked {
                        locked_visible = locked_visible || member.visible_at(self.frame);
                    } else {
                        unlocked += 1;
                        chosen += usize::from(self.selected.contains(&member.id));
                        unlocked_visible = unlocked_visible || member.visible_at(self.frame);
                    }
                }
                (
                    key.0,
                    self.doc.objects[members[0]]
                        .group_name
                        .as_deref()
                        .map_or_else(|| format!("Group {}", key.1), str::to_owned),
                    Icon::Group,
                    1,
                    Some(self.state.groups.contains(&key)),
                    unlocked > 0 && chosen == unlocked,
                    if unlocked == 0 {
                        locked_visible
                    } else {
                        unlocked_visible
                    },
                    if unlocked == 0 {
                        Locks::Locked
                    } else if unlocked == members.len() {
                        Locks::Unlocked
                    } else {
                        Locks::Mixed
                    },
                )
            }
            Row::Object(i) => {
                let o = &self.doc.objects[i];
                let icon = if o.is_instance() {
                    Icon::Layer
                } else if o.image.is_some() {
                    Icon::Image
                } else if o.text.is_some() {
                    Icon::Text
                } else {
                    match o.shape {
                        Shape::Ellipse => Icon::Ellipse,
                        Shape::Rectangle => Icon::Rectangle,
                        _ => Icon::Path,
                    }
                };
                (
                    o.layer,
                    o.display_name(),
                    icon,
                    if o.group.is_some() { 2 } else { 1 },
                    None,
                    self.selected.contains(&o.id),
                    o.visible_at(self.frame),
                    if o.locked {
                        Locks::Locked
                    } else {
                        Locks::Unlocked
                    },
                )
            }
        };
        let l = self.doc.layers.iter().find(|l| l.id == layer)?;
        let is_layer = matches!(row, Row::Layer(_));
        let editable = l.visible && !l.locked && locks != Locks::Locked;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
        let cy = rect.center().y;
        let chip = |x: f32| Rect::from_center_size(pos2(x, cy), vec2(20., ROW));
        let lock_rect = chip(rect.right() - 10.);
        let eye_rect = chip(rect.right() - 30.);
        let start = rect.left() + 4. + depth as f32 * 12.;
        let chevron = chip(start + 6.).intersect(Rect::from_min_max(
            pos2(start, rect.top()),
            pos2(start + 14., rect.bottom()),
        ));
        let title = Rect::from_min_max(
            pos2(chevron.right(), rect.top()),
            pos2(eye_rect.left(), rect.bottom()),
        );
        let key = match row {
            Row::Layer(_) => tree_id.with(("layer", layer)),
            Row::Group(g) => tree_id.with(("group", g)),
            Row::Object(i) => tree_id.with(("object", self.doc.objects[i].id)),
        };
        let field = key.with(("rename", self.frame));
        let editing = rename.as_ref().is_some_and(|edit| edit.field == field);
        let label = ui.interact(
            title,
            key,
            if editing {
                Sense::hover()
            } else {
                Sense::click()
            },
        );
        label.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled() && (is_layer || editable),
                selected,
                &name,
            )
        });
        if label.gained_focus() {
            label.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
        }
        let ink = if !is_layer && !editable {
            ui.visuals().weak_text_color()
        } else {
            ui.visuals().text_color()
        };
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, 2., SELECTED);
            p.rect_filled(Rect::from_min_size(rect.min, vec2(2., ROW)), 0., MINT);
        } else if label.hovered() || label.has_focus() {
            p.rect_filled(rect, 2., ui.visuals().widgets.hovered.bg_fill);
        }
        if depth > 0 {
            let x = rect.left() + depth as f32 * 12. - 4.;
            let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
            p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], stroke);
            p.line_segment([pos2(x, cy), pos2(x + 5., cy)], stroke);
        }
        if let Some(open) = expanded {
            let r = ui.interact(chevron, key.with("expand"), Sense::click());
            let hint = if open { "Collapse" } else { "Expand" };
            r.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("{hint} {name}"),
                )
            });
            draw_icon(
                ui,
                chevron.center(),
                if open { Icon::Open } else { Icon::Closed },
                ink,
            );
            if r.on_hover_text(hint).clicked() {
                return Some(Action::Expand);
            }
        }
        draw_icon(ui, pos2(title.left() + 7., cy), icon, ink);
        let text_left = title.left() + 19.;
        let eye = icon_control(
            ui,
            eye_rect,
            key.with("eye"),
            Icon::Eye(visible),
            is_layer || editable,
            if visible { "Hide" } else { "Show" },
            &name,
        );
        if eye.clicked() && (is_layer || editable) {
            return Some(Action::Visibility);
        }
        let lock = icon_control(
            ui,
            lock_rect,
            key.with("lock"),
            Icon::Lock(l.locked || locks != Locks::Unlocked),
            is_layer || l.visible && !l.locked,
            if !is_layer && l.locked {
                "Locked by layer"
            } else if l.locked || locks != Locks::Unlocked {
                "Unlock"
            } else {
                "Lock"
            },
            &name,
        );
        if lock.clicked() {
            return Some(Action::Lock);
        }
        if !is_layer && editable && locks == Locks::Unlocked {
            let begin = rename_click(ui, &label, tree_id);
            if begin {
                let (target, stored_name) = match row {
                    Row::Group(key) => (
                        NameTarget::Group(key),
                        self.doc.objects[self.tree.members[&key][0]]
                            .group_name
                            .as_deref()
                            .unwrap_or(""),
                    ),
                    Row::Object(i) => (
                        NameTarget::Object(self.doc.objects[i].id),
                        self.doc.objects[i].name.as_str(),
                    ),
                    Row::Layer(_) => unreachable!(),
                };
                *rename = Some(Rename {
                    target,
                    field,
                    frame: self.frame,
                    exposure: self.exposure,
                    text: stored_name.to_owned(),
                    rect: None,
                });
            }
            if let Some(edit) = rename.as_mut().filter(|edit| edit.field == field) {
                let rect =
                    Rect::from_min_max(pos2(text_left, title.top() + 1.), title.max - vec2(1., 1.));
                edit.rect = Some(rect);
                rename_field(ui, rect, field, &mut edit.text, &name, begin);
                return begin.then_some(Action::Select(false));
            }
        }
        let font = egui::FontId::proportional(12.5);
        let galley = egui::WidgetText::from(name.as_str()).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (title.right() - text_left - 2.).max(1.),
            font,
        );
        ui.painter()
            .with_clip_rect(ui.clip_rect().intersect(title))
            .galley(pos2(text_left, cy - galley.size().y / 2.), galley, ink);
        label
            .on_hover_text(&name)
            .clicked()
            .then(|| Action::Select(ui.input(|i| i.modifiers.shift || i.modifiers.command)))
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Layer,
    Group,
    Open,
    Closed,
    Path,
    Ellipse,
    Rectangle,
    Image,
    Text,
    Eye(bool),
    Lock(bool),
}

// egui counts consecutive clicks across the whole window. Both clicks must
// land on the same row's title to start a rename.
fn rename_click(ui: &Ui, row: &Response, tree: Id) -> bool {
    if !row.clicked_by(egui::PointerButton::Primary) || ui.input(|i| i.modifiers.any()) {
        return false;
    }
    let Some(pos) = row.interact_pointer_pos() else {
        return false;
    };
    let now = ui.input(|i| i.time);
    let (delay, distance) = ui.ctx().options(|o| {
        (
            o.input_options.max_double_click_delay,
            o.input_options.max_click_dist,
        )
    });
    ui.data_mut(|d| {
        let key = tree.with("last-name-click");
        let previous = d.get_temp::<(Id, f64, Pos2)>(key);
        if previous.is_some_and(|(id, time, at)| {
            id == row.id && now - time < delay && at.distance(pos) <= distance
        }) {
            d.remove::<(Id, f64, Pos2)>(key);
            true
        } else {
            d.insert_temp(key, (row.id, now, pos));
            false
        }
    })
}

fn rename_field(ui: &mut Ui, rect: Rect, field: Id, value: &mut String, label: &str, begin: bool) {
    if begin {
        ui.memory_mut(|m| m.request_focus(field));
    }
    let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let mut output = egui::TextEdit::singleline(value)
        .id(field)
        .char_limit(80)
        .desired_width(rect.width())
        .hint_text(label)
        .show(&mut cell);
    if begin {
        let range = egui::text::CCursorRange::two(
            egui::text::CCursor::default(),
            egui::text::CCursor::new(value.chars().count()),
        );
        output.state.cursor.set_char_range(Some(range));
        output.state.store(ui.ctx(), field);
    }
}

fn icon_control(
    ui: &Ui,
    rect: Rect,
    id: Id,
    icon: Icon,
    enabled: bool,
    label: &str,
    name: &str,
) -> Response {
    let r = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let caption = format!("{label} {name}");
    r.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled() && enabled,
            &caption,
        )
    });
    let color = if !enabled {
        ui.visuals().weak_text_color().gamma_multiply(0.6)
    } else if r.hovered() || matches!(icon, Icon::Lock(true)) {
        MINT
    } else {
        ui.visuals().text_color()
    };
    draw_icon(ui, rect.center(), icon, color);
    r.on_hover_text(caption)
}

fn draw_icon(ui: &Ui, c: Pos2, icon: Icon, color: Color32) {
    let p = ui.painter();
    let at = |x, y| c + vec2(x, y);
    let stroke = Stroke::new(1.15_f32, color);
    let line = |pts: &[[f32; 2]]| {
        p.add(egui::Shape::line(
            pts.iter().map(|&[x, y]| at(x, y)).collect(),
            stroke,
        ))
    };
    match icon {
        Icon::Open | Icon::Closed => {
            let pts = if matches!(icon, Icon::Open) {
                [[-3., -2.], [3., -2.], [0., 2.]]
            } else {
                [[-2., -3.], [-2., 3.], [2., 0.]]
            };
            p.add(egui::Shape::convex_polygon(
                pts.into_iter().map(|[x, y]| at(x, y)).collect(),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Layer => {
            line(&[[0., -6.], [7., -2.], [0., 2.], [-7., -2.], [0., -6.]]);
            line(&[[-7., 2.], [0., 6.], [7., 2.]]);
        }
        Icon::Group => {
            line(&[
                [-6., -4.],
                [-2., -4.],
                [0., -2.],
                [6., -2.],
                [6., 5.],
                [-6., 5.],
                [-6., -4.],
            ]);
        }
        Icon::Eye(show) => {
            line(&[
                [-7., 0.],
                [-4., -3.],
                [0., -4.],
                [4., -3.],
                [7., 0.],
                [4., 3.],
                [0., 4.],
                [-4., 3.],
                [-7., 0.],
            ]);
            p.circle_filled(c, 1.7, color);
            if !show {
                line(&[[-6., 5.], [6., -5.]]);
            }
        }
        Icon::Lock(locked) => {
            let body = Rect::from_min_max(at(-4.5, 0.), at(4.5, 5.5));
            if locked {
                p.rect_filled(body, 1., color);
                line(&[
                    [-2.8, 0.],
                    [-2.8, -3.5],
                    [-1.4, -5.3],
                    [1.4, -5.3],
                    [2.8, -3.5],
                    [2.8, 0.],
                ]);
            } else {
                p.rect_stroke(body, 1., stroke, egui::StrokeKind::Inside);
                line(&[[-2.8, 0.], [-2.8, -4.], [-1., -6.], [2., -6.], [4., -4.]]);
            }
        }
        Icon::Path => {
            line(&[[-5., 4.], [-3., -1.], [1., -3.], [5., -2.]]);
            for xy in [[-5., 4.], [1., -3.], [5., -2.]] {
                p.circle_filled(at(xy[0], xy[1]), 1.6, color);
            }
        }
        Icon::Ellipse => {
            p.circle_stroke(c, 4.5, stroke);
        }
        Icon::Rectangle => {
            line(&[[-5., -4.], [5., -4.], [5., 4.], [-5., 4.], [-5., -4.]]);
        }
        Icon::Image => {
            line(&[[-5., -4.], [5., -4.], [5., 4.], [-5., 4.], [-5., -4.]]);
            line(&[[-4., 3.], [0., -1.], [4., 3.]]);
            p.circle_filled(at(2.5, -2.), 1., color);
        }
        Icon::Text => {
            line(&[[-5., -4.], [5., -4.]]);
            line(&[[0., -4.], [0., 5.]]);
            line(&[[-2., 5.], [2., 5.]]);
        }
    }
}

fn footer(
    ui: &mut Ui,
    clip: &mut Clip,
    selected: &mut Vec<u64>,
    frame: u32,
    layer: u64,
    layer_focus: bool,
    scope: SelectionScope,
) {
    if let Some((index, active)) = clip
        .drawing_at(frame)
        .layers
        .iter()
        .enumerate()
        .find(|(_, l)| l.id == layer)
    {
        let choice = ui
            .add_enabled_ui(!active.locked, |ui| {
                super::blending::combo(ui, "Layer blend", active.blend_mode, false)
            })
            .inner;
        if let Some(mode) = choice {
            clip.drawing_at_mut(frame).layers[index].blend_mode = mode;
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.;
        let layer_only = selected.is_empty() || layer_focus;
        let doc = clip.drawing_at(frame);
        let at = doc.layers.iter().position(|l| l.id == layer);
        let layer_count = doc.layers.len();
        for (up, order) in [(true, Arrange::Forward), (false, Arrange::Backward)] {
            let available =
                !layer_only || at.is_some_and(|i| if up { i + 1 < layer_count } else { i > 0 });
            let tip = match (layer_only, up) {
                (true, true) => "Raise layer",
                (true, false) => "Lower layer",
                (false, true) => "Raise selection",
                (false, false) => "Lower selection",
            };
            let button = ui.add_enabled(available, egui::Button::new("").min_size(vec2(21., 18.)));
            button.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, available, tip)
            });
            let center = button.rect.center();
            let direction = if up { -1. } else { 1. };
            let color = if available {
                ui.style().interact(&button).fg_stroke.color
            } else {
                ui.visuals().weak_text_color()
            };
            let stroke = Stroke::new(1.35_f32, color);
            let head = center + vec2(0., 4.5 * direction);
            ui.painter()
                .line_segment([center - vec2(0., 4.5 * direction), head], stroke);
            ui.painter().add(egui::Shape::line(
                vec![center - vec2(3.5, 0.), head, center + vec2(3.5, 0.)],
                stroke,
            ));
            if button.on_hover_text(tip).clicked() {
                let doc = clip.drawing_at_mut(frame);
                if layer_only {
                    if let Some(i) = at {
                        doc.layers.swap(i, if up { i + 1 } else { i - 1 });
                    }
                } else {
                    commands::arrange_in(doc, selected, frame, order, scope);
                }
            }
        }
        ui.separator();
        let options = if layer_only || scope != SelectionScope::Groups {
            commands::GroupingOptions::default()
        } else {
            commands::grouping_options(clip.drawing_at(frame), selected)
        };
        if ui
            .add_enabled(options.can_group, egui::Button::new("Group"))
            .clicked()
        {
            commands::GroupAction::Group.apply(clip.drawing_at_mut(frame), selected);
        }
        if ui
            .add_enabled(options.can_ungroup, egui::Button::new("Ungroup"))
            .clicked()
        {
            commands::GroupAction::Ungroup.apply(clip.drawing_at_mut(frame), selected);
        }
    });
}

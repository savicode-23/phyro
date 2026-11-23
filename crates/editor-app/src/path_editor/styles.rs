use super::{NodeSelection, NodeSet};
use crate::keymap::{self, Action};
use crate::{commands::is_editable, timeline::KeyMode};
use document_core::{
    path_edit::{style_nodes, HandleStyle},
    Document, Property, Shape,
};
use eframe::egui::{self, Event};

const STYLES: [(HandleStyle, Action, &str, &str); 3] = [
    (
        HandleStyle::Corner,
        Action::Corner,
        "Corner",
        "Retract both handles to make a sharp corner",
    ),
    (
        HandleStyle::Smooth,
        Action::Smooth,
        "Smooth",
        "Keep handles aligned with independent lengths",
    ),
    (
        HandleStyle::Symmetric,
        Action::Symmetric,
        "Symmetric",
        "Keep handles aligned and equal in length",
    ),
];

pub(super) fn common_style(shape: &Shape, nodes: &NodeSet) -> Option<HandleStyle> {
    let mut selected = nodes
        .runs()
        .iter()
        .flat_map(|&(c, a, b)| &shape.contours()[c][a..=b]);
    let style = selected.next()?.handle_style;
    selected.all(|n| n.handle_style == style).then_some(style)
}

pub(super) fn apply_to_shape(
    shape: &mut Shape,
    size: [f32; 2],
    nodes: &NodeSet,
    style: HandleStyle,
) -> bool {
    let closed = !matches!(shape, Shape::Path { closed: false, .. });
    let mut changed = false;
    for &(c, a, b) in nodes.runs() {
        changed |= style_nodes(&mut shape.contours_mut()[c], closed, size, a..=b, style);
    }
    changed
}

pub(super) fn buttons(
    ui: &mut egui::Ui,
    enabled: bool,
    current: Option<HandleStyle>,
) -> Option<HandleStyle> {
    let mut chosen = None;
    for (style, action, name, tip) in STYLES {
        if ui
            .add_enabled(
                enabled,
                egui::Button::new(name).selected(current == Some(style)),
            )
            .on_hover_ui(|ui| {
                ui.label(keymap::hint(ui.ctx(), tip, action));
            })
            .clicked()
        {
            chosen = Some(style);
        }
    }
    chosen
}

pub(crate) fn shortcuts(ctx: &egui::Context) -> Vec<HandleStyle> {
    let mut chosen = Vec::new();
    let map = keymap::active(ctx);
    ctx.input_mut(|i| {
        if i.focused {
            i.events.retain(|event| {
                if let Event::Key {
                    key,
                    pressed: true,
                    repeat,
                    modifiers,
                    ..
                } = event
                {
                    let action = map.resolve(*key, *modifiers);
                    if let Some((style, ..)) = STYLES.iter().find(|(_, a, ..)| Some(*a) == action) {
                        if !repeat {
                            chosen.push(*style);
                        }
                        return false;
                    }
                }
                true
            });
        }
    });
    chosen
}

fn target(
    ctx: &egui::Context,
    doc: &Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
) -> Option<(usize, NodeSelection)> {
    let [id] = selection else { return None };
    let index = object_index(doc, *id, frame, mode)?;
    Some((index, NodeSelection::load(ctx, *id, frame)))
}

fn object_index(doc: &Document, id: u64, frame: u32, mode: KeyMode) -> Option<usize> {
    let index = doc.objects.iter().position(|o| o.id == id)?;
    let o = &doc.objects[index];
    (o.text.is_none()
        && !o.is_instance()
        && o.visible_at(frame)
        && !o.locked
        && is_editable(doc, o.layer)
        && mode.allows(o, frame, Property::Geometry))
    .then_some(index)
}

pub(crate) fn toolbar(
    ui: &mut egui::Ui,
    doc: &Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
    editable: bool,
) -> Option<HandleStyle> {
    let target = target(ui.ctx(), doc, selection, frame, mode);
    let styles = target.as_ref().and_then(|(i, nodes)| {
        let o = &doc.objects[*i];
        let shape = if o.animated.guided_geometry.is_some() {
            // Corresponding points can introduce additional nodes between poses.
            o.shape_at(frame)
        } else {
            // Indexed tracks keep the preceding key's node types between poses.
            let keys = &o.animated.geometry_keys;
            let preceding = keys
                .range(..=frame)
                .next_back()
                .or_else(|| keys.first_key_value());
            let shape = preceding
                .filter(|_| !Property::Geometry.is_muted(o))
                .map_or(&o.shape, |(_, shape)| shape);
            std::borrow::Cow::Borrowed(shape)
        };
        nodes
            .nodes
            .valid(&shape)
            .then(|| common_style(&shape, &nodes.nodes))
    });
    buttons(ui, editable && styles.is_some(), styles.flatten())
}

pub(crate) fn apply(
    ctx: &egui::Context,
    doc: &mut Document,
    selection: &[u64],
    frame: u32,
    mode: KeyMode,
    style: HandleStyle,
) -> bool {
    let Some((index, selected)) = target(ctx, doc, selection, frame, mode) else {
        return false;
    };
    apply_nodes(
        &mut doc.objects[index],
        frame,
        mode,
        &selected.nodes,
        |_| style,
    )
}

pub(crate) fn cycle_at(
    doc: &mut Document,
    id: u64,
    frame: u32,
    mode: KeyMode,
    contour: usize,
    node: usize,
) -> bool {
    let Some(index) = object_index(doc, id, frame, mode) else {
        return false;
    };
    let nodes = NodeSet::range(contour, node, node);
    apply_nodes(
        &mut doc.objects[index],
        frame,
        mode,
        &nodes,
        |shape| match shape.contours()[contour][node].handle_style {
            HandleStyle::Symmetric => HandleStyle::Smooth,
            HandleStyle::Corner | HandleStyle::Smooth => HandleStyle::Symmetric,
        },
    )
}

fn apply_nodes(
    object: &mut document_core::VectorObject,
    frame: u32,
    mode: KeyMode,
    nodes: &NodeSet,
    style: impl Fn(&Shape) -> HandleStyle,
) -> bool {
    let size = object.size;
    let update =
        |shape: &mut Shape| nodes.valid(shape) && apply_to_shape(shape, size, nodes, style(shape));
    if object.animated.geometry_keys.is_empty() && mode != KeyMode::Auto {
        return update(&mut object.shape);
    }
    let mut pose = object.shape_at(frame).into_owned();
    update(&mut pose)
        && mode.prepare(object, frame, Property::Geometry)
        && object.set_shape_at(frame, pose).is_ok()
}

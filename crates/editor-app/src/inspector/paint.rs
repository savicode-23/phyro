use super::{color_row, commands, gradient, number, opacity, CanvasStyle};
use crate::canvas::PaintChannel::{self, Fill, Stroke};
use document_core::{Document, Gradient, KeyValue, Property, VectorObject};
use eframe::egui::Ui;
use std::{collections::HashSet, sync::Arc};

pub(super) struct ChannelPaint<'a> {
    pub color: [u8; 4],
    pub gradient: Option<&'a Gradient>,
    pub disabled: bool,
}

struct Targets<'a> {
    doc: &'a mut Arc<Document>,
    selected: &'a [u64],
    primary: Option<usize>,
    frame: u32,
    style: &'a mut CanvasStyle,
}

impl Targets<'_> {
    fn source(&self) -> Option<&VectorObject> {
        self.primary.map(|index| &self.doc.objects[index])
    }

    fn paint(&self, channel: PaintChannel) -> ChannelPaint<'_> {
        if let Some(object) = self.source() {
            return ChannelPaint {
                color: channel.color_at(object, self.frame),
                gradient: channel.gradient(object),
                disabled: channel.is_none(object),
            };
        }
        match channel {
            Fill => ChannelPaint {
                color: self.style.fill,
                gradient: self.style.gradient.as_deref(),
                disabled: self.style.no_fill,
            },
            Stroke => ChannelPaint {
                color: self.style.stroke,
                gradient: self.style.stroke_gradient.as_deref(),
                disabled: self.style.no_stroke,
            },
        }
    }

    fn allows(&self, property: Property) -> bool {
        self.source().is_none_or(|o| {
            commands::is_editable(self.doc, o.layer)
                && self.style.key_mode.allows(o, self.frame, property)
        })
    }

    fn for_selected(&mut self, property: Property, mut edit: impl FnMut(&mut VectorObject)) {
        let Some(index) = self.primary else { return };
        let mode = self.style.key_mode;
        let frame = self.frame;
        let allows = |object: &VectorObject| mode.allows(object, frame, property);
        if self.selected.len() == 1 {
            let object = &self.doc.objects[index];
            if commands::is_editable(self.doc, object.layer) && allows(object) {
                edit(&mut Arc::make_mut(self.doc).objects[index]);
            }
        } else {
            let ids: HashSet<_> = self
                .style
                .group_scope()
                .resolve(self.doc, self.selected)
                .into_iter()
                .collect();
            let enabled = |o: &VectorObject| {
                ids.contains(&o.id) && commands::has_vector_paint(o) && allows(o)
            };
            if self.doc.objects.iter().any(enabled) {
                for object in Arc::make_mut(self.doc)
                    .objects
                    .iter_mut()
                    .filter(|o| enabled(o))
                {
                    edit(object);
                }
            }
        }
    }

    fn change_paint(&mut self, channel: PaintChannel, edit: gradient::Edit) {
        let frame = self.frame;
        let mode = self.style.key_mode;
        let disabled = matches!(edit, gradient::Edit::Type(gradient::PaintType::None));
        let keep_previous_paint = disabled || matches!(edit, gradient::Edit::Restore);
        let fallback_width = if self.style.stroke_width > 0. {
            self.style.stroke_width
        } else {
            1.
        };
        let (color, gradient, disabled) = if let Some(primary) = self.primary {
            self.for_selected(channel.property(), |object| {
                let was_disabled = channel.is_none(object);
                channel.set_none(object, disabled);
                if keep_previous_paint {
                    return;
                }
                let old = channel.gradient(object);
                let next = edit.apply(old, channel.color_at(object, frame));
                if channel == Stroke
                    && (was_disabled || old.is_none() && next.is_some())
                    && object.stroke_width_at(frame) == 0.
                {
                    mode.set(
                        object,
                        frame,
                        Property::StrokeWidth,
                        KeyValue::Scalar(fallback_width),
                    );
                }
                *channel.gradient_mut(object) = next;
            });
            let source = &self.doc.objects[primary];
            if channel == Stroke {
                self.style.stroke_width = source.stroke_width_at(frame);
            }
            (
                channel.color_at(source, frame),
                channel.gradient(source).cloned(),
                channel.is_none(source),
            )
        } else {
            let paint = self.paint(channel);
            let color = paint.color;
            let new = edit.apply(paint.gradient, color);
            if channel == Stroke
                && !keep_previous_paint
                && (paint.disabled || paint.gradient.is_none() && new.is_some())
                && self.style.stroke_width == 0.
            {
                self.style.stroke_width = fallback_width;
            }
            (color, new, disabled)
        };
        match channel {
            Fill => {
                self.style.no_fill = disabled;
                self.style.fill = color;
                self.style.gradient = gradient.map(Arc::new);
            }
            Stroke => {
                self.style.no_stroke = disabled;
                self.style.stroke = color;
                self.style.stroke_gradient = gradient.map(Arc::new);
            }
        }
    }

    fn set_solid(&mut self, channel: PaintChannel, color: [u8; 4]) {
        let frame = self.frame;
        let mode = self.style.key_mode;
        self.for_selected(channel.property(), |object| {
            if mode.set(object, frame, channel.property(), KeyValue::Color(color)) {
                channel.set_none(object, false);
                *channel.gradient_mut(object) = None;
            }
        });
        match channel {
            Fill => {
                self.style.no_fill = false;
                self.style.fill = color;
                self.style.gradient = None;
            }
            Stroke => {
                self.style.no_stroke = false;
                self.style.stroke = color;
                self.style.stroke_gradient = None;
            }
        }
    }

    fn row(&mut self, ui: &mut Ui, channel: PaintChannel) -> bool {
        let mut paint = self.paint(channel);
        let row = color_row::show(
            ui,
            channel,
            &mut paint,
            self.selected.len() > 1,
            self.style.gradient_channel == channel,
            self.allows(channel.property()),
        );
        let color = paint.color;
        if row.activate {
            self.style.gradient_channel = channel;
        }
        if let Some(edit) = row.gradient_edit {
            self.change_paint(channel, edit);
        }
        if row.color_changed {
            self.set_solid(channel, color);
        }
        row.picked
    }
}

pub(super) fn show(
    ui: &mut Ui,
    doc: &mut Arc<Document>,
    selected: &[u64],
    style: &mut CanvasStyle,
    frame: u32,
) -> Option<PaintChannel> {
    let primary = selected
        .last()
        .and_then(|id| {
            doc.objects
                .iter()
                .position(|o| o.id == *id && commands::has_vector_paint(o))
        })
        .or_else(|| {
            (selected.len() > 1)
                .then(|| {
                    doc.objects
                        .iter()
                        .rposition(|o| commands::has_vector_paint(o) && selected.contains(&o.id))
                })
                .flatten()
        });
    if !selected.is_empty() && primary.is_none() {
        let image_index = match selected {
            [id] => doc
                .objects
                .iter()
                .position(|o| o.id == *id && o.image.is_some()),
            _ => None,
        };
        if let Some(index) = image_index {
            let object = &doc.objects[index];
            let image = object.image.as_ref().unwrap();
            ui.weak(format!(
                "Embedded image · {} × {} px",
                image.width(),
                image.height()
            ));
            let mut included = !object.reference_only;
            if ui
                .add_enabled(
                    commands::is_editable(doc, object.layer),
                    eframe::egui::Checkbox::new(&mut included, "Include in export"),
                )
                .on_hover_text("Include this image in exported frames, sprite sheets and SVGs.")
                .changed()
            {
                Arc::make_mut(doc).objects[index].reference_only = !included;
            }
        } else {
            ui.weak("Edit the source artwork to change its paint.");
        }
        super::blending::objects(ui, doc, selected, style.group_scope());
        opacity(ui, doc, selected, frame, style.key_mode);
        return None;
    }
    let mut targets = Targets {
        doc,
        selected,
        primary,
        frame,
        style,
    };
    let identity = (selected.last().copied(), frame);
    let mut picker = None;
    for channel in [Fill, Stroke] {
        if ui
            .push_id((channel.name(), identity), |ui| targets.row(ui, channel))
            .inner
        {
            picker = Some(channel);
        }
    }
    let mut width = targets
        .source()
        .map_or(targets.style.stroke_width, |o| o.stroke_width_at(frame));
    if ui
        .horizontal(|ui| {
            color_row::number_label(ui, "Stroke width");
            number(ui, &mut width, 0.2, " px", Some(0.))
        })
        .inner
    {
        targets.style.stroke_width = width;
        let mode = targets.style.key_mode;
        targets.for_selected(Property::StrokeWidth, |o| {
            mode.set(o, frame, Property::StrokeWidth, KeyValue::Scalar(width));
        });
    }
    super::blending::objects(ui, targets.doc, selected, targets.style.group_scope());
    opacity(ui, targets.doc, selected, frame, targets.style.key_mode);
    let channel = targets.style.gradient_channel;
    let paint = targets.paint(channel);
    if let Some(g) = paint.gradient.filter(|_| !paint.disabled) {
        let edit = ui
            .push_id(("gradient", channel.name(), identity), |ui| {
                ui.add_enabled_ui(targets.allows(channel.property()), |ui| {
                    gradient::controls(ui, channel, g, selected.len() > 1)
                })
                .inner
            })
            .inner;
        if let Some(edit) = edit {
            targets.change_paint(channel, edit);
        }
    }
    picker
}

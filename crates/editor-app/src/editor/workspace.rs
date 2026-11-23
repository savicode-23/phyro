use crate::canvas::Tool;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui,
    WidgetInfo, WidgetType,
};

use crate::theme::{MINT, PETROL, RAISED, SELECTED, SURFACE};

pub(super) fn status_line(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(text).truncate());
}

pub(super) fn apply_theme(ctx: &egui::Context) {
    let mut theme = egui::Visuals::dark();
    theme.panel_fill = SURFACE;
    theme.window_fill = RAISED;
    theme.extreme_bg_color = Color32::from_rgb(22, 30, 33);
    theme.faint_bg_color = RAISED;
    theme.widgets.noninteractive.fg_stroke.color = Color32::from_rgb(211, 218, 224);
    theme.widgets.noninteractive.bg_stroke.color = Color32::from_rgb(64, 72, 81);
    theme.widgets.inactive.fg_stroke.color = Color32::from_rgb(218, 224, 229);
    theme.widgets.inactive.weak_bg_fill = Color32::from_rgb(53, 60, 68);
    theme.widgets.hovered.weak_bg_fill = Color32::from_rgb(60, 81, 80);
    theme.widgets.active.bg_fill = SELECTED;
    theme.widgets.active.weak_bg_fill = PETROL;
    theme.widgets.active.fg_stroke.color = MINT;
    theme.hyperlink_color = MINT;
    theme.selection.bg_fill = SELECTED;
    theme.selection.stroke = Stroke::new(1.0_f32, MINT);
    ctx.set_visuals(theme);
}

/// A labeled tool stays identifiable before the artist learns its shortcut.
pub(super) fn tools(ui: &mut Ui, current: Tool) -> Option<Tool> {
    let mut chosen = None;
    let clip = ui.clip_rect();
    let height = clip.bottom() - ui.cursor().top().max(clip.top()) - 2.;
    let row_height = ((height - 8. * ui.spacing().item_spacing.y) / 9.)
        .floor()
        .clamp(27., 37.);
    for tool in [
        Tool::Select,
        Tool::Node,
        Tool::Pen,
        Tool::Pencil,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Text,
        Tool::Eyedropper,
        Tool::Gradient,
    ] {
        let (name, action) = tool.label_and_action();
        let selected = tool == current;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, ui.is_enabled(), selected, name)
        });
        let visuals = ui.style().interact_selectable(&response, selected);
        let ink = if selected {
            MINT
        } else {
            visuals.fg_stroke.color
        };
        let painter = ui.painter();
        if selected || response.hovered() || response.has_focus() || response.highlighted() {
            painter.rect_filled(rect, 4., visuals.weak_bg_fill);
        }
        if response.has_focus() {
            painter.rect_stroke(rect, 4., Stroke::new(1.0_f32, ink), StrokeKind::Inside);
        }
        glyph(
            painter,
            tool,
            pos2(rect.center().x, rect.top() + row_height * 0.29),
            ink,
        );
        painter.text(
            pos2(rect.center().x, rect.bottom() - 2.),
            Align2::CENTER_BOTTOM,
            name,
            FontId::proportional(if tool == Tool::Eyedropper { 10. } else { 11.5 }),
            ink,
        );
        if response
            .on_hover_ui(|ui| {
                ui.label(crate::keymap::hint(ui.ctx(), name, action));
            })
            .clicked()
        {
            chosen = Some(tool);
        }
    }
    chosen
}

fn glyph(painter: &egui::Painter, tool: Tool, center: Pos2, ink: Color32) {
    let p = |x, y| center + vec2(x, y);
    let stroke = Stroke::new(1.5_f32, ink);
    match tool {
        Tool::Select => {
            painter.add(Shape::closed_line(
                vec![
                    p(-5., -7.),
                    p(6., 1.),
                    p(1., 2.),
                    p(3., 7.),
                    p(0., 8.),
                    p(-2., 3.),
                    p(-5., 6.),
                ],
                stroke,
            ));
        }
        Tool::Node => {
            painter.line_segment([p(-7., 4.), p(1., -3.)], stroke);
            painter.line_segment([p(1., -3.), p(7., 0.)], stroke);
            for pos in [p(-7., 4.), p(1., -3.), p(7., 0.)] {
                painter.rect_filled(Rect::from_center_size(pos, vec2(4., 4.)), 0., ink);
            }
        }
        Tool::Rectangle => {
            painter.rect_stroke(
                Rect::from_center_size(center, vec2(15., 11.)),
                0.,
                stroke,
                StrokeKind::Middle,
            );
        }
        Tool::Ellipse => {
            painter.add(egui::epaint::EllipseShape::stroke(
                center,
                vec2(8., 6.),
                stroke,
            ));
        }
        Tool::Pen => {
            painter.add(Shape::closed_line(
                vec![p(-6., 7.), p(-4., -2.), p(2., -7.), p(7., -2.), p(2., 4.)],
                stroke,
            ));
            painter.line_segment([p(-6., 7.), p(0., 0.)], stroke);
            painter.circle_filled(p(0., 0.), 1.5, ink);
        }
        Tool::Pencil => {
            painter.add(Shape::closed_line(
                vec![p(-6., 6.), p(-4., 1.), p(3., -6.), p(6., -3.), p(-1., 4.)],
                stroke,
            ));
            painter.line_segment([p(1., -4.), p(4., -1.)], stroke);
            painter.line_segment([p(-4., 1.), p(-1., 4.)], stroke);
        }
        Tool::Text => {
            painter.line_segment([p(-6., -6.), p(6., -6.)], stroke);
            painter.line_segment([p(0., -6.), p(0., 7.)], stroke);
            painter.line_segment([p(-3., 7.), p(3., 7.)], stroke);
        }
        Tool::Eyedropper => crate::canvas::paint_picker_icon(painter, center, ink),
        Tool::Gradient => {
            painter.line_segment([p(-5., 4.), p(5., -4.)], stroke);
            painter.rect_filled(Rect::from_center_size(p(-5., 4.), vec2(5., 5.)), 0., ink);
            painter.circle_stroke(p(5., -4.), 2.5, stroke);
        }
    }
}

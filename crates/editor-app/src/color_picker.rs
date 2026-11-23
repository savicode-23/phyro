use eframe::egui::{
    self, color_picker, ecolor, vec2, Color32, Key, Mesh, Modifiers, Pos2, Response, Sense, Stroke,
    Ui, Vec2,
};
use std::f32::consts::TAU;
pub(crate) mod recent;
pub(crate) use recent::remember;

const PICKER_WIDTH: f32 = 216.;

#[derive(Clone, Copy, Default, PartialEq)]
struct Hsv {
    hue: f32,
    saturation: f32,
    value: f32,
}

impl Hsv {
    fn from_rgb(rgb: [u8; 3], previous: Self) -> Self {
        let (h, s, v) = ecolor::hsv_from_rgb(rgb.map(|x| f32::from(x) / 255.));
        Self {
            hue: if s == 0. { previous.hue } else { h },
            saturation: if v == 0. { previous.saturation } else { s },
            value: v,
        }
    }

    fn rgb(self) -> [u8; 3] {
        ecolor::rgb_from_hsv((self.hue, self.saturation, self.value))
            .map(|c| (c * 255.).round() as u8)
    }
}

fn pure_hue(hue: f32) -> Color32 {
    let [r, g, b] = Hsv {
        hue,
        saturation: 1.,
        value: 1.,
    }
    .rgb();
    Color32::from_rgb(r, g, b)
}

#[derive(Clone, Copy)]
struct Wheel {
    center: Pos2,
    radius: f32,
}

impl Wheel {
    fn direction(hue: f32) -> Vec2 {
        let (sin, cos) = (TAU * hue).sin_cos();
        vec2(sin, -cos)
    }

    // The triangle stays still while the hue changes so that value and
    // saturation retain the same pointer positions for every hue.
    fn corners(self) -> [Pos2; 3] {
        [0., 2. / 3., 1. / 3.].map(|h| self.center + Self::direction(h) * self.radius * 0.77)
    }

    // Barycentric weights for pure hue and white. The remaining weight is black.
    fn weights(self, p: Pos2) -> (f32, f32) {
        let [h, w, b] = self.corners();
        let color = (b.y - p.y) / (b.y - h.y);
        let white = (b.x - p.x - color * (b.x - h.x)) / (b.x - w.x);
        (color, white)
    }

    fn inside(self, p: Pos2) -> bool {
        let (color, white) = self.weights(p);
        color >= 0. && white >= 0. && color + white <= 1.
    }

    fn set_sv(self, hsv: &mut Hsv, p: Pos2) {
        let p = if self.inside(p) {
            p
        } else {
            let [a, b, c] = self.corners();
            [(a, b), (b, c), (c, a)]
                .map(|(a, b)| {
                    let ab = b - a;
                    a + ab * ((p - a).dot(ab) / ab.length_sq()).clamp(0., 1.)
                })
                .into_iter()
                .min_by(|a, b| a.distance_sq(p).total_cmp(&b.distance_sq(p)))
                .unwrap()
        };
        let (color, white) = self.weights(p);
        hsv.value = (color + white).clamp(0., 1.);
        if hsv.value > 0. {
            hsv.saturation = (color / hsv.value).clamp(0., 1.);
        }
    }

    fn sv_marker(self, hsv: Hsv) -> Pos2 {
        let [h, w, b] = self.corners();
        b + (h - b) * (hsv.saturation * hsv.value) + (w - b) * ((1. - hsv.saturation) * hsv.value)
    }

    fn interact(self, hsv: &mut Hsv, origin: Pos2, pos: Pos2) {
        let distance = self.center.distance(origin);
        if (self.radius * 0.82..=self.radius + 2.).contains(&distance) {
            let d = pos - self.center;
            if d.length_sq() > f32::EPSILON {
                hsv.hue = (d.x.atan2(-d.y) / TAU).rem_euclid(1.);
            }
        } else if self.inside(origin) {
            self.set_sv(hsv, pos);
        }
    }

    fn paint(self, ui: &Ui, hsv: Hsv) {
        const SEGMENTS: u32 = 96;
        let mut mesh = Mesh::default();
        mesh.reserve_vertices(2 * (SEGMENTS as usize + 1) + 3);
        mesh.reserve_triangles(2 * SEGMENTS as usize + 1);
        for i in 0..=SEGMENTS {
            let h = i as f32 / SEGMENTS as f32;
            let radial = Self::direction(h) * self.radius;
            let color = pure_hue(h);
            mesh.colored_vertex(self.center + radial, color);
            mesh.colored_vertex(self.center + radial * 0.82, color);
            if i < SEGMENTS {
                let k = 2 * i;
                mesh.add_triangle(k, k + 1, k + 2);
                mesh.add_triangle(k + 1, k + 3, k + 2);
            }
        }
        let start = mesh.vertices.len() as u32;
        for (point, color) in
            self.corners()
                .into_iter()
                .zip([pure_hue(hsv.hue), Color32::WHITE, Color32::BLACK])
        {
            mesh.colored_vertex(point, color);
        }
        mesh.add_triangle(start, start + 1, start + 2);
        ui.painter().add(mesh);
        for marker in [
            self.center + Self::direction(hsv.hue) * self.radius * 0.91,
            self.sv_marker(hsv),
        ] {
            ui.painter().circle_stroke(
                marker,
                5.4,
                Stroke::new(3.0_f32, Color32::from_black_alpha(210)),
            );
            ui.painter()
                .circle_stroke(marker, 5.4, Stroke::new(1.5_f32, Color32::WHITE));
        }
    }
}

fn wheel(ui: &mut Ui, hsv: &mut Hsv) {
    let side = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ColorButton,
            ui.is_enabled(),
            "Hue, saturation and value",
        )
    });
    let wheel = Wheel {
        center: rect.center(),
        radius: side / 2. - 6.,
    };
    if let Some(pos) = response.interact_pointer_pos() {
        let press = response.id.with("press");
        if ui.input(|i| i.pointer.primary_down()) {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                ui.data_mut(|d| d.insert_temp(press, origin));
                wheel.interact(hsv, origin, pos);
            }
        } else if response.clicked_by(egui::PointerButton::Primary)
            || response.drag_stopped_by(egui::PointerButton::Primary)
        {
            let origin = ui.data_mut(|d| d.remove_temp::<Pos2>(press)).unwrap_or(pos);
            wheel.interact(hsv, origin, pos);
        }
    }
    if response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        ui.input_mut(|i| {
            let mut step = |modifiers, plus, minus| {
                f32::from(i.consume_key(modifiers, plus) as u8)
                    - f32::from(i.consume_key(modifiers, minus) as u8)
            };
            hsv.hue = (hsv.hue + step(Modifiers::NONE, Key::ArrowRight, Key::ArrowLeft) / 360.)
                .rem_euclid(1.);
            hsv.saturation = (hsv.saturation
                + step(Modifiers::SHIFT, Key::ArrowUp, Key::ArrowDown) * 0.01)
                .clamp(0., 1.);
            hsv.value = (hsv.value + step(Modifiers::NONE, Key::ArrowUp, Key::ArrowDown) * 0.01)
                .clamp(0., 1.);
        });
        ui.painter().rect_stroke(
            rect,
            4.,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    wheel.paint(ui, *hsv);
    response.on_hover_text("Drag the ring for hue or the triangle for saturation and value. Arrow keys change hue and value. Shift+Up/Down changes saturation.");
}

fn contents(ui: &mut Ui, rgba: &mut [u8; 4]) {
    ui.set_width(PICKER_WIDTH);
    let id = ui.make_persistent_id("picker color");
    let (last, remembered) = ui
        .data(|d| d.get_temp::<([u8; 3], Hsv)>(id))
        .unwrap_or_default();
    let rgb = [rgba[0], rgba[1], rgba[2]];
    let mut hsv = if rgb == last {
        remembered
    } else {
        Hsv::from_rgb(rgb, remembered)
    };
    let tab_id = ui.make_persistent_id("picker sliders");
    let mut sliders = ui.data(|d| d.get_temp::<bool>(tab_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut sliders, false, "Wheel");
        ui.selectable_value(&mut sliders, true, "Sliders");
    });
    ui.data_mut(|d| d.insert_temp(tab_id, sliders));
    if sliders {
        ui.spacing_mut().slider_width = 210.;
        let mut color = ecolor::Hsva::from_srgba_unmultiplied(*rgba);
        if color.s == 0. {
            color.h = hsv.hue;
        }
        if color_picker::color_picker_hsva_2d(ui, &mut color, color_picker::Alpha::OnlyBlend) {
            *rgba = color.to_srgba_unmultiplied();
            hsv.hue = color.h;
            hsv = Hsv::from_rgb([rgba[0], rgba[1], rgba[2]], hsv);
        }
    } else {
        let before = hsv;
        wheel(ui, &mut hsv);
        if hsv != before {
            rgba[..3].copy_from_slice(&hsv.rgb());
        }
        let before = *rgba;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.;
            ui.spacing_mut().interact_size.x = 42.;
            for (channel, value) in ["R ", "G ", "B ", "A "].into_iter().zip(rgba.iter_mut()) {
                ui.add(egui::DragValue::new(value).speed(0.5).prefix(channel));
            }
        });
        if rgba[..3] != before[..3] {
            hsv = Hsv::from_rgb([rgba[0], rgba[1], rgba[2]], hsv);
        }
    }
    let previous = *rgba;
    recent::swatches(ui, rgba);
    if previous[..3] != rgba[..3] {
        hsv = Hsv::from_rgb([rgba[0], rgba[1], rgba[2]], hsv);
    }
    ui.data_mut(|d| d.insert_temp(id, ([rgba[0], rgba[1], rgba[2]], hsv)));
}

pub(crate) fn button(ui: &mut Ui, rgba: &mut [u8; 4]) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(ui.spacing().interact_size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::new(egui::WidgetType::ColorButton));
    let popup_id = response.id.with("color picker");
    if response.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }
    if ui.is_rect_visible(rect) {
        let [r, g, b, a] = *rgba;
        color_picker::show_color_at(
            ui.painter(),
            Color32::from_rgba_unmultiplied(r, g, b, a),
            rect.shrink(1.),
        );
        let visuals = ui.style().interact(&response);
        ui.painter()
            .rect_stroke(rect, 2., visuals.bg_stroke, egui::StrokeKind::Inside);
    }
    if !ui.memory(|m| m.is_popup_open(popup_id)) {
        return response;
    }
    let height = ui
        .memory(|m| m.area_rect(popup_id))
        .map_or(PICKER_WIDTH + 3. * ui.spacing().interact_size.y, |r| {
            r.height()
        });
    let screen = ui.ctx().screen_rect();
    let below = screen.bottom() - rect.bottom();
    let side = if below < height && rect.top() - screen.top() > below {
        egui::AboveOrBelow::Above
    } else {
        egui::AboveOrBelow::Below
    };
    let before = *rgba;
    egui::popup_above_or_below_widget(
        ui,
        popup_id,
        &response,
        side,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| contents(ui, rgba),
    );
    if before != *rgba {
        response.mark_changed();
        remember(ui.ctx(), *rgba);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_wheel_uses_arrows_for_each_color_component() {
        let ctx = egui::Context::default();
        let mut hsv = Hsv {
            hue: 0.5,
            saturation: 0.5,
            value: 0.5,
        };
        let mut frame = |events, focus| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.set_width(216.);
                        if focus {
                            let id = ui.next_auto_id();
                            ui.memory_mut(|m| m.request_focus(id));
                        }
                        wheel(ui, &mut hsv);
                    });
                },
            );
        };
        frame(vec![], true);
        frame(vec![], false);
        for (key, modifiers) in [
            (Key::ArrowRight, Modifiers::NONE),
            (Key::ArrowDown, Modifiers::NONE),
            (Key::ArrowUp, Modifiers::SHIFT),
        ] {
            frame(
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    modifiers,
                    pressed: true,
                    repeat: false,
                }],
                false,
            );
        }
        assert!((hsv.hue - (0.5 + 1. / 360.)).abs() < 1e-5);
        assert!((hsv.saturation - 0.51).abs() < 1e-5);
        assert!((hsv.value - 0.49).abs() < 1e-5);
    }

    #[test]
    fn triangle_matches_its_painted_colors_and_preserves_hue_through_black() {
        let wheel = Wheel {
            center: Pos2::new(170., 230.),
            radius: 100.,
        };
        let mut hsv = Hsv {
            hue: 1. / 3.,
            saturation: 0.75,
            value: 0.8,
        };
        let marker = wheel.sv_marker(hsv);
        let [green, white, black] = wheel.corners();
        for (position, rgb) in [
            (black, [0; 3]),
            (white, [255; 3]),
            (green, [0, 255, 0]),
            (marker, [51, 204, 51]),
        ] {
            wheel.set_sv(&mut hsv, position);
            assert_eq!(hsv.rgb(), rgb);
        }
        let grey = Hsv::from_rgb([128; 3], hsv);
        assert_eq!(grey.hue, hsv.hue);
        assert_eq!(grey.saturation, 0.);
        let black = Hsv::from_rgb([0; 3], hsv);
        assert_eq!(black.hue, hsv.hue);
        assert_eq!(black.saturation, hsv.saturation);
    }
}

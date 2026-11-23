//! Key values are graphed against clip frames. Tangents belong to the outgoing key.
use document_core::{Easing, KeyValue, Property, VectorObject};
use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Ui};

#[derive(Clone, Copy, Default, PartialEq)]
enum Grip {
    #[default]
    None,
    Out,
    Incoming,
    Key,
}

#[derive(Clone, Copy, Default)]
struct GraphState {
    selected: u32,
    component: usize,
    grip: Grip,
    playhead: Option<u32>,
    drag_limits: Option<(f32, f32)>,
}

fn components(value: &KeyValue) -> &'static [&'static str] {
    match value {
        KeyValue::Pair(_) => &["X", "Y"],
        KeyValue::Color(_) => &["Red", "Green", "Blue", "Alpha"],
        KeyValue::Shape(_) => &["Morph"],
        _ => &["Value"],
    }
}

fn number(v: &KeyValue, c: usize) -> f32 {
    match v {
        KeyValue::Pair(p) => p[c.min(1)],
        KeyValue::Color(p) => f32::from(p[c.min(3)]),
        KeyValue::Scalar(n) => *n,
        KeyValue::Visible(b) => u8::from(*b) as f32,
        KeyValue::Shape(_) => 0.,
    }
}

fn number_edit(v: &mut KeyValue, c: usize, n: f32) {
    match v {
        KeyValue::Pair(p) => p[c.min(1)] = n,
        KeyValue::Color(p) => p[c.min(3)] = n.round().clamp(0., 255.) as u8,
        KeyValue::Scalar(x) => *x = n,
        KeyValue::Visible(x) => *x = n >= 0.5,
        KeyValue::Shape(_) => {}
    }
}

pub fn show(
    ui: &mut Ui,
    object: &mut VectorObject,
    property: Property,
    frame: u32,
    length: u32,
) -> bool {
    let mut changed = false;
    let key = ui.id().with(("graph", object.id, property.label()));
    let mut state = ui
        .data_mut(|d| d.get_temp::<GraphState>(key))
        .unwrap_or_default();
    let frames = property.frames(object);
    if frames.is_empty() {
        ui.weak("Add a key to edit its curve.");
        return false;
    }
    if state.playhead != Some(frame) || !frames.contains(&state.selected) {
        state.selected = frames
            .iter()
            .copied()
            .take_while(|t| *t <= frame)
            .last()
            .unwrap_or(frames[0]);
        state.playhead = Some(frame);
    }
    let mut value = property.get(object, state.selected).unwrap();
    ui.horizontal_wrapped(|ui| {
        for (i, name) in components(&value).iter().enumerate() {
            if components(&value).len() > 1 {
                ui.selectable_value(&mut state.component, i, *name);
            }
        }
        ui.label("Key frame");
        let mut shown = state.selected + 1;
        if ui
            .add(egui::DragValue::new(&mut shown).range(1..=length))
            .changed()
            && property.move_key(object, state.selected, shown - 1).is_ok()
        {
            state.selected = shown - 1;
            changed = true;
        }
        if !matches!(value, KeyValue::Shape(_) | KeyValue::Visible(_)) {
            let mut n = number(&value, state.component);
            if ui.add(egui::DragValue::new(&mut n).speed(0.05)).changed() {
                number_edit(&mut value, state.component, n);
                changed |= property.set(object, state.selected, value.clone());
            }
        }
    });
    let frames = property.frames(object);
    let points: Vec<_> = frames
        .iter()
        .enumerate()
        .map(|(i, &f)| {
            let v = property.get(object, f).unwrap();
            let y = if matches!(v, KeyValue::Shape(_)) {
                i as f32
            } else {
                number(&v, state.component)
            };
            (f, y)
        })
        .collect();
    let (min, max) = points
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), (_, v)| {
            (a.min(*v), b.max(*v))
        });
    let (min, max) = state.drag_limits.unwrap_or((min, max));
    let pad = ((max - min) * 0.12).max(0.01);
    let (outer, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().clamp(185., 660.), 124.),
        Sense::click_and_drag(),
    );
    let rect = outer.shrink2(egui::vec2(9., 10.));
    let (left, right) = (0_u32, length.saturating_sub(1).max(1));
    let point = |(t, y): (f32, f32)| {
        Pos2::new(
            rect.left() + rect.width() * (t - left as f32) / (right - left) as f32,
            rect.bottom() - rect.height() * (y - (min - pad)) / (max - min + 2. * pad),
        )
    };
    let screen = |(t, y): (u32, f32)| point((t as f32, y));
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 3., ui.visuals().extreme_bg_color);
    for i in 0..=4 {
        let f = i as f32 / 4.;
        let x = rect.left() + f * rect.width();
        let y = rect.top() + f * rect.height();
        let stroke = Stroke::new(0.5_f32, ui.visuals().widgets.noninteractive.bg_stroke.color);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            stroke,
        );
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            stroke,
        );
    }
    let tint = Color32::from_rgb(101, 220, 189);
    let mut handles = None;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (start, end) = (screen(a), screen(b));
        let d = end - start;
        let curve = property.easing_at(object, a.0);
        let (out, incoming) = curve.controls();
        let control = |v: [f32; 2]| start + egui::vec2(d.x * v[0], d.y * v[1]);
        let (c1, c2) = (control(out), control(incoming));
        if curve == Easing::Hold {
            let mid = egui::pos2(end.x, start.y);
            painter.line_segment([start, mid], (2., tint));
            painter.line_segment([mid, end], (2., tint));
        } else if curve == Easing::Linear || d.y == 0. {
            painter.line_segment([start, end], (2., tint));
        } else {
            // Normalized easing handles map directly into the graph in both axes.
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [start, c1, c2, end],
                false,
                Color32::TRANSPARENT,
                Stroke::new(2.0_f32, tint),
            ));
        }
        if a.0 == state.selected && curve != Easing::Hold && d.y.abs() > 0.1 {
            for (k, c) in [(start, c1), (end, c2)] {
                painter.line_segment([k, c], (1., Color32::LIGHT_BLUE));
                painter.circle_filled(c, 3., Color32::LIGHT_BLUE);
            }
            handles = Some((start, end, c1, c2));
        }
    }
    for &(t, y) in &points {
        painter.circle_filled(
            screen((t, y)),
            if t == state.selected { 4.5 } else { 3.5 },
            if t == state.selected {
                Color32::GOLD
            } else {
                tint
            },
        );
    }
    if let Some(mouse) = response.interact_pointer_pos() {
        if response.clicked() || response.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(mouse);
            state.grip = Grip::None;
            if let Some(&(t, _)) = points.iter().find(|&&p| screen(p).distance(origin) < 6.) {
                state.selected = t;
                state.grip = Grip::Key;
                if response.drag_started() {
                    state.drag_limits = Some((min, max));
                }
            } else if let Some((_, _, c1, c2)) = handles {
                if c1.distance(origin) < 9. {
                    state.grip = Grip::Out;
                } else if c2.distance(origin) < 9. {
                    state.grip = Grip::Incoming;
                }
            }
        }
        if response.dragged() {
            match state.grip {
                Grip::Out | Grip::Incoming => {
                    if let Some((start, end, _, _)) = handles {
                        let d = end - start;
                        let p = [
                            ((mouse.x - start.x) / d.x).clamp(0., 1.),
                            ((mouse.y - start.y) / d.y).clamp(0., 1.),
                        ];
                        let (mut a, mut b) = property.easing_at(object, state.selected).controls();
                        if state.grip == Grip::Out {
                            a = p
                        } else {
                            b = p
                        }
                        changed |= property.set_easing(
                            object,
                            state.selected,
                            Easing::Cubic {
                                out: a,
                                incoming: b,
                            },
                        );
                    }
                }
                Grip::Key => {
                    let t = (left as f32
                        + (mouse.x - rect.left()) / rect.width() * (right - left) as f32)
                        .round()
                        .clamp(left as f32, right as f32) as u32;
                    if t != state.selected && property.move_key(object, state.selected, t).is_ok() {
                        state.selected = t;
                        changed = true;
                    }
                    if let Some(mut v) = property.get(object, state.selected) {
                        if !matches!(v, KeyValue::Shape(_) | KeyValue::Visible(_)) {
                            number_edit(
                                &mut v,
                                state.component,
                                min - pad
                                    + (rect.bottom() - mouse.y) / rect.height()
                                        * (max - min + 2. * pad),
                            );
                            changed |= property.set(object, state.selected, v);
                        }
                    }
                }
                Grip::None => {}
            }
        }
    }
    if response.drag_stopped() {
        state.drag_limits = None;
        state.grip = Grip::None;
    }
    response.on_hover_text("Click a key to select it. Drag keys to change frame and value, or drag a blue handle to shape the outgoing segment.");
    if property != Property::Visibility && frames.iter().any(|&t| t > state.selected) {
        let old = property.easing_at(object, state.selected);
        let mut curve = old;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(key.with("easing"))
                .selected_text(curve.label())
                .show_ui(ui, |ui| {
                    for v in [
                        Easing::Hold,
                        Easing::Linear,
                        Easing::EaseIn,
                        Easing::EaseOut,
                        Easing::EaseInOut,
                        Easing::Cubic {
                            out: [0.25, 0.1],
                            incoming: [0.75, 0.9],
                        },
                    ] {
                        if ui
                            .selectable_label(v.label() == curve.label(), v.label())
                            .clicked()
                        {
                            curve = v;
                        }
                    }
                });
            let clipboard = egui::Id::new("copied-outgoing-easing");
            if ui.small_button("Copy easing").clicked() {
                ui.data_mut(|d| d.insert_temp(clipboard, curve));
            }
            let copied = ui.data_mut(|d| d.get_temp::<Easing>(clipboard));
            if ui
                .add_enabled(copied.is_some(), egui::Button::new("Paste easing"))
                .clicked()
            {
                curve = copied.unwrap();
            }
        });
        if let Easing::Cubic {
            mut out,
            mut incoming,
        } = curve
        {
            ui.horizontal_wrapped(|ui| {
                for (label, handle) in [("Outgoing", &mut out), ("Incoming", &mut incoming)] {
                    ui.label(label);
                    for n in handle {
                        ui.add(egui::DragValue::new(n).speed(0.01).range(0. ..=1.));
                    }
                }
            });
            curve = Easing::Cubic { out, incoming };
        }
        if curve != old {
            changed |= property.set_easing(object, state.selected, curve);
        }
    }
    ui.data_mut(|d| d.insert_temp(key, state));
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::Document;

    #[test]
    #[ignore = "release benchmark of curve drawing and tessellation"]
    fn curve_key_density() {
        for frames in [24, 100, 1000] {
            for curve in [Easing::Linear, Easing::EaseInOut] {
                let mut doc = Document::default();
                doc.add_rectangle([0.; 2], [10.; 2], [255; 4]);
                let object = &mut doc.objects[0];
                for frame in 0..frames {
                    object.keys.insert(frame, [(frame % 17) as f32, 0.]);
                    Property::Position.set_easing(object, frame, curve);
                }
                let ctx = egui::Context::default();
                let mut times = vec![];
                let mut shapes = 0;
                let mut vertices = 0;
                for i in 0..64 {
                    let playhead = [0, frames / 2, frames - 1][i % 3];
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(1100., 740.),
                        )),
                        ..Default::default()
                    };
                    let start = std::time::Instant::now();
                    let output = ctx.run(input, |ctx| {
                        egui::Window::new("Animation curve")
                            .default_size([324., 196.])
                            .show(ctx, |ui| {
                                assert!(!show(ui, object, Property::Position, playhead, frames));
                            });
                    });
                    shapes = output.shapes.len();
                    let mesh = ctx.tessellate(output.shapes, output.pixels_per_point);
                    let elapsed = start.elapsed().as_secs_f64() * 1000.;
                    vertices = mesh
                        .iter()
                        .map(|p| match &p.primitive {
                            egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                            _ => 0,
                        })
                        .sum::<usize>();
                    if i >= 4 {
                        times.push(elapsed);
                    }
                }
                times.sort_unstable_by(f64::total_cmp);
                println!("{frames} keys, {curve:?}, p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms, {shapes} shapes, {vertices} vertices",
                    times[29], times[56], times[59]);
            }
        }
    }

    #[test]
    fn drawn_key_segments_follow_the_same_time_and_value_as_playback() {
        let mut doc = Document::default();
        doc.add_rectangle([0.; 2], [10.; 2], [255; 4]);
        let object = &mut doc.objects[0];
        let values = [-25., 60., 60., -10., 40., 50., 0., 90.];
        object.rotation_keys.extend((0_u32..).zip(values));
        let easing = [
            Easing::Hold,
            Easing::EaseInOut,
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::Cubic {
                out: [0.92, 0.13],
                incoming: [0.16, 0.84],
            },
        ];
        for (tick, curve) in (0_u32..).zip(easing) {
            Property::Rotation.set_easing(object, tick, curve);
        }
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(530., 320.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(!show(
                        ui,
                        object,
                        Property::Rotation,
                        0,
                        values.len() as u32
                    ));
                });
            },
        );
        let tint = Color32::from_rgb(101, 220, 189);
        let mut keys = vec![];
        let mut lines = vec![];
        let mut cubics = vec![];
        for item in &output.shapes {
            match &item.shape {
                egui::Shape::Circle(c) if c.fill == tint || c.fill == Color32::GOLD => {
                    keys.push(c.center)
                }
                egui::Shape::LineSegment { points, stroke } if stroke.color == tint => {
                    lines.push(*points)
                }
                egui::Shape::CubicBezier(curve) => cubics.push(curve),
                _ => {}
            }
        }
        assert_eq!(keys.len(), values.len());
        let step = egui::pos2(keys[1].x, keys[0].y);
        assert_eq!(
            lines,
            [
                [keys[0], step],
                [step, keys[1]],
                [keys[1], keys[2]],
                [keys[2], keys[3]]
            ]
        );
        assert_eq!(cubics.len(), 4);
        for (i, curve) in cubics.iter().enumerate() {
            let k = i + 3;
            assert_eq!([curve.points[0], curve.points[3]], [keys[k], keys[k + 1]]);
            for sample in 1..100 {
                let screen = curve.sample(sample as f32 / 100.);
                let span = keys[k + 1] - keys[k];
                let time = (screen.x - keys[k].x) / span.x;
                let expected_y = keys[k].y + span.y * easing[k].sample(time.into()) as f32;
                assert!((screen.y - expected_y).abs() < 0.002);
            }
        }
    }
    #[test]
    fn dragging_a_cubic_handle_shapes_the_outgoing_track_in_time() {
        let mut doc = Document::default();
        doc.add_rectangle([0.; 2], [10.; 2], [255; 4]);
        let o = &mut doc.objects[0];
        o.rotation_keys.extend([(0, 0.), (4, 90.)]);
        Property::Rotation.set_easing(o, 0, Easing::EaseInOut);
        let ctx = egui::Context::default();
        let mut render = |events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(530., 320.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        show(ui, o, Property::Rotation, 0, 5);
                    });
                },
            )
        };
        let output = render(vec![]);
        let handle = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Circle(c) if c.fill == Color32::LIGHT_BLUE => Some(c.center),
                _ => None,
            })
            .expect("Outgoing cubic handle is drawn");
        let dest = handle + egui::vec2(-40., -27.);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        render(vec![egui::Event::PointerMoved(handle)]);
        render(vec![button(handle, true)]);
        render(vec![egui::Event::PointerMoved(dest)]);
        render(vec![button(dest, false)]);
        assert!(matches!(
            Property::Rotation.easing_at(o, 0),
            Easing::Cubic { .. }
        ));
        assert!(o.rotation_at(2) > 48.);
        assert_eq!(o.rotation_keys[&0], 0.);
        assert_eq!(o.rotation_keys[&4], 90.);
    }
}

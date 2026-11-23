use super::*;

fn render(
    ctx: &egui::Context,
    timeline: &mut Timeline,
    clip: &mut Clip,
    object: u64,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700., 340.),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                timeline.ruler(ui, clip, Some(object), None);
            });
        },
    )
}

fn bars(output: &egui::FullOutput, track: &str) -> Vec<Rect> {
    let y = label(output, track).center().y;
    output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.fill == paint::TWEEN_BAR && (r.rect.center().y - y).abs() < 1. =>
            {
                Some(r.rect.intersect(s.clip_rect))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn drawing_layers_show_each_held_frame_and_mark_only_drawing_starts() {
    let mut doc = Document {
        frames: 8,
        ..Document::default()
    };
    let object = doc.add_rectangle([4.; 2], [8.; 2], [255; 4]);
    crate::commands::add_layer(&mut doc);
    let mut clip = AnimationProject::from_document(doc).clips.remove(0);
    let mut timeline = Timeline::default();
    let output = render(&egui::Context::default(), &mut timeline, &mut clip, object);
    let first = label(&output, "1").center().x;
    let cell = timeline.frame_width;
    for (name, occupied) in [("Layer 1", true), ("Layer 2", false)] {
        let y = label(&output, name).center().y;
        let mut dividers = Vec::new();
        let mut starts = Vec::new();
        for shape in &output.shapes {
            match &shape.shape {
                egui::Shape::Mesh(mesh) if shape.clip_rect.y_range().contains(y) => {
                    assert_eq!(
                        mesh.texture_id,
                        egui::TextureId::default(),
                        "drawing rows have no thumbnail texture"
                    );
                    for quad in mesh.vertices.as_chunks::<4>().0 {
                        let bounds = Rect::from_two_pos(quad[0].pos, quad[3].pos);
                        if bounds.y_range().contains(y)
                            && bounds.width() <= 2.
                            && bounds.height() >= 10.
                        {
                            dividers.push(bounds.center().x);
                        }
                    }
                }
                egui::Shape::Circle(c)
                    if (c.center.y - y).abs() < 10.
                        && (first - cell * 0.5..first + cell * 7.5).contains(&c.center.x) =>
                {
                    starts.push((c.center.x, c.fill.a() > 0));
                }
                _ => {}
            }
        }
        for tick in 1..8 {
            let edge = first + (tick as f32 - 0.5) * cell;
            assert!(
                dividers.iter().any(|x| (x - edge).abs() <= 1.),
                "missing cell boundary at frame {} in {name}",
                tick + 1
            );
        }
        assert_eq!(
            starts.len(),
            1,
            "a held drawing has one start marker in {name}"
        );
        assert!((starts[0].0 - first).abs() < cell * 0.5);
        assert_eq!(
            starts[0].1, occupied,
            "filled and empty drawings have distinct start markers"
        );
    }
}

#[test]
fn tween_bars_stop_at_holds_equal_values_and_drawing_boundaries() {
    let mut doc = Document {
        frames: 16,
        ..Document::default()
    };
    let object = doc.add_rectangle([4.; 2], [8.; 2], [255; 4]);
    let o = &mut doc.objects[0];
    o.keys.extend([
        (1, [4.; 2]),
        (4, [8.; 2]),
        (6, [2.; 2]),
        (9, [2.; 2]),
        (13, [8.; 2]),
    ]);
    Channel::Position.set_easing(o, 4, document_core::Easing::Hold);
    o.animated.visible_keys.extend([(1, true), (4, false)]);
    o.opacity_keys.extend([(1, 1.), (4, 0.)]);
    Channel::Opacity.set_muted(o, true);
    let mut clip = AnimationProject::from_document(doc).clips.remove(0);
    clip.cels.insert(11, clip.cels[&0].clone());
    let output = render(
        &egui::Context::default(),
        &mut Timeline::default(),
        &mut clip,
        object,
    );
    let position = bars(&output, "Position");
    assert_eq!(position.len(), 1);
    assert!((position[0].left() - label(&output, "2").center().x).abs() < 1.);
    assert!((position[0].right() - label(&output, "5").center().x).abs() < 1.);
    assert!(
        bars(&output, "Visibility").is_empty(),
        "discrete keys have no tween"
    );
    assert!(
        bars(&output, "Opacity").is_empty(),
        "a muted track does not interpolate"
    );
}

#[test]
fn tween_crossing_the_viewport_is_visible_with_both_keys_offscreen() {
    let mut doc = Document {
        frames: 1000,
        ..Document::default()
    };
    let object = doc.add_rectangle([4.; 2], [8.; 2], [255; 4]);
    doc.objects[0].keys.extend([(0, [4.; 2]), (999, [30.; 2])]);
    let mut clip = AnimationProject::from_document(doc).clips.remove(0);
    let mut timeline = Timeline {
        frame: 500,
        reveal_frame: true,
        ..Timeline::default()
    };
    let ctx = egui::Context::default();
    render(&ctx, &mut timeline, &mut clip, object);
    let output = render(&ctx, &mut timeline, &mut clip, object);
    label(&output, "501");
    let visible = bars(&output, "Position");
    assert_eq!(visible.len(), 1);
    assert!(
        visible[0].width() > 500.,
        "the tween spans the visible time columns"
    );
    let y = label(&output, "Position").center().y;
    assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Path(p)
        if p.points.len() == 4 && p.points.iter().all(|pt| (pt.y - y).abs() < 6. && s.clip_rect.contains(*pt)))));
}

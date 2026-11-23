use super::super::{DEFAULT_FRAME_WIDTH, MIN_FRAME_WIDTH};
use super::{layout::HEADER, *};
use document_core::{AnimationProject, Document};
mod frame_cells;

fn label(output: &egui::FullOutput, name: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == name => {
                let rect = t.galley.rect.translate(t.pos.to_vec2());
                s.clip_rect.contains_rect(rect).then_some(rect)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing visible label {name}"))
}

fn selected_ranges(timeline: &Timeline, clip: &Clip) -> Vec<(u32, u32)> {
    timeline.focused_frames(clip).unwrap().ranges().collect()
}

#[test]
fn compact_frames_select_only_valid_columns_and_scroll_while_dragging_past_the_last_row() {
    let ctx = egui::Context::default();
    let mut timeline = Timeline::default();
    let mut clip = AnimationProject::from_document(Document::default())
        .clips
        .remove(0);
    let frame = |timeline: &mut Timeline, clip: &mut Clip, width, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 200.),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    timeline.ruler(ui, clip, None, None);
                });
            },
        )
    };
    for width in [800., 1400.] {
        let short = frame(&mut timeline, &mut clip, width, vec![]);
        assert_eq!(
            label(&short, "3").center().x - label(&short, "2").center().x,
            18.
        );
    }

    timeline.frame_width = MIN_FRAME_WIDTH;
    timeline.reset_for_document();
    assert_eq!(timeline.frame_width, MIN_FRAME_WIDTH);
    let click = |timeline: &mut Timeline, clip: &mut Clip, pos| {
        frame(timeline, clip, 800., vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                timeline,
                clip,
                800.,
                vec![egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
    };
    for count in [1, 2] {
        clip.set_length(count).unwrap();
        let output = frame(&mut timeline, &mut clip, 800., vec![]);
        let first = label(&output, "1");
        click(
            &mut timeline,
            &mut clip,
            first.center() + egui::vec2(48., 0.),
        );
        assert_eq!(timeline.frame, 0);
        assert!(timeline.focused_frames(&clip).is_none());
    }
    timeline.frame_width = DEFAULT_FRAME_WIDTH;
    clip.add_event(1, "Land").unwrap();
    let output = frame(&mut timeline, &mut clip, 800., vec![]);
    assert!(output.shapes.iter().all(|s| !matches!(&s.shape, egui::Shape::Text(t) if matches!(t.galley.text(), "Events" | "Land"))));
    click(&mut timeline, &mut clip, label(&output, "2").center());
    assert_eq!(timeline.frame, 1);
    assert_eq!(clip.events[&1], ["Land"]);
    let output = frame(&mut timeline, &mut clip, 800., vec![]);
    label(&output, "2");

    clip.set_length(1000).unwrap();
    timeline.frame_width = MIN_FRAME_WIDTH;
    let output = frame(&mut timeline, &mut clip, 800., vec![]);
    let y = label(&output, "Layer 1").bottom() + 24.;
    let start = egui::pos2(label(&output, "20").center().x, y);
    let edge = egui::pos2(790., y);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    frame(
        &mut timeline,
        &mut clip,
        800.,
        vec![egui::Event::PointerMoved(start)],
    );
    frame(&mut timeline, &mut clip, 800., vec![button(start, true)]);
    frame(
        &mut timeline,
        &mut clip,
        800.,
        vec![egui::Event::PointerMoved(edge)],
    );
    let first_end = timeline.frame;
    for _ in 0..24 {
        frame(&mut timeline, &mut clip, 800., vec![]);
    }
    frame(&mut timeline, &mut clip, 800., vec![button(edge, false)]);
    assert!(
        first_end > 19 && timeline.frame > first_end,
        "a stationary pointer at the right edge should keep extending the range"
    );
    assert_eq!(selected_ranges(&timeline, &clip), [(19, timeline.frame)]);
}

#[test]
fn bone_row_belongs_to_the_selected_drawing_and_clip() {
    let mut doc = Document::default();
    let id = doc.add_bone("Elbow", None, [1.; 2], [4., 1.]).unwrap();
    let mut project = AnimationProject::from_document(doc);
    let clip = &mut project.clips[0];
    let selected = Some((clip.session_id(), 0, id));
    assert!(BoneTrack::selected(clip, 0, selected).is_none());
    clip.drawing_at_mut(0)
        .set_bone_rotation_key(id, 1, 10.)
        .unwrap();
    let copy = project.duplicate_clip(0, "Copy").unwrap();
    let clip = &mut project.clips[0];
    let new_drawing = clip.duplicate_frame(1).unwrap();
    let (track, _) = BoneTrack::selected(clip, 1, selected).unwrap();
    assert!(track.key_at(clip, 1));
    assert!(!track.key_at(clip, new_drawing));
    assert!(BoneTrack::selected(clip, new_drawing, selected).is_none());
    assert!(BoneTrack::selected(&project.clips[copy], 1, selected).is_none());
}

#[test]
fn pointer_drags_retime_keys_and_drawings_on_release_and_shift_selects_ranges() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([0.; 2], [1.; 2], [255; 4]);
    doc.objects[0].keys.insert(1, [8., 9.]);
    let bone = doc.add_bone("Wrist", None, [0.; 2], [4., 0.]).unwrap();
    doc.set_bone_rotation_key(bone, 0, 0.).unwrap();
    doc.set_bone_rotation_key(bone, 2, 30.).unwrap();
    let mut clip = AnimationProject::from_document(doc).clips.remove(0);
    clip.blank_frame(3).unwrap();
    let mut timeline = Timeline::default();
    let ctx = egui::Context::default();
    let panel = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 500.));
    let dispatch = |timeline: &mut Timeline, clip: &mut Clip, event: egui::Event, modifiers| {
        let mut changed = false;
        let input = egui::RawInput {
            screen_rect: Some(panel),
            events: vec![event],
            modifiers,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let selected_bone = Some((clip.session_id(), 0, bone));
                changed |= timeline.ruler(ui, clip, Some(id), selected_bone);
            });
        });
        changed
    };
    let tracks = Tracks::new(
        &clip,
        0,
        Some(id),
        Some((clip.session_id(), 0, bone)),
        Channel::Position,
        true,
        false,
    );
    let cell = timeline.frame_width;
    let target = |tick: u32, row: Option<Row>| {
        let y = row.map_or(HEADER * 0.5, |row| {
            let band = tracks
                .bands(0., tracks.height)
                .find(|band| band.row == row)
                .unwrap();
            band.top + band.height * 0.5
        });
        egui::pos2(8. + LABEL_WIDTH + (tick as f32 + 0.5) * cell, 8. + y)
    };
    let drag = |timeline: &mut Timeline, clip: &mut Clip, from, to, modifiers| {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        let before = clip.clone();
        dispatch(timeline, clip, egui::Event::PointerMoved(from), modifiers);
        assert!(!dispatch(timeline, clip, button(from, true), modifiers));
        assert!(!dispatch(
            timeline,
            clip,
            egui::Event::PointerMoved(to),
            modifiers
        ));
        assert_eq!(&before, clip);
        dispatch(timeline, clip, button(to, false), modifiers)
    };
    let plain = egui::Modifiers::NONE;
    // Bone diamonds get their own row below the selected object's property.
    assert!(drag(
        &mut timeline,
        &mut clip,
        target(2, Some(Row::Bone)),
        target(1, Some(Row::Bone)),
        plain
    ));
    assert_eq!(clip.drawing_at(1).skeleton.bones[0].rotation_keys[&1], 30.);
    let after_bone_move = clip.clone();
    assert!(!drag(
        &mut timeline,
        &mut clip,
        target(1, Some(Row::Bone)),
        target(0, Some(Row::Bone)),
        plain
    ));
    assert_eq!(clip, after_bone_move, "an occupied key survives a drop");
    assert!(!drag(
        &mut timeline,
        &mut clip,
        target(1, Some(Row::Bone)),
        target(4, Some(Row::Bone)),
        plain
    ));
    assert_eq!(clip, after_bone_move, "keys belong to a single drawing");
    // Clicking an unkeyed cell still seeks; dragging there selects keys.
    assert!(!drag(
        &mut timeline,
        &mut clip,
        target(3, Some(Row::Bone)),
        target(3, Some(Row::Bone)),
        plain
    ));
    assert_eq!(timeline.frame, 3);
    assert!(drag(
        &mut timeline,
        &mut clip,
        target(1, Some(Row::Property(Channel::Position))),
        target(2, Some(Row::Property(Channel::Position))),
        plain
    ));
    assert_eq!(clip.drawing_at(2).objects[0].keys[&2], [8., 9.]);
    // The cell center inside a drawing start selects time. Its leading edge retimes it.
    let drawing_start = target(4, Some(Row::Layer(0)));
    assert!(!drag(
        &mut timeline,
        &mut clip,
        drawing_start,
        target(2, None),
        plain
    ));
    assert_eq!(selected_ranges(&timeline, &clip), [(2, 4)]);
    assert!(!drag(
        &mut timeline,
        &mut clip,
        target(1, None),
        target(0, None),
        egui::Modifiers::SHIFT
    ));
    assert_eq!(selected_ranges(&timeline, &clip), [(0, 4)]);
    assert!(drag(
        &mut timeline,
        &mut clip,
        drawing_start - egui::vec2(cell / 2. - 1., 0.),
        target(3, Some(Row::Layer(0))),
        plain
    ));
    assert_eq!(clip.exposure(0), (0, 3));
    assert!(!drag(
        &mut timeline,
        &mut clip,
        target(1, None),
        target(0, None),
        egui::Modifiers::SHIFT
    ));
    assert_eq!(selected_ranges(&timeline, &clip), [(0, 3)]);
    // A Ctrl drag can start within the selection and extend it without dropping the origin.
    assert!(!drag(
        &mut timeline,
        &mut clip,
        target(2, None),
        target(5, None),
        egui::Modifiers::CTRL
    ));
    assert_eq!(selected_ranges(&timeline, &clip), [(0, 5)]);
}

#[test]
fn drawing_spans_cover_the_visible_ticks_with_no_per_tick_copies() {
    let doc = Document {
        frames: 1000,
        ..Default::default()
    };
    let mut clip = AnimationProject::from_document(doc).clips.remove(0);
    let drawing = clip.cels[&0].clone();
    assert_eq!(
        paint::visible_holds(&clip, 1, 650..=690).collect::<Vec<_>>(),
        [(0, 1000)]
    );
    clip.cels.insert(680, drawing.clone());
    assert_eq!(
        paint::visible_holds(&clip, 1, 650..=690).collect::<Vec<_>>(),
        [(0, 680), (680, 1000)]
    );
    for tick in 1..1000 {
        clip.cels.insert(tick, drawing.clone());
    }
    assert!(paint::visible_holds(&clip, 1, 650..=690).eq((650..=690).map(|t| (t, t + 1))));
}

#[test]
fn named_rows_follow_the_layers_panel_and_only_materialize_visible_rows() {
    let mut doc = Document::default();
    let id = doc.add_rectangle([0.; 2], [10.; 2], [225; 4]);
    for i in 2..=1000 {
        doc.layers.push(document_core::Layer {
            id: i,
            name: format!("Layer {i}"),
            ..Default::default()
        });
    }
    doc.objects[0].layer = 499;
    let clip = AnimationProject::from_document(doc).clips.remove(0);
    let tracks = Tracks::new(&clip, 0, Some(id), None, Channel::Position, true, false);
    let top: Vec<_> = tracks.bands(HEADER, HEADER + 60.).map(|b| b.row).collect();
    assert_eq!(top, [Row::Layer(999), Row::Layer(998)]);
    let (start, end) = tracks.selected_rows().unwrap();
    let selected: Vec<_> = tracks.bands(start, end).map(|b| b.row).collect();
    assert_eq!(
        selected,
        [
            Row::Layer(498),
            Row::Object,
            Row::Property(Channel::Position)
        ]
    );
    assert!(tracks.bands(15_000., 15_120.).count() <= 7);
}

#[test]
fn bone_keys_follow_the_shared_rig_even_when_the_selected_artwork_layer_changes_drawings() {
    let mut doc = Document {
        frames: 12,
        ..Default::default()
    };
    let id = doc.add_rectangle([0.; 2], [5.; 2], [255; 4]);
    let bone = doc.add_bone("Wrist", None, [0.; 2], [4., 0.]).unwrap();
    doc.set_bone_rotation_key(bone, 1, 25.).unwrap();
    let mut clip = AnimationProject::from_document(doc).clips.remove(0);
    let other = clip.cels[&0].clone();
    clip.cels.insert(6, other);
    clip.duplicate_layer_frame(2, 1).unwrap();
    let pick = Some((clip.session_id(), 0, bone));
    let tracks = Tracks::new(&clip, 3, Some(id), pick, Channel::Position, true, false);
    assert_eq!(tracks.exposure.start, 2);
    assert!(tracks.key_at(Row::Bone, &clip, 1));
    let mut keys = KeySelection {
        scope: Some(Scope::new(&clip, 1, Some(id), pick)),
        marks: [(Row::Bone, 1)].into(),
    };
    keys.sync(Scope::new(&clip, 3, Some(id), pick));
    let before = clip.clone();
    keys.edit(&mut clip, Some(2), 3).unwrap();
    clip.sync_layer_edits(&before, 3).unwrap();
    for tick in [0, 2, 5] {
        let map = &clip.drawing_at(tick).skeleton.bones[0].rotation_keys;
        assert_eq!(map.get(&3), Some(&25.));
        assert!(!map.contains_key(&1));
    }
    assert!(keys.edit(&mut clip, Some(3), 6).is_err());
    assert!(BoneTrack::selected(&clip, 6, pick).is_none());
    assert!(clip.drawing_at(6).skeleton.bones[0]
        .rotation_keys
        .contains_key(&1));
}

#[test]
fn object_key_can_cross_a_background_boundary_inside_its_layer_hold() {
    let mut doc = Document {
        frames: 1000,
        ..Default::default()
    };
    let actor = doc.add_rectangle([0.; 2], [14.; 2], [255; 4]);
    doc.objects[0].keys.insert(12, [12., 0.]);
    let backdrop = crate::commands::add_layer(&mut doc);
    doc.layers[0].name = "Actor".into();
    let mut clip = AnimationProject::from_document(doc).clips.remove(0);
    clip.duplicate_layer_frame(15, backdrop).unwrap();
    let before = clip.clone();
    let mut keys = KeySelection {
        scope: Some(Scope::new(&clip, 12, Some(actor), None)),
        marks: [(Row::Property(Channel::Position), 12)].into(),
    };
    keys.sync(Scope::new(&clip, 20, Some(actor), None));
    assert_eq!(keys.marks.len(), 1);
    keys.edit(&mut clip, Some(10), 22).unwrap();
    clip.sync_layer_edits(&before, 22).unwrap();
    for tick in [0, 15, 999] {
        let track = &clip.drawing_at(tick).objects[0].keys;
        assert_eq!(track.get(&22), Some(&[12., 0.]));
        assert!(!track.contains_key(&12));
    }
    assert_eq!(
        paint::visible_holds(&clip, backdrop, 10..=20).collect::<Vec<_>>(),
        [(0, 15), (15, 1000)]
    );
    assert_eq!(
        paint::visible_holds(&clip, 1, 10..=20).collect::<Vec<_>>(),
        [(0, 1000)]
    );
}

#[test]
fn property_diamonds_and_scrubbing_respect_the_selected_exposure() {
    let mut first = Document::default();
    let id = first.add_rectangle([0.; 2], [5.; 2], [200; 4]);
    first.objects[0].name = "Sleeve".into();
    first.objects[0].keys.insert(1, [1.; 2]);
    let mut next = first.clone();
    next.objects[0].name = "Button".into();
    next.objects[0].keys.insert(5, [7.; 2]);
    let mut clip = AnimationProject::from_document(first).clips.remove(0);
    clip.cels.insert(4, next.into());
    let mut timeline = Timeline::default();
    let tracks = Tracks::new(&clip, 0, Some(id), None, Channel::Position, true, false);
    let row = Row::Property(Channel::Position);
    assert!(tracks.key_at(row, &clip, 1));
    assert!(!tracks.key_at(row, &clip, 5));
    let ctx = egui::Context::default();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 300.),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                timeline.ruler(ui, &mut clip, Some(id), None);
            });
        },
    );
    let diamonds = output
        .shapes
        .iter()
        .filter(
            |s| matches!(&s.shape, egui::Shape::Path(p) if p.fill == KEY && p.points.len() == 4),
        )
        .count();
    assert_eq!(
        diamonds, 1,
        "only the sleeve's own key belongs on this track"
    );
}

#[test]
fn ruler_and_labels_stay_pinned_after_scrolling_both_axes() {
    let mut doc = Document {
        frames: 1000,
        ..Default::default()
    };
    let id = doc.add_rectangle([0.; 2], [4.; 2], [255; 4]);
    doc.objects[0].name = "Sleeve".into();
    for property in Channel::ALL {
        property.add(&mut doc.objects[0], 942);
    }
    doc.layers.insert(
        0,
        document_core::Layer {
            id: 2,
            name: "Ground".into(),
            ..Default::default()
        },
    );
    let mut clip = AnimationProject::from_document(doc).clips.remove(0);
    let mut timeline = Timeline::default();
    let ctx = egui::Context::default();
    let frame = |timeline: &mut Timeline, clip: &mut Clip, events: Vec<egui::Event>| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(700., 166.),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    timeline.ruler(ui, clip, Some(id), None);
                });
            },
        )
    };
    let initial = frame(&mut timeline, &mut clip, vec![]);
    let header = label(&initial, "Frame");
    let x = label(&initial, "Layer 1").left();
    let click = |timeline: &mut Timeline, clip: &mut Clip, pos| {
        frame(timeline, clip, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                timeline,
                clip,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
    };
    let group = label(&initial, "Sleeve").center();
    click(&mut timeline, &mut clip, group);
    let collapsed = frame(&mut timeline, &mut clip, vec![]);
    assert_eq!(label(&collapsed, "Ground").left(), x);
    assert!(!collapsed
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "Position")));
    click(&mut timeline, &mut clip, group);
    timeline.frame = 942;
    timeline.reveal_frame = true;
    frame(&mut timeline, &mut clip, vec![]);
    let middle = frame(&mut timeline, &mut clip, vec![]);
    assert_eq!(label(&middle, "Layer 1").left(), x);
    let tick = label(&middle, "943");
    let point = egui::pos2(310., 124.);
    frame(
        &mut timeline,
        &mut clip,
        vec![
            egui::Event::PointerMoved(point),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., -800.),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    for _ in 0..32 {
        frame(&mut timeline, &mut clip, vec![]);
    }
    let bottom = frame(&mut timeline, &mut clip, vec![]);
    assert_eq!(label(&bottom, "Frame"), header);
    assert_eq!(label(&bottom, "943"), tick);
    assert_eq!(label(&bottom, "Ground").left(), x);
    assert!(label(&bottom, "Path progress").left() > x);
    click(&mut timeline, &mut clip, label(&bottom, "940").center());
    assert_eq!(timeline.frame, 939);
    for width in [DEFAULT_FRAME_WIDTH, MIN_FRAME_WIDTH] {
        timeline.frame_width = width;
        timeline.frame = 999;
        timeline.reveal_frame = true;
        frame(&mut timeline, &mut clip, vec![]);
        let last_page = frame(&mut timeline, &mut clip, vec![]);
        let last = label(&last_page, "1000");
        for shape in last_page.shapes {
            if let egui::Shape::Text(t) = shape.shape {
                let bounds = t.galley.rect.translate(t.pos.to_vec2());
                if t.galley.text().parse::<u32>().is_ok() && t.galley.text() != "1000" {
                    assert!(
                        !bounds.intersects(last),
                        "frame number overlapped 1000 at {width} points per frame"
                    );
                }
            }
        }
    }
}

#[test]
fn a_mixed_key_selection_retimes_rig_child_and_easing_only_in_its_own_exposure() {
    use document_core::Easing;
    let mut project = AnimationProject::default();
    project.clips[0].set_length(1000).unwrap();
    let child = project.add_clip("Painted hand").unwrap();
    project.clips[child].set_length(4).unwrap();
    let placed = project
        .place_clip(project.clips[child].id, 0, 0, 1, [0.; 2])
        .unwrap();
    let clip = &mut project.clips[0];
    let doc = clip.drawing_at_mut(0);
    let bone = doc.add_bone("Elbow", None, [0., 0.], [5., 0.]).unwrap();
    doc.set_bone_rotation_key(bone, 48, 30.).unwrap();
    doc.set_drawing_key(placed, 48, 2).unwrap();
    doc.objects[0].rotation_keys.insert(48, 45.);
    assert!(Channel::Rotation.set_easing(&mut doc.objects[0], 48, Easing::Hold));
    let later = clip.cels[&0].clone();
    clip.cels.insert(500, later.clone());
    let mut selection = KeySelection {
        scope: Some(Scope::new(
            clip,
            48,
            Some(placed),
            Some((clip.session_id(), 0, bone)),
        )),
        marks: [
            (Row::Bone, 48),
            (Row::Child, 48),
            (Row::Property(Channel::Rotation), 48),
        ]
        .into(),
    };
    assert!(selection.edit(clip, Some(452), 500).is_err());
    assert!(std::sync::Arc::ptr_eq(&clip.cels[&0], &later));
    selection.edit(clip, Some(-2), 46).unwrap();
    let doc = clip.drawing_at(46);
    assert_eq!(
        doc.skeleton.bones[0]
            .rotation_keys
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [46]
    );
    assert_eq!(
        doc.objects[0].clip_instance.as_ref().unwrap().drawing_keys[&46],
        2
    );
    assert_eq!(
        Channel::Rotation.easing_at(&doc.objects[0], 46),
        Easing::Hold
    );
    assert!(!doc.objects[0].animated.easing[&Channel::Rotation].contains_key(&48));
    selection.edit(clip, None, 46).unwrap();
    let doc = clip.drawing_at(46);
    assert!(doc.skeleton.bones[0].rotation_keys.is_empty());
    assert!(doc.objects[0]
        .clip_instance
        .as_ref()
        .unwrap()
        .drawing_keys
        .is_empty());
    assert!(doc.objects[0].rotation_keys.is_empty());
    assert!(!doc.objects[0]
        .animated
        .easing
        .contains_key(&Channel::Rotation));
    assert!(std::sync::Arc::ptr_eq(&clip.cels[&500], &later));
    project.validate().unwrap();
}

#[test]
fn moving_both_guided_keys_past_their_previous_frames_keeps_the_correspondence() {
    use document_core::{
        morph::{MorphGuide, MorphPoint},
        PathNode, Shape,
    };
    let mut project = AnimationProject::default();
    let clip = &mut project.clips[0];
    clip.set_length(1000).unwrap();
    let doc = clip.drawing_at_mut(0);
    let path = |points: &[[f32; 2]]| Shape::Path {
        closed: false,
        nodes: points.iter().copied().map(PathNode::corner).collect(),
    };
    let start = path(&[[0., 0.4], [1., 0.5]]);
    let end = path(&[[0., 0.2], [0.7, 0.1], [1., 1.]]);
    let id = doc.add_shape([2.; 2], [20.; 2], [255; 4], start.clone(), 1);
    let object = &mut doc.objects[0];
    let guide = MorphGuide {
        from: MorphPoint {
            segment: 0,
            t: 0.35,
        },
        to: MorphPoint { segment: 1, t: 0.0 },
    };
    object
        .set_guided_morph(10, 20, start, end, vec![guide])
        .unwrap();
    let midpoint = object.shape_at(15).into_owned();
    let row = Row::Property(Channel::Geometry);
    let mut selection = KeySelection {
        scope: Some(Scope::new(clip, 15, Some(id), None)),
        marks: [(row, 10), (row, 20)].into(),
    };
    selection.edit(clip, Some(150), 165).unwrap();
    let object = &clip.drawing_at(165).objects[0];
    assert_eq!(object.morph_guides(160, 170), Some([guide].as_slice()));
    assert_eq!(object.shape_at(165).as_ref(), &midpoint);
    project.validate().unwrap();
}

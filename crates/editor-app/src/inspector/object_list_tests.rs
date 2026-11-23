use super::*;
use egui::{Event, Modifiers, Pos2, Rect, Vec2};

#[test]
fn layer_selection_keeps_order_and_only_offers_moves_to_an_editable_layer() {
    let mut doc = Document::default();
    let other = commands::add_layer(&mut doc);
    let hidden = commands::add_layer(&mut doc);
    let locked = commands::add_layer(&mut doc);
    doc.layers
        .iter_mut()
        .find(|l| l.id == hidden)
        .unwrap()
        .visible = false;
    doc.layers
        .iter_mut()
        .find(|l| l.id == locked)
        .unwrap()
        .locked = true;
    let mut object = |layer| doc.add_shape([0.; 2], [3.; 2], [255; 4], Shape::Rectangle, layer);
    let a = object(1);
    let b = object(other);
    let c = object(hidden);
    let d = object(locked);
    for repeats in [1, 5] {
        let input = [b, d, a, u64::MAX, c, b].repeat(repeats);
        let expected = [b, a, b].repeat(repeats);
        for (layer, enabled) in [(1, true), (other, true), (locked, false), (hidden, false)] {
            let mut ids = input.clone();
            assert_eq!(
                commands::retain_editable_selection(&doc, &mut ids, layer),
                enabled
            );
            assert_eq!(ids, expected);
        }
        let mut same = [a, d, c, u64::MAX].repeat(repeats);
        assert!(!commands::retain_editable_selection(&doc, &mut same, 1));
        assert_eq!(same, vec![a; repeats]);
    }
}

struct List {
    ctx: egui::Context,
    clip: document_core::Clip,
    selected: Vec<u64>,
    style: CanvasStyle,
    root: egui::Id,
    scroll: egui::Id,
    width: f32,
}

impl List {
    fn new() -> Self {
        let mut doc = Document::default();
        let second = commands::add_layer(&mut doc);
        doc.layers[0].name = "Sleeve".into();
        doc.layers[1].name = "Lantern".into();
        for _ in 0..10_000 {
            doc.add_rectangle([0.; 2], [1.; 2], [255; 4]);
        }
        doc.objects[1].layer = second;
        doc.objects[0].group = Some(3);
        doc.objects[9999].group = Some(3);
        doc.objects[9999].name = "Cuff".into();
        Self {
            ctx: Default::default(),
            clip: document_core::AnimationProject::from_document(doc)
                .clips
                .remove(0),
            selected: vec![],
            root: egui::Id::NULL,
            scroll: egui::Id::NULL,
            width: 256.,
            style: CanvasStyle {
                fill: [0; 4],
                gradient: None,
                stroke_gradient: None,
                no_fill: false,
                no_stroke: false,
                gradient_channel: crate::canvas::PaintChannel::Fill,
                cap: Default::default(),
                join: Default::default(),
                stroke: [0; 4],
                stroke_width: 0.,
                layer: 1,
                edit_motion: false,
                key_mode: Default::default(),
                snap: Default::default(),
            },
        }
    }
    fn doc(&self) -> &Document {
        self.clip.drawing_at(0)
    }
    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers) -> egui::FullOutput {
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(self.width, 410.))),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    self.root = ui.make_persistent_id("layer-tree");
                    self.scroll = ui.make_persistent_id(egui::Id::new("layer-tree-rows"));
                    layers(ui, &mut self.clip, &mut self.selected, 0, &mut self.style);
                });
            },
        )
    }
    fn click(&mut self, id: egui::Id, mods: Modifiers) {
        self.frame(vec![], mods);
        let pos = self
            .ctx
            .read_response(id)
            .expect("visible widget")
            .rect
            .center();
        self.frame(vec![Event::PointerMoved(pos)], mods);
        for pressed in [true, false] {
            self.frame(
                vec![Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: mods,
                }],
                mods,
            );
        }
    }
}

#[test]
fn separate_layer_and_group_disclosures_and_editable_controls_preserve_selection() {
    let mut w = List::new();
    w.frame(vec![], Modifiers::NONE);
    let g = w.root.with(("group", (1u64, 3u64)));
    let s = w.root.with(("layer", 1u64));
    let (first, last) = (w.doc().objects[0].id, w.doc().objects[9999].id);
    assert!(w.ctx.read_response(w.root.with(("object", last))).is_none());
    w.click(g.with("expand"), Modifiers::NONE);
    w.click(w.root.with(("object", last)), Modifiers::NONE);
    assert_eq!(w.selected, [first, last]);
    w.click(
        w.root.with(("object", w.doc().objects[9998].id)),
        Modifiers::SHIFT,
    );
    assert_eq!(w.selected.len(), 3);
    w.click(g, Modifiers::SHIFT);
    assert_eq!(w.selected, [w.doc().objects[9998].id]);
    w.click(g.with("eye"), Modifiers::NONE);
    assert!(!w.doc().objects[0].visible_at(0) && !w.doc().objects[9999].visible_at(0));
    w.click(s.with("lock"), Modifiers::NONE);
    assert!(w.doc().layers[0].locked && w.selected.is_empty());
    w.click(s.with("expand"), Modifiers::NONE);
    let collapsed = w.frame(vec![], Modifiers::NONE);
    assert!(!collapsed
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text()=="Group 3")));
}

#[test]
fn keyboard_focus_crosses_virtual_rows_and_selects_its_artwork() {
    let mut w = List::new();
    w.frame(vec![], Modifiers::NONE);
    let start = w.root.with(("object", w.doc().objects[9998].id));
    let target = w.root.with(("object", w.doc().objects[9979].id));
    w.ctx.memory_mut(|m| m.request_focus(start));
    let press = |w: &mut List, k| {
        for pressed in [true, false] {
            w.frame(
                vec![Event::Key {
                    key: k,
                    pressed,
                    repeat: false,
                    physical_key: None,
                    modifiers: Modifiers::NONE,
                }],
                Modifiers::NONE,
            );
        }
    };
    for _ in 0..80 {
        if w.ctx.memory(|m| m.has_focus(target)) {
            break;
        }
        press(&mut w, egui::Key::Tab);
    }
    assert!(w.ctx.memory(|m| m.has_focus(target)));
    press(&mut w, egui::Key::Enter);
    assert_eq!(w.selected, [w.doc().objects[9979].id]);
}

#[test]
fn virtual_tree_keeps_footer_visible_and_scrolls_to_the_last_of_ten_thousand_objects() {
    let mut w = List::new();
    let frame = w.frame(vec![], Modifiers::NONE);
    let footer = |o: &egui::FullOutput| {
        o.shapes
            .iter()
            .rev()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Ungroup" => Some(t.pos),
                _ => None,
            })
            .unwrap()
    };
    let pos = footer(&frame);
    assert!(frame.shapes.len() < 250);
    let first_visible = w.root.with(("object", w.doc().objects[9998].id));
    let last_visible = w.root.with(("object", w.doc().objects[2].id));
    assert!(w.ctx.read_response(first_visible).is_some());
    assert!(w.ctx.read_response(last_visible).is_none());
    let mut scroll = egui::scroll_area::State::load(&w.ctx, w.scroll).expect("layer viewport");
    scroll.offset.y = 250_000.;
    scroll.store(&w.ctx, w.scroll);
    w.frame(vec![], Modifiers::NONE);
    let end = w.frame(vec![], Modifiers::NONE);
    assert_eq!(footer(&end), pos);
    assert!(end.shapes.len() < 250);
    w.click(last_visible, Modifiers::NONE);
    assert_eq!(w.selected, [w.doc().objects[2].id]);
    w.ctx.options_mut(|o| o.screen_reader = true);
    w.frame(vec![], Modifiers::NONE);
    assert!(w.ctx.read_response(first_visible).is_some());
}

#[test]
fn narrow_dock_keeps_the_header_and_actions_on_their_own_rows() {
    let mut w = List::new();
    w.width = 180.;
    let out = w.frame(vec![], Modifiers::NONE);
    let bounds = |label| {
        out.shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect()),
                _ => None,
            })
            .unwrap()
    };
    let title = bounds("Layers and objects");
    let add = bounds("+");
    let more = bounds("…");
    assert!(title.right() <= more.left());
    assert!(more.right() <= add.left() && add.right() <= w.width);
    let ungroup = bounds("Ungroup");
    let arrows: Vec<_> = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Path(path)
                if path.points.len() == 3 && (path.points[0].y - ungroup.center().y).abs() < 3. =>
            {
                Some(&path.points)
            }
            _ => None,
        })
        .collect();
    assert_eq!(arrows.len(), 2);
    assert!(arrows[0][1].y < arrows[0][0].y && arrows[1][1].y > arrows[1][0].y);
    assert!(ungroup.right() <= w.width && ungroup.bottom() < 410.);
}

#[test]
#[ignore = "release timing benchmark"]
fn scroll_layer_tree_with_ten_thousand_objects() {
    let mut w = List::new();
    w.frame(vec![], Modifiers::NONE);
    let mut times = Vec::with_capacity(120);
    for i in 0..120 {
        let mut scroll = egui::scroll_area::State::load(&w.ctx, w.scroll).unwrap();
        scroll.offset.y = (i * 1900) as f32;
        scroll.store(&w.ctx, w.scroll);
        let start = std::time::Instant::now();
        let out = w.frame(vec![], Modifiers::NONE);
        times.push(start.elapsed());
        assert!(out.shapes.len() < 250);
    }
    times.sort_unstable();
    println!(
        "10,000-object layer tree, 120 scroll updates: median {:?}, p95 {:?}",
        times[60], times[114]
    );
}

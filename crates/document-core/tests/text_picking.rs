use document_core::{
    typography::{TextStyle, Typeface},
    Document, Layer, Shape, Stencil,
};

fn lettering() -> TextStyle {
    TextStyle {
        content: "M     M\n\nM     M".into(),
        face: Typeface::HackRegular,
        size: 14.,
        ..TextStyle::default()
    }
}

#[test]
fn editable_text_owns_its_line_and_word_spaces_until_painted_artwork_covers_them() {
    let mut doc = Document::default();
    let id = lettering()
        .insert(&mut doc, 1, [14., 11.], [255; 4])
        .unwrap();
    let text = &doc.objects[0];
    let between_glyphs = text.local_to_world([0.5, 0.15], 0);
    let blank_line = text.local_to_world([0.25, 0.5], 0);
    for p in [between_glyphs, blank_line] {
        assert_eq!(doc.hit_test(p, 0), None);
        let (object, style) = doc.text_at(p, 0).unwrap();
        assert_eq!(object.id, id);
        assert!(std::ptr::eq(style, object.text.as_ref().unwrap()));
        assert_eq!(style, &lettering());
    }

    doc.layers.push(Layer {
        id: 2,
        name: "Overlay".into(),
        visible: true,
        locked: false,
    });
    let overlay = doc.add_shape(
        [blank_line[0] - 4., blank_line[1] - 4.],
        [8.; 2],
        [210; 4],
        Shape::Ellipse,
        2,
    );
    // Layer order takes precedence over the positions in the object array.
    doc.objects.swap(0, 1);
    assert_eq!(doc.hit_test(blank_line, 0), Some(overlay));
    assert!(doc.text_at(blank_line, 0).is_none());
    let clear_corner = [blank_line[0] - 3.8, blank_line[1] - 3.8];
    assert_eq!(doc.hit_test(clear_corner, 0), None);
    assert_eq!(doc.text_at(clear_corner, 0).unwrap().0.id, id);
    assert_eq!(doc.text_at(between_glyphs, 0).unwrap().0.id, id);
}

#[test]
fn text_picking_follows_the_top_editable_paint_at_each_frame() {
    let mut doc = Document::default();
    let back = lettering().insert(&mut doc, 1, [12.; 2], [255; 4]).unwrap();
    doc.layers.push(Layer {
        id: 2,
        name: "Top".into(),
        visible: true,
        locked: false,
    });
    let front = TextStyle {
        content: "A     B\n\nC     D".into(),
        ..lettering()
    }
    .insert(&mut doc, 2, [12.; 2], [180; 4])
    .unwrap();
    let p = doc.objects[1].local_to_world([0.5, 0.5], 0);
    let chosen = |doc: &Document, frame| doc.text_at(p, frame).unwrap().0.id;

    doc.objects[1]
        .animated
        .visible_keys
        .extend([(0, true), (1, false), (2, true)]);
    doc.objects[1]
        .opacity_keys
        .extend([(0, 1.), (2, 0.), (3, 1.)]);
    assert_eq!(chosen(&doc, 0), front);
    assert_eq!(chosen(&doc, 1), back);
    assert_eq!(chosen(&doc, 2), back);
    assert_eq!(chosen(&doc, 3), front);
    doc.layers[1].visible = false;
    assert_eq!(chosen(&doc, 3), back);
    doc.layers[1].visible = true;
    doc.layers[1].locked = true;
    assert_eq!(chosen(&doc, 3), back);
    doc.layers[1].locked = false;
    doc.objects[1].fill[3] = 0;
    assert_eq!(chosen(&doc, 3), back);
    doc.objects[1].stroke_width = 1.5;
    doc.objects[1].stroke = [190; 4];
    assert_eq!(chosen(&doc, 3), front);
}

#[test]
fn picks_respect_the_animated_transformed_layout_and_its_stencil() {
    let mut doc = Document::default();
    let id = lettering().insert(&mut doc, 1, [5., 7.], [255; 4]).unwrap();
    let object = &mut doc.objects[0];
    object.rotation = 37.;
    object.scale = [0.78, 1.26];
    object.animated.skew = 14.;
    object.keys.extend([(0, [5., 7.]), (4, [77., 91.])]);
    object.stencil = Some(Stencil {
        shape: Shape::Rectangle,
        origin: [0.; 2],
        size: [0.55, 1.],
        fill_rule: Default::default(),
        alpha: None,
    });
    let inside = object.local_to_world([0.24, 0.5], 4);
    let clipped = object.local_to_world([0.76, 0.5], 4);
    let outside = object.local_to_world([-0.2, 0.5], 4);
    assert_eq!(doc.hit_test(inside, 4), None);
    assert_eq!(doc.text_at(inside, 4).unwrap().0.id, id);
    assert!(doc.text_at(inside, 0).is_none());
    assert!(doc.text_at(clipped, 4).is_none());
    assert!(doc.text_at(outside, 4).is_none());
}

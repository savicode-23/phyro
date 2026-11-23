use super::*;
use document_core::{typography::TextStyle, SymbolDefinition};

#[test]
fn animated_ticks_share_authored_artwork_and_repaint_the_requested_pose() {
    let mut p = AnimationProject::default();
    let parent = p.clips[0].drawing_at_mut(0);
    TextStyle::default()
        .insert(parent, 1, [4., 4.], [255; 4])
        .unwrap();
    let child = p.add_clip("Walk").unwrap();
    p.clips[child].set_length(5).unwrap();
    let cel = p.clips[child].drawing_at_mut(0);
    cel.add_rectangle([4., 55.], [15., 9.], [180, 60, 30, 255]);
    cel.objects[0]
        .keys
        .extend([(0, [4., 55.]), (4, [84., 55.])]);
    p.place_clip(p.clips[child].id, 0, 0, 1, [0.; 2]).unwrap();
    let mut master = Document {
        width: 16,
        height: 16,
        frames: 1,
        ..Default::default()
    };
    master.add_rectangle([0.; 2], [16.; 2], [120, 70, 40, 255]);
    let symbol = p
        .add_symbol(SymbolDefinition {
            name: "Badge".into(),
            artwork: Arc::new(master.clone()),
        })
        .unwrap();
    p.place_symbol(symbol, 0, 0, 1, [98., 81.]).unwrap();
    let initial = p.clone();

    let mut canvas = Canvas::default();
    let ctx = egui::Context::default();
    let mut paint = |doc: &Document, frame, preview: Option<&Arc<Document>>| {
        let output = ctx.run(Default::default(), |ctx| {
            canvas
                .artwork_texture(ctx, doc, frame, 1., preview)
                .unwrap();
        });
        let cache = canvas.texture.as_ref().unwrap();
        assert_eq!(cache.frame, frame);
        let delta = &output
            .textures_delta
            .set
            .iter()
            .find(|(id, _)| *id == cache.handle.id())
            .expect("an animated tick must upload freshly rendered art")
            .1;
        assert!(delta.pos.is_none());
        let egui::ImageData::Color(image) = &delta.image else {
            panic!("expected a color image")
        };
        let source = preview.map_or(doc, Arc::as_ref);
        let expected = color_image(renderer::render(source, frame).unwrap());
        assert!(**image == expected, "wrong artwork at frame {frame}");
        assert_eq!(cache.document.as_ref(), source);
        Arc::clone(&cache.document)
    };
    let initial_pose = paint(p.clips[0].drawing_at(0), 0, None);
    let next_pose = paint(p.clips[0].drawing_at(1), 1, None);
    assert!(Arc::ptr_eq(&initial_pose, &next_pose));

    p.clips[child].drawing_at_mut(0).objects[0].fill = [20, 90, 160, 255];
    p.hydrate_composition().unwrap();
    let changed_child = paint(p.clips[0].drawing_at(2), 2, None);
    assert!(!Arc::ptr_eq(&next_pose, &changed_child));
    master.objects[0].fill = [30, 210, 90, 255];
    p.replace_symbol_artwork(symbol, master).unwrap();
    let changed_symbol = paint(p.clips[0].drawing_at(3), 3, None);
    assert!(!Arc::ptr_eq(&changed_child, &changed_symbol));
    TextStyle {
        content: "B".into(),
        ..Default::default()
    }
    .apply(&mut p.clips[0].drawing_at_mut(4).objects[0])
    .unwrap();
    let changed_text = paint(p.clips[0].drawing_at(4), 4, None);
    assert!(!Arc::ptr_eq(&changed_symbol, &changed_text));

    let doc = p.clips[0].drawing_at(0);
    let mut preview = doc.clone();
    preview.objects[0].position = [65., 20.];
    let preview = Arc::new(preview);
    assert!(Arc::ptr_eq(&preview, &paint(doc, 0, Some(&preview))));
    let cancelled = paint(doc, 1, None);
    assert!(!Arc::ptr_eq(&preview, &cancelled));
    let restored = paint(initial.clips[0].drawing_at(2), 2, None);
    assert_eq!(restored, initial_pose);
    assert!(!Arc::ptr_eq(&cancelled, &restored));
}

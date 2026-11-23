use document_core::Document;
use renderer::{render, render_scaled};

#[test]
fn preview_and_png_sample_the_same_animated_geometry() {
    let mut doc = Document::default();
    doc.add_rectangle([4.0, 3.0], [3.0, 2.0], [255, 0, 0, 128]);
    doc.objects[0].keys.insert(0, [4.0, 3.0]);
    doc.objects[0].keys.insert(4, [12.0, 3.0]);

    let native = render(&doc, 2).unwrap();
    let zoom = render_scaled(&doc, 2, 8.0).unwrap();
    let png = native.encode_png().unwrap();
    let reopened = renderer::Pixmap::decode_png(&png).unwrap();

    for (x, y, alpha) in [(4, 3, 0), (8, 3, 128), (10, 4, 128), (11, 4, 0)] {
        let pixel = native.pixel(x, y).unwrap();
        assert_eq!(pixel.alpha(), alpha);
        assert_eq!(pixel.red(), alpha);
        assert_eq!(zoom.pixel(x * 8 + 4, y * 8 + 4).unwrap(), pixel);
        assert_eq!(reopened.pixel(x, y).unwrap(), pixel);
    }
}

#[test]
fn fractional_edges_remain_visible_at_high_zoom() {
    let mut doc = Document {
        width: 4,
        height: 4,
        ..Document::default()
    };
    doc.add_rectangle([0.5, 0.0], [0.5, 3.0], [20, 160, 240, 255]);
    let native = render(&doc, 0).unwrap();
    let zoom = render_scaled(&doc, 0, 64.0).unwrap();

    assert!((1..255).contains(&native.pixel(0, 1).unwrap().alpha()));
    for (x, expected_alpha) in [(31, 0), (32, 255), (63, 255), (64, 0)] {
        assert_eq!(zoom.pixel(x, 64).unwrap().alpha(), expected_alpha);
    }
}

#[test]
fn later_objects_composite_over_earlier_objects_and_clip_at_artboard() {
    let mut doc = Document::default();
    doc.add_rectangle([-1.0, -1.0], [2.0, 2.0], [255, 0, 0, 255]);
    doc.add_rectangle([0.0, 0.0], [2.0, 2.0], [0, 0, 255, 255]);
    let image = render(&doc, 0).unwrap();
    assert_eq!(&image.data()[..4], &[0, 0, 255, 255]);
    assert_eq!(image.pixel(2, 2).unwrap().alpha(), 0);
}

#[test]
fn invalid_render_requests_return_before_allocating() {
    let doc = Document::default();
    for scale in [0.0, f32::NAN, f32::INFINITY, f32::MAX] {
        assert!(render_scaled(&doc, 0, scale).is_err());
    }
    assert!(render(&doc, doc.frames).is_err());
}

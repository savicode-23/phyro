//! Capability probe for candidate rendering backends. These tools are not yet
//! exposed in Phyro's document format. Run with `cargo test -p renderer`.

use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Mask, Paint, PathBuilder, Pixmap, Point, Rect,
    SpreadMode, Stroke, Transform,
};

#[test]
fn masked_gradient_and_antialiased_cubic_stroke_render_without_a_window() {
    let mut surface = Pixmap::new(80, 48).unwrap();
    let mut mask = Mask::new(80, 48).unwrap();
    let clip = PathBuilder::from_rect(Rect::from_xywh(8.0, 4.0, 64.0, 40.0).unwrap());
    mask.fill_path(&clip, FillRule::Winding, true, Transform::identity());

    let paint = Paint {
        shader: LinearGradient::new(
            Point::from_xy(0.0, 0.0),
            Point::from_xy(80.0, 0.0),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(255, 0, 0, 255)),
                GradientStop::new(1.0, Color::from_rgba8(0, 0, 255, 255)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap(),
        anti_alias: true,
        ..Paint::default()
    };
    let mut curve = PathBuilder::new();
    curve.move_to(0.0, 24.0);
    curve.cubic_to(24.0, 2.0, 56.0, 46.0, 80.0, 24.0);
    let stroke = Stroke {
        width: 4.0,
        ..Stroke::default()
    };
    surface.stroke_path(
        &curve.finish().unwrap(),
        &paint,
        &stroke,
        Transform::identity(),
        Some(&mask),
    );

    for x in [2, 77] {
        assert!((0..48).all(|y| surface.pixel(x, y).unwrap().alpha() == 0));
    }
    let strongest = |x| {
        (0..48)
            .map(|y| surface.pixel(x, y).unwrap())
            .max_by_key(|p| p.alpha())
            .unwrap()
    };
    assert!(strongest(16).red() > strongest(16).blue());
    assert!(strongest(64).blue() > strongest(64).red());
    let partial_coverage = |p: &tiny_skia::PremultipliedColorU8| (1..255).contains(&p.alpha());
    assert!(surface.pixels().iter().any(partial_coverage));
}

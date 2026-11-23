use document_core::{Shape, VectorObject};
use tiny_skia::{IntRect, Point, Rect, Transform};

/// Whether this object's ink stays inside the page, including edge coverage
/// on the given pixel grid. Unknown or very large extents use the regular painter.
pub fn wholly_on_artboard(
    object: &VectorObject,
    frame: u32,
    page_size: [u32; 2],
    scale: f32,
) -> bool {
    if page_size.contains(&0) || !scale.is_finite() || scale <= 0. || object.is_instance() {
        return false;
    }
    let [w, h] = page_size.map(|v| (f64::from(v) * f64::from(scale)).round().max(1.) as u32);
    if w.max(h) > 1 << 20 {
        return false;
    }
    let space = Transform::from_scale(
        w as f32 / page_size[0] as f32,
        h as f32 / page_size[1] as f32,
    );
    pixel_bounds(object, frame, space).is_some_and(|b| {
        b.left() > 0 && b.top() > 0 && b.right() < w as i32 && b.bottom() < h as i32
    })
}

/// Can either axis of the object's exact artboard bounds align inside the
/// supplied [left, top, right, bottom] region? An off-canvas object can still
/// supply an x or y guide even when its painted area misses the region.
pub fn bounds_may_align(o: &VectorObject, frame: u32, [l, t, r, b]: [f32; 4]) -> bool {
    let outside_both = |[x, y]: [f32; 2]| (x < l || x > r) && (y < t || y > b);
    if !outside_both(o.position)
        || o.is_instance()
        || o.skin.is_some()
        || !o.effects.is_empty()
        || !o.animated.geometry_keys.is_empty()
        // When an outline collapses, VectorObject::bounds uses the posed origin.
        || !outside_both(o.position_at(frame))
    {
        return true;
    }
    let has_stroke = !o.no_stroke && o.stroke_width_at(frame) > 0.;
    let Some(extent) = pixel_bounds_with_stroke(o, frame, Transform::identity(), has_stroke) else {
        return true;
    };
    // At very large coordinates f32 rounding can exceed the raster padding.
    if extent.left().min(extent.top()) < -(1 << 20) || extent.right().max(extent.bottom()) > 1 << 20
    {
        return true;
    }
    (extent.left() as f32 <= r && l <= extent.right() as f32)
        || (extent.top() as f32 <= b && t <= extent.bottom() as f32)
}

/// Conservative raster bounds, including stroke joins and a margin for edge coverage.
/// Unbounded symbols and numerical overflow fall back to the regular painter.
pub(super) fn pixel_bounds(o: &VectorObject, frame: u32, transform: Transform) -> Option<IntRect> {
    pixel_bounds_with_stroke(o, frame, transform, o.has_stroke_at(frame))
}

fn pixel_bounds_with_stroke(
    o: &VectorObject,
    frame: u32,
    transform: Transform,
    has_stroke: bool,
) -> Option<IntRect> {
    let pixels = if o.clip_instance.is_some() {
        let (art, _) = o.instance_at(frame)?;
        let viewport = Rect::from_xywh(0., 0., art.width as f32, art.height as f32)?;
        map_rect(viewport, o.symbol_transform(frame)?.post_concat(transform))?
    } else {
        if o.is_instance() {
            return None;
        }
        let world = if o.shape == Shape::Rectangle
            && !has_stroke
            && o.rotation_at(frame) == 0.
            && o.skew_at(frame) == 0.
            && transform.kx == 0.
            && transform.ky == 0.
            && transform.tx == 0.
            && transform.ty == 0.
            && transform.sx > 0.
            && transform.sy > 0.
        {
            // Match the legacy rectangle fast path, even for very large boxes.
            let ([x, y], [w, h]) = o.scaled_box_at(frame);
            Rect::from_ltrb(x, y, x + w, y + h)?
        } else {
            let (size, placement) = o.outline_transform_at(frame);
            o.evaluated_shape_at(frame)
                .control_bounds(size, placement)?
        };
        let stroked = if has_stroke {
            let style = o.stroke_style_at(frame);
            let padding = style.width * 0.5 * style.miter_limit.max(2.);
            world.outset(padding, padding)?
        } else {
            world
        };
        map_rect(stroked, transform)?
    };
    covered_pixels(pixels)
}

pub(super) fn union(a: IntRect, b: IntRect) -> IntRect {
    IntRect::from_ltrb(
        a.left().min(b.left()),
        a.top().min(b.top()),
        a.right().max(b.right()),
        a.bottom().max(b.bottom()),
    )
    .unwrap()
}

// The cache lives for one pass over one document. Every borrowed definition
// stays alive for that pass, even when different definitions share a numeric ID.
pub(super) type SymbolBounds =
    std::collections::HashMap<*const document_core::Document, Option<IntRect>>;

pub(super) fn symbol_bounds(
    symbols: &mut SymbolBounds,
    object: &VectorObject,
    frame: u32,
    transform: Transform,
) -> Option<IntRect> {
    let art = object.static_artwork()?.as_ref();
    let source = *symbols.entry(std::ptr::from_ref(art)).or_insert_with(|| {
        let mut extent = None;
        for child in art.paint_order() {
            if child.visible_at(0) && child.opacity_at(0) > 0. {
                let bounds = pixel_bounds(child, 0, Transform::identity())?;
                if bounds.left().min(bounds.top()) <= -(1 << 20)
                    || bounds.right().max(bounds.bottom()) >= 1 << 20
                {
                    return None;
                }
                extent = Some(extent.map_or(bounds, |prior| union(prior, bounds)));
            }
        }
        extent
    });
    symbol_pixels(object, frame, transform, source?)
}

fn symbol_pixels(
    o: &VectorObject,
    frame: u32,
    transform: Transform,
    source: IntRect,
) -> Option<IntRect> {
    let transform = o.symbol_transform(frame)?.post_concat(transform);
    // Large terms can cancel in a translated or sheared instance. Only use
    // the rounded master bounds while matrix roundoff fits the pixel margin.
    let extent_x = source
        .left()
        .unsigned_abs()
        .max(source.right().unsigned_abs()) as f32;
    let extent_y = source
        .top()
        .unsigned_abs()
        .max(source.bottom().unsigned_abs()) as f32;
    let error_scale = (extent_x * transform.sx.abs()
        + extent_y * transform.kx.abs()
        + transform.tx.abs())
    .max(extent_x * transform.ky.abs() + extent_y * transform.sy.abs() + transform.ty.abs());
    if !error_scale.is_finite() || error_scale > (1 << 20) as f32 {
        return None;
    }
    covered_pixels(map_rect(source.to_rect(), transform)?)
}

fn covered_pixels(pixels: Rect) -> Option<IntRect> {
    let pixels = pixels.outset(2., 2.)?;
    IntRect::from_ltrb(
        pixels.left().floor() as i32,
        pixels.top().floor() as i32,
        pixels.right().ceil() as i32,
        pixels.bottom().ceil() as i32,
    )
}

fn map_rect(rect: Rect, transform: Transform) -> Option<Rect> {
    if !transform.has_skew() && transform.sx > 0. && transform.sy > 0. {
        return Rect::from_ltrb(
            rect.left() * transform.sx + transform.tx,
            rect.top() * transform.sy + transform.ty,
            rect.right() * transform.sx + transform.tx,
            rect.bottom() * transform.sy + transform.ty,
        );
    }
    let mut corners = [
        Point::from_xy(rect.left(), rect.top()),
        Point::from_xy(rect.right(), rect.top()),
        Point::from_xy(rect.right(), rect.bottom()),
        Point::from_xy(rect.left(), rect.bottom()),
    ];
    transform.map_points(&mut corners);
    Rect::from_points(&corners)
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::{Document, LineCap, LineJoin, PathNode};
    use tiny_skia::Pixmap;

    #[test]
    fn containment_includes_off_box_handles_stroke_and_subpixel_page_edges() {
        let mut doc = Document::default();
        let mut nodes = [[0., 0.], [1., 1.]].map(PathNode::corner);
        nodes[0].outgoing = [-1.5, 0.5];
        doc.add_shape(
            [42., 26.],
            [12., 18.],
            [255; 4],
            Shape::Path {
                nodes: nodes.to_vec(),
                closed: false,
            },
            1,
        );
        let object = &mut doc.objects[0];
        object.stroke_width = 2.;
        for scale in [0.75, 1., 1.7] {
            assert!(wholly_on_artboard(object, 0, [80, 64], scale));
            let [w, h] = [80., 64.].map(|d| (d * scale).round() as u32);
            let mut surround = Pixmap::new(w + 16, h + 16).unwrap();
            let transform =
                Transform::from_scale(w as f32 / 80., h as f32 / 64.).post_translate(8., 8.);
            crate::draw_object(&mut surround, object, 0, transform, [8.; 2], 1.);
            assert!(surround.pixels().iter().any(|p| p.alpha() > 0));
            for (i, pixel) in surround.pixels().iter().enumerate() {
                let x = i as u32 % surround.width();
                let y = i as u32 / surround.width();
                if !(8..w + 8).contains(&x) || !(8..h + 8).contains(&y) {
                    assert_eq!(pixel.alpha(), 0);
                }
            }
        }
        object.keys.extend([(0, [42., 26.]), (1, [7., 26.])]);
        assert!(!wholly_on_artboard(object, 1, [80, 64], 1.));
        object.stroke_width = 52.;
        assert!(!wholly_on_artboard(object, 0, [80, 64], 1.));
        object.stroke_width = 0.;
        object.keys.clear();
        object.position = [42., 1.];
        assert!(!wholly_on_artboard(object, 0, [80, 64], 0.75));
    }

    #[test]
    fn snap_region_checks_each_axis_of_the_stroked_transformed_curve() {
        let mut d = Document::default();
        let mut nodes = [[0., 0.], [0.3, 0.1], [0.32, 0.11], [1., 1.]].map(PathNode::corner);
        nodes[1].incoming = [-3., 1.];
        nodes[2].outgoing = [2., -4.];
        d.add_shape(
            [460., 520.],
            [33., 18.],
            [255; 4],
            Shape::Path {
                nodes: nodes.to_vec(),
                closed: false,
            },
            1,
        );
        let o = d.objects.last_mut().unwrap();
        o.stroke = [255; 4];
        o.stroke_width = 14.;
        o.rotation = 47.;
        o.animated.skew = -19.;
        for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
            for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
                o.cap = cap;
                o.join = join;
                let [l, t, r, b] = o.bounds(0);
                for x in [l, (l + r) * 0.5, r] {
                    assert!(bounds_may_align(o, 0, [x, b + 100., x, b + 120.]));
                }
                for y in [t, (t + b) * 0.5, b] {
                    assert!(bounds_may_align(o, 0, [r + 100., y, r + 120., y]));
                }
                assert!(!bounds_may_align(o, 0, [0., 0., 100., 100.]));
            }
        }
        o.position = [1e8, 1e8];
        assert!(bounds_may_align(o, 0, [0., 0., 100., 100.]));
    }

    #[test]
    fn pixel_bounds_include_paint_extending_into_the_artboard() {
        let mut doc = Document::default();
        let mut nodes = vec![PathNode::corner([-0.7, 0.5]), PathNode::corner([2.2, 1.])];
        nodes[0].outgoing = [1.3, -1.8];
        nodes[1].incoming = [2.3, 2.5];
        doc.add_shape(
            [-13., 12.],
            [28., 13.],
            [0; 4],
            Shape::Path {
                nodes,
                closed: false,
            },
            1,
        );
        let obj = &mut doc.objects[0];
        obj.stroke = [253, 193, 17, 255];
        obj.stroke_width = 5.;
        obj.cap = LineCap::Square;
        obj.join = LineJoin::Miter;
        obj.rotation = 27.;
        obj.animated.skew = -13.;
        for t in [
            Transform::from_scale(1.5, 1.5),
            Transform::from_row(0.8, 0.45, -0.3, 1.1, 30., 6.),
            Transform::from_row(-1.5, -0.25, 0.4, 0.7, 70., 13.),
        ] {
            let obj = &doc.objects[0];
            let bounds = pixel_bounds(obj, 0, t).unwrap();
            let mut uncropped = Pixmap::new(110, 110).unwrap();
            crate::draw_object(&mut uncropped, obj, 0, t, [0.; 2], 1.);
            let mut count = 0;
            for (i, p) in uncropped.pixels().iter().enumerate() {
                if p.alpha() != 0 {
                    let x = (i % 110) as i32;
                    let y = (i / 110) as i32;
                    assert!((bounds.left()..bounds.right()).contains(&x));
                    assert!((bounds.top()..bounds.bottom()).contains(&y));
                    count += 1;
                }
            }
            assert!(count > 100);
        }
    }

    #[test]
    fn off_artboard_artwork_reenters_the_export_at_its_keyed_position() {
        let mut doc = Document::default();
        doc.add_shape([475., -250.], [31., 43.], [255; 4], Shape::Ellipse, 1);
        doc.objects[0]
            .keys
            .extend([(0, [475., -250.]), (2, [-3., 10.])]);
        let outside = crate::render_scaled(&doc, 0, 1.4).unwrap();
        assert!(outside.pixels().iter().all(|p| p.alpha() == 0));
        let actual = crate::render_scaled(&doc, 2, 1.4).unwrap();
        let transform = Transform::from_scale(
            actual.width() as f32 / doc.width as f32,
            actual.height() as f32 / doc.height as f32,
        );
        let mut expected = Pixmap::new(actual.width(), actual.height()).unwrap();
        crate::draw_object(&mut expected, &doc.objects[0], 2, transform, [0.; 2], 1.);
        assert!(expected.pixels().iter().any(|p| p.alpha() > 0));
        assert!(actual.data() == expected.data());
    }
}

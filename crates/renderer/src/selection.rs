use crate::{bounds::SymbolBounds, clip_groups, Pixmap, RenderPurpose, MAX_PIXELS};
use document_core::VectorObject;
use tiny_skia::{IntRect, Rect, Transform};

/// Frame one object in a small square inspector preview, including signed pasteboard coordinates.
/// The surface is bounded even when the artwork covers thousands of document pixels.
pub fn object_thumbnail(object: &VectorObject, frame: u32, edge: u32) -> Option<Pixmap> {
    let edge = edge.clamp(8, 128);
    let [left, top, right, bottom] = painted_extent(object, frame, RenderPurpose::Editor)?;
    let width = right - left;
    let height = bottom - top;
    let side = edge as f32;
    let scale = (side - 6.) / width.max(height);
    let origin = [
        (side - scale * width) * 0.5 - left * scale,
        (side - scale * height) * 0.5 - top * scale,
    ];
    render_object_viewport(object, frame, [edge, edge], scale, origin)
}

/// Paint an object into a bounded preview, using artboard coordinates for its geometry.
/// The origin is the output pixel position of artboard (0, 0).
pub fn render_object_viewport(
    object: &VectorObject,
    frame: u32,
    size: [u32; 2],
    scale: f32,
    origin: [f32; 2],
) -> Option<Pixmap> {
    if !scale.is_finite()
        || scale <= 0.
        || !origin.iter().all(|v| v.is_finite())
        || u64::from(size[0]) * u64::from(size[1]) > MAX_PIXELS
    {
        return None;
    }
    let transform = Transform::from_scale(scale, scale).post_translate(origin[0], origin[1]);
    let mut image = Pixmap::new(size[0], size[1])?;
    clip_groups::paint(
        &mut image,
        std::iter::once((0, object)),
        frame,
        transform,
        origin,
        None,
        RenderPurpose::Editor,
    )
    .ok()?;
    Some(image)
}

#[cfg(test)]
#[test]
fn thumbnail_frames_large_offpage_art_in_a_bounded_surface_and_uses_the_current_paint() {
    let mut drawing = document_core::Document::default();
    drawing.add_rectangle([-120_000., 40_000.], [60_000., 24_000.], [40, 120, 60, 255]);
    let object = &mut drawing.objects[0];
    object
        .animated
        .fill_keys
        .extend([(0, object.fill), (1, [120, 60, 40, 255])]);
    for (frame, color) in [(0, [40, 120, 60, 255]), (1, [120, 60, 40, 255])] {
        let thumb = object_thumbnail(object, frame, 48).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (48, 48));
        let center = thumb.pixel(24, 24).unwrap();
        assert_eq!(
            [center.red(), center.green(), center.blue(), center.alpha()],
            color
        );
        assert_eq!(thumb.pixel(0, 0).unwrap().alpha(), 0);
    }
}

/// Render visible-layer objects in their supplied paint order. The origin is the
/// first pixel of the tightly cropped result in the scaled artboard coordinate system.
pub fn render_artwork(
    objects: &[&VectorObject],
    layers: &[document_core::Layer],
    frame: u32,
    scale: impl Into<f64>,
) -> Result<(Pixmap, [i32; 2]), String> {
    render_artwork_pose(objects, layers, frame, scale)?
        .ok_or_else(|| "The selected artwork has no painted pixels at this frame.".into())
}

/// Render the tight painted pose, returning None for a transparent animation frame.
pub fn render_artwork_pose(
    objects: &[&VectorObject],
    layers: &[document_core::Layer],
    frame: u32,
    scale: impl Into<f64>,
) -> Result<Option<(Pixmap, [i32; 2])>, String> {
    let scale = scale.into();
    let Some([l, t, r, b]) = artwork_bounds(objects, frame, scale)? else {
        return Ok(None);
    };
    let rect = IntRect::from_ltrb(l, t, r, b)
        .filter(|r| u64::from(r.width()) * u64::from(r.height()) <= MAX_PIXELS)
        .ok_or("The selection exceeds the 16 megapixel image limit. Reduce its scale or extent.")?;
    // Keep a small raster margin around the final crop. The stroker can tessellate
    // an edge differently if its unmasked curve is clipped at the surface boundary.
    const BORDER: i32 = 8;
    let rect = IntRect::from_ltrb(
        rect.left().saturating_sub(BORDER),
        rect.top().saturating_sub(BORDER),
        rect.right().saturating_add(BORDER),
        rect.bottom().saturating_add(BORDER),
    )
    .filter(|r| u64::from(r.width()) * u64::from(r.height()) <= MAX_PIXELS)
    .unwrap_or(rect);
    let transform = Transform::from_scale(scale as f32, scale as f32)
        .post_translate(-(rect.left() as f32), -(rect.top() as f32));
    let mut image = selection_image(
        objects,
        layers,
        frame,
        [rect.width(), rect.height()],
        transform,
    )?;
    let Some(crop) = ink_bounds(&image) else {
        return Ok(None);
    };
    let origin = [rect.left() + crop.left(), rect.top() + crop.top()];
    if crop.width() != image.width() || crop.height() != image.height() {
        image = image
            .clone_rect(crop)
            .ok_or("Could not crop the selection image.")?;
    }
    Ok(Some((image, origin)))
}

/// Render an existing crop at its final integer dimensions, using native vector geometry.
pub fn render_artwork_in_viewport(
    objects: &[&VectorObject],
    layers: &[document_core::Layer],
    frame: u32,
    origin: [i32; 2],
    native: [u32; 2],
    output: [u32; 2],
) -> Result<Pixmap, String> {
    if native.contains(&0)
        || output.contains(&0)
        || u64::from(output[0]) * u64::from(output[1]) > MAX_PIXELS
    {
        return Err("Choose a non-empty selection image of at most 16 megapixels.".into());
    }
    let sx = output[0] as f32 / native[0] as f32;
    let sy = output[1] as f32 / native[1] as f32;
    let transform = Transform::from_scale(sx, sy)
        .post_translate(-(origin[0] as f32) * sx, -(origin[1] as f32) * sy);
    selection_image(objects, layers, frame, output, transform)
}

fn selection_image(
    objects: &[&VectorObject],
    layers: &[document_core::Layer],
    frame: u32,
    size: [u32; 2],
    transform: Transform,
) -> Result<Pixmap, String> {
    let mut image =
        Pixmap::new(size[0], size[1]).ok_or("Could not allocate the selection image.")?;
    clip_groups::paint_with_layers(
        &mut image,
        clip_groups::runs(objects.iter().copied(), frame),
        layers,
        (frame, transform, [transform.tx, transform.ty]),
        Some(&mut SymbolBounds::new()),
        RenderPurpose::Export,
    )?;
    Ok(image)
}

/// Integer viewport enclosing the vector paint, including strokes, masks and
/// placed artwork. Computing these bounds does not allocate a raster surface.
pub fn artwork_bounds(
    objects: &[&VectorObject],
    frame: u32,
    scale: impl Into<f64>,
) -> Result<Option<[i32; 4]>, String> {
    let scale = scale.into();
    if !(scale > 0. && scale <= 32.) {
        return Err("Choose a positive export scale up to 32×.".into());
    }
    let Some(edges) = objects
        .iter()
        .filter_map(|o| painted_extent(o, frame, RenderPurpose::Export))
        .reduce(union_edges)
    else {
        return Ok(None);
    };
    let mut extent = edges.map(|n| f64::from(n) * scale);
    extent[0] = extent[0].floor();
    extent[1] = extent[1].floor();
    extent[2] = extent[2].ceil();
    extent[3] = extent[3].ceil();
    if extent
        .iter()
        .any(|&n| !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&n))
    {
        return Err("The selection extends beyond the supported output coordinates.".into());
    }
    Ok(Some(extent.map(|n| n as i32)))
}

fn union_edges(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn painted_extent(object: &VectorObject, frame: u32, purpose: RenderPurpose) -> Option<[f32; 4]> {
    if (object.reference_only && purpose == RenderPurpose::Export)
        || !object.visible_at(frame)
        || object.opacity_at(frame) <= 0.
    {
        return None;
    }
    let [l, t, r, b] = if let Some((source, tick)) = object.instance_at(frame) {
        // A symbol can paint past its own artboard. Nested clips retain their
        // viewport, and invisible or masked children should not enlarge the crop.
        let [l, t, r, b] = source
            .paint_order()
            .into_iter()
            .filter_map(|o| painted_extent(o, tick, purpose))
            .reduce(union_edges)?;
        let mut rect = Rect::from_ltrb(l, t, r, b)?;
        if object.clip_instance.is_some() {
            rect = rect.intersect(&Rect::from_xywh(
                0.,
                0.,
                source.width as f32,
                source.height as f32,
            )?)?;
        }
        let rect = rect.transform(object.symbol_transform(frame)?)?;
        [rect.left(), rect.top(), rect.right(), rect.bottom()]
    } else {
        object.painted_shape_bounds(frame)?
    };
    let mut rect = Rect::from_ltrb(l, t, r, b)?;
    if let Some(stencil) = &object.stencil {
        if stencil.alpha == Some([0, 0]) {
            return None;
        }
        let path = stencil.outline(object.paint_space(frame)?)?;
        let bounds = path.compute_tight_bounds().unwrap_or_else(|| path.bounds());
        rect = rect.intersect(&bounds)?;
    }
    (rect.width() > 0. && rect.height() > 0.).then_some([
        rect.left(),
        rect.top(),
        rect.right(),
        rect.bottom(),
    ])
}

/// The smallest rectangle containing every nontransparent pixel of an image.
pub fn ink_bounds(image: &Pixmap) -> Option<IntRect> {
    let mut left = image.width() as usize;
    let mut right = 0;
    let mut top = image.height() as usize;
    let mut bottom = 0;
    for (y, row) in image
        .pixels()
        .chunks_exact(image.width() as usize)
        .enumerate()
    {
        if let Some(first) = row.iter().position(|p| p.alpha() != 0) {
            left = left.min(first);
            right = right.max(row.iter().rposition(|p| p.alpha() != 0).unwrap() + 1);
            top = top.min(y);
            bottom = y + 1;
        }
    }
    IntRect::from_ltrb(left as i32, top as i32, right as i32, bottom as i32)
}

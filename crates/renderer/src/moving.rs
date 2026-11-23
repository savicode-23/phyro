use crate::{bounds::SymbolBounds, clip_groups, Pixmap, RenderPurpose};
use document_core::VectorObject;
use tiny_skia::{IntRect, Transform};

/// A paused placement rasterized in its current orientation for an interactive translation.
/// Signed world coordinates keep the same preview usable on the page and pasteboard.
pub struct MoveSprite {
    pub pixels: Pixmap,
    pub origin: [f32; 2],
    pub scale: f32,
    pub requested_scale: f32,
}

pub const MAX_SPRITE_EDGE: u32 = 2048;

pub fn move_sprite(object: &VectorObject, frame: u32, scale: f32) -> Option<MoveSprite> {
    if !(object.is_instance() || object.image.is_some()) || object.clip_source.is_some() {
        return None;
    }
    selection_run(&[object], frame, scale, MAX_SPRITE_EDGE)
}

/// Render a static span above a move in its original compositing order. Moving
/// behind translucent objects or across the page edge then costs a quad, too.
pub fn foreground_sprite(objects: &[&VectorObject], frame: u32, scale: f32) -> Option<MoveSprite> {
    foreground_in_region(objects, frame, scale, None)
        .ok()
        .flatten()
}

/// Cache the visible part of a static compositing run, in signed artwork
/// coordinates. Distant pasteboard objects need not enlarge its texture.
pub fn foreground_in_region(
    objects: &[&VectorObject],
    frame: u32,
    scale: f32,
    region: Option<[f32; 4]>,
) -> Result<Option<MoveSprite>, &'static str> {
    let mut symbols = SymbolBounds::new();
    let mut bounds =
        sprite_bounds(objects, frame, scale, &mut symbols).ok_or("Unknown foreground extent")?;
    if let Some([l, t, r, b]) = region {
        let space = IntRect::from_ltrb(
            (l * scale).floor() as i32,
            (t * scale).floor() as i32,
            (r * scale).ceil() as i32,
            (b * scale).ceil() as i32,
        )
        .ok_or("Invalid viewport")?;
        const MARGIN: i32 = 4;
        let padded = IntRect::from_ltrb(
            space.left().saturating_sub(MARGIN),
            space.top().saturating_sub(MARGIN),
            space.right().saturating_add(MARGIN),
            space.bottom().saturating_add(MARGIN),
        )
        .ok_or("Invalid viewport")?;
        let Some(visible) = bounds.intersect(&padded) else {
            return Ok(None);
        };
        bounds = visible;
    }
    rasterize_sprite(objects, frame, scale, scale, bounds, &mut symbols)
        .map(Some)
        .ok_or("Foreground exceeds the preview budget")
}

/// Rasterize a selected paint-order run once for an interactive transform.
/// Its full extent can cross both the artboard and the current viewport.
pub fn selection_run(
    objects: &[&VectorObject],
    frame: u32,
    scale: f32,
    max_edge: u32,
) -> Option<MoveSprite> {
    let mut symbols = SymbolBounds::new();
    let mut density = scale;
    let mut bounds = sprite_bounds(objects, frame, scale, &mut symbols)?;
    let edge = bounds.width().max(bounds.height());
    let max_edge = max_edge.clamp(16, MAX_SPRITE_EDGE);
    if edge > max_edge {
        density *= (max_edge - 8) as f32 / edge as f32;
        bounds = sprite_bounds(objects, frame, density, &mut symbols)?;
    }
    rasterize_sprite(objects, frame, density, scale, bounds, &mut symbols)
}

fn sprite_bounds(
    objects: &[&VectorObject],
    frame: u32,
    scale: f32,
    symbols: &mut SymbolBounds,
) -> Option<IntRect> {
    if !scale.is_finite() || scale <= 0. {
        return None;
    }
    let space = Transform::from_scale(scale, scale);
    let mut extent: Option<IntRect> = None;
    for &object in objects {
        if !object.visible_at(frame) || object.opacity_at(frame) <= 0. {
            continue;
        }
        let next = painted_bounds(object, frame, space, symbols)?;
        extent = Some(match extent {
            Some(b) => IntRect::from_ltrb(
                b.left().min(next.left()),
                b.top().min(next.top()),
                b.right().max(next.right()),
                b.bottom().max(next.bottom()),
            )?,
            None => next,
        });
    }
    extent
}

fn painted_bounds(
    object: &VectorObject,
    frame: u32,
    space: Transform,
    symbols: &mut SymbolBounds,
) -> Option<IntRect> {
    if object.static_artwork().is_some() {
        return crate::bounds::symbol_bounds(symbols, object, frame, space);
    }
    let bounds = crate::bounds::pixel_bounds(object, frame, space)?;
    let Some(clip) = &object.clip_instance else {
        return Some(bounds);
    };
    // An imported 128-pixel pose can live in a 1000-pixel child drawing.
    // Sample the current pose, including ink beyond its original artboard,
    // without reserving a texture for all the transparent space around it.
    let (drawing, tick) = clip.sample(frame)?;
    let child_space = object.symbol_transform(frame)?.post_concat(space);
    let mut ink = None;
    for child in drawing.paint_order() {
        if child.visible_at(tick) && child.opacity_at(tick) > 0. {
            let Some(extent) = painted_bounds(child, tick, child_space, symbols) else {
                // Unknown ink still cannot escape the outer viewport.
                return Some(bounds);
            };
            if let Some(visible) = bounds.intersect(&extent) {
                ink = Some(ink.map_or(visible, |old| crate::bounds::union(old, visible)));
                if ink == Some(bounds) {
                    break;
                }
            }
        }
    }
    // Keep a drawable transparent sample for an empty pose.
    ink.or_else(|| IntRect::from_xywh(bounds.left(), bounds.top(), 1, 1))
}

fn rasterize_sprite(
    objects: &[&VectorObject],
    frame: u32,
    scale: f32,
    requested_scale: f32,
    bounds: IntRect,
    symbols: &mut SymbolBounds,
) -> Option<MoveSprite> {
    let size = [bounds.width(), bounds.height()];
    if size[0].max(size[1]) > MAX_SPRITE_EDGE {
        return None;
    }
    let x = bounds.left() as f32;
    let y = bounds.top() as f32;
    let mut pixels = Pixmap::new(size[0], size[1])?;
    clip_groups::paint(
        &mut pixels,
        clip_groups::runs(objects.iter().copied(), frame),
        frame,
        Transform::from_scale(scale, scale).post_translate(-x, -y),
        [-x, -y],
        Some(symbols),
        RenderPurpose::Editor,
    )
    .ok()?;
    Some(MoveSprite {
        pixels,
        origin: [x / scale, y / scale],
        scale,
        requested_scale,
    })
}

#[cfg(test)]
#[test]
fn cropped_foreground_keeps_shared_mask_pixels_across_the_viewport_edges() {
    let mut drawing = document_core::Document::default();
    let first = drawing.add_rectangle([-65., 2.], [156., 78.], [29, 207, 120, 173]);
    let second = drawing.add_rectangle([-25., 7.], [113., 76.], [168, 80, 157, 210]);
    for object in &mut drawing.objects {
        object.group = Some(9);
    }
    let mask = drawing.add_rectangle([30., 0.], [30., 90.], [255; 4]);
    drawing.objects[2].rotation = 8.;
    drawing
        .set_clip(&[first, second, mask], 0, Some([75, 205]))
        .unwrap();
    drawing.add_rectangle([9000., 0.], [10., 20.], [255; 4]);
    let objects = drawing.paint_order();
    let small = foreground_in_region(&objects, 0, 1., Some([33., 20., 50., 60.]))
        .unwrap()
        .unwrap();
    let page = crate::render(&drawing, 0).unwrap();
    assert!(small.pixels.width() < 40);
    for y in 20..60 {
        for x in 33..50 {
            assert_eq!(
                small.pixels.pixel(
                    (x as i32 - small.origin[0] as i32) as u32,
                    (y as i32 - small.origin[1] as i32) as u32
                ),
                page.pixel(x, y)
            );
        }
    }
    assert!(
        foreground_in_region(&objects, 0, 1., Some([10_000., 0., 10_050., 50.]))
            .unwrap()
            .is_none()
    );
}

#[cfg(test)]
#[test]
fn translating_a_symbol_through_foreground_and_page_edge_preserves_paint_order() {
    use document_core::Document;
    use tiny_skia::PixmapPaint;
    let mut master = Document::default();
    master.add_rectangle([-8., 0.], [34., 24.], [20, 200, 150, 255]);
    let mut lower = Document::default();
    lower.add_rectangle([0., 0.], [96., 96.], [44, 40, 77, 255]);
    let mut scene = lower.clone();
    scene.add_rectangle([3., 8.], [128., 128.], [0; 4]);
    scene.objects[1].embedded_artwork = Some(std::sync::Arc::new(master).into());
    scene.add_rectangle([12., 16.], [22., 42.], [220, 70, 30, 144]);
    let moving = move_sprite(&scene.objects[1], 0, 1.).unwrap();
    let above = foreground_sprite(&[&scene.objects[2]], 0, 1.).unwrap();
    assert!(moving.origin[0] < 0.);
    for dx in [0, 42, -18] {
        let mut layered = crate::render(&lower, 0).unwrap();
        for (sprite, shift) in [(&moving, dx), (&above, 0)] {
            layered.draw_pixmap(
                sprite.origin[0] as i32 + shift,
                sprite.origin[1] as i32,
                sprite.pixels.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
        scene.objects[1].position[0] = 3. + dx as f32;
        let exact = crate::render(&scene, 0).unwrap();
        // The extra intermediate premultiplication can round a channel by one.
        assert!(
            layered
                .data()
                .iter()
                .zip(exact.data())
                .all(|(a, b)| a.abs_diff(*b) <= 1),
            "preview differed at offset {dx}"
        );
    }
}

#[cfg(test)]
#[test]
fn imported_clip_preview_covers_the_sampled_ink_without_its_expanded_empty_page() {
    use document_core::{AnimationProject, Document, Shape, Stencil};
    let mut project = AnimationProject::from_document(Document {
        width: 1000,
        height: 1000,
        ..Default::default()
    });
    let child = project.add_clip("Character").unwrap();
    let nested = project.add_clip("Arm").unwrap();
    let art = project.clips[nested].drawing_at_mut(0);
    art.add_rectangle([-20., 5.], [40., 30.], [60, 100, 200, 190]);
    art.objects[0].stencil = Some(Stencil {
        shape: Shape::Rectangle,
        origin: [0., 0.],
        size: [1., 1.],
        fill_rule: Default::default(),
        alpha: Some([100, 255]),
    });
    // Artwork authored beyond the original import size is visible in the enlarged child.
    project.clips[child].drawing_at_mut(0).add_rectangle(
        [210., 50.],
        [15., 20.],
        [180, 70, 30, 220],
    );
    project
        .place_clip(project.clips[nested].id, child, 0, 1, [12., 8.])
        .unwrap();
    project
        .place_clip(project.clips[child].id, 0, 0, 1, [365., 220.])
        .unwrap();
    let doc = project.clips[0].drawing_at_mut(0);
    for (density, flip, rotation, shear) in [(1., 1., 0., 0.), (1.5, -1., 17., 9.)] {
        let pose = &mut doc.objects[0];
        pose.size = [128.; 2];
        pose.clip_instance.as_mut().unwrap().source_size = Some([128; 2]);
        pose.scale = [flip, 1.];
        pose.rotation = rotation;
        pose.animated.skew = shear;
        let preview = move_sprite(pose, 0, density).unwrap();
        assert_eq!(preview.scale, density);
        assert!(preview.pixels.width() < 350 && preview.pixels.height() < 220);
        let exact = crate::render_for_export(doc, 0, density).unwrap();
        let [x, y] = preview.origin.map(|v| (v * density).round() as i32);
        let mut placed = Pixmap::new(exact.width(), exact.height()).unwrap();
        placed.draw_pixmap(
            x,
            y,
            preview.pixels.as_ref(),
            &Default::default(),
            Transform::identity(),
            None,
        );
        assert!(exact.pixels().iter().filter(|p| p.alpha() != 0).count() > 800);
        let max_error = placed
            .data()
            .iter()
            .zip(exact.data())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        // Translating a rotated path onto the crop grid can round one of
        // tiny-skia's antialias coverage samples differently.
        let tolerance = if rotation == 0. { 0 } else { 16 };
        assert!(
            max_error <= tolerance,
            "color difference {max_error} at {rotation} degrees"
        );
    }
}

#[cfg(test)]
#[test]
fn oversized_placement_preview_fits_the_cache_and_keeps_its_world_extent() {
    let mut master = document_core::Document::default();
    master.add_rectangle([0., 0.], [128., 128.], [255; 4]);
    let mut scene = document_core::Document::default();
    scene.add_rectangle([-32., 5.], [4096., 256.], [0; 4]);
    scene.objects[0].embedded_artwork = Some(std::sync::Arc::new(master).into());
    let wide = &scene.objects[0];
    let preview = move_sprite(wide, 0, 3.).unwrap();
    assert_eq!(preview.requested_scale, 3.);
    assert!(preview.scale < preview.requested_scale);
    assert!(preview.pixels.width() <= MAX_SPRITE_EDGE);
    assert!(preview.origin[0] < -32.);
    assert!(preview.origin[0] + preview.pixels.width() as f32 / preview.scale > 4064.);
    assert!(
        preview
            .pixels
            .pixel(preview.pixels.width() / 2, preview.pixels.height() / 2)
            .unwrap()
            .alpha()
            > 0
    );
    assert!(foreground_sprite(&[wide], 0, 3.).is_none());
}

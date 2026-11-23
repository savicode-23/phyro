//! Shared CPU rasterization for the editor texture and headless PNG export.
//! Pixels in the returned buffer are premultiplied RGBA, as expected by egui.

use document_core::{Document, Shape, VectorObject};
pub use tiny_skia::Pixmap;
use tiny_skia::{FillRule, FilterQuality, IntRect, Paint, Pattern, Rect, SpreadMode, Transform};
mod appearance;
mod bounds;
mod clip_groups;
mod moving;
pub use moving::{
    foreground_in_region, foreground_sprite, move_sprite, selection_run, MoveSprite,
    MAX_SPRITE_EDGE,
};
pub mod pasteboard;
pub use bounds::{bounds_may_align, wholly_on_artboard};
mod preview;
pub use preview::{edit_patch, render_editable, DragPreview, EditBackdrop, EditPatch};
mod selection;
pub use selection::{
    artwork_bounds, ink_bounds, object_thumbnail, render_artwork, render_artwork_in_viewport,
    render_artwork_pose, render_object_viewport,
};
mod symbol;
#[cfg(test)]
mod viewport_tests;

use document_core::MAX_RASTER_PIXELS as MAX_PIXELS;

#[derive(Clone, Copy, PartialEq)]
enum RenderPurpose {
    Editor,
    Export,
}

/// Render the artboard at the project's native pixel dimensions.
pub fn render(document: &Document, frame: u32) -> Result<Pixmap, String> {
    render_scaled(document, frame, 1.0)
}

/// Rasterize evaluated vector geometry directly at the requested resolution.
pub fn render_scaled(document: &Document, frame: u32, scale: f32) -> Result<Pixmap, String> {
    render_page(document, frame, scale.into(), RenderPurpose::Editor)
}

/// Render one layer at the same bounds and pixel grid as the editor's page.
pub fn render_layer_scaled(
    document: &Document,
    layer: u64,
    frame: u32,
    scale: f32,
) -> Result<Pixmap, String> {
    let page = artboard(document, frame, scale)?;
    let mut pixmap = page.allocate()?;
    let objects = document.objects.iter().filter(|o| o.layer == layer);
    clip_groups::paint(
        &mut pixmap,
        clip_groups::runs(objects, frame),
        frame,
        page.transform,
        [0.; 2],
        Some(&mut bounds::SymbolBounds::new()),
        RenderPurpose::Editor,
    )?;
    Ok(pixmap)
}

/// Export the artboard without drawing references, including references inside placements.
pub fn render_for_export(
    document: &Document,
    frame: u32,
    scale: impl Into<f64>,
) -> Result<Pixmap, String> {
    render_page(document, frame, scale.into(), RenderPurpose::Export)
}

fn render_page(
    document: &Document,
    frame: u32,
    scale: f64,
    purpose: RenderPurpose,
) -> Result<Pixmap, String> {
    let page = artboard(document, frame, scale)?;
    let mut pixmap = page.allocate()?;
    clip_groups::paint(
        &mut pixmap,
        clip_groups::runs(document.paint_order().into_iter(), frame),
        frame,
        page.transform,
        [0.; 2],
        Some(&mut bounds::SymbolBounds::new()),
        purpose,
    )?;
    Ok(pixmap)
}

/// Sample a signed artwork coordinate, including the pasteboard, on the given pixel grid.
/// This uses the normal compositing path and returns premultiplied RGBA.
pub fn sample_artwork(
    doc: &Document,
    frame: u32,
    point: [f32; 2],
    scale: [f32; 2],
) -> Result<[u8; 4], String> {
    if frame >= doc.frames {
        return Err("Invalid artwork sample location.".into());
    }
    sample_objects(
        clip_groups::runs(doc.paint_order().into_iter(), frame),
        frame,
        point,
        scale,
    )
}

fn sample_objects<'a>(
    objects: impl IntoIterator<Item = (usize, &'a VectorObject)>,
    frame: u32,
    point: [f32; 2],
    scale: [f32; 2],
) -> Result<[u8; 4], String> {
    if !point.iter().all(|v| v.is_finite()) || !scale.iter().all(|v| v.is_finite() && *v > 0.) {
        return Err("Invalid artwork sample location.".into());
    }
    let mut objects = objects.into_iter().peekable();
    if objects.peek().is_none() {
        return Ok([0; 4]);
    }
    // Give rasterized curves a margin around the sample; large document
    // dimensions never require allocating or rendering a full page on a click.
    const MIDDLE: u32 = 8;
    let origin = [0, 1].map(|i| MIDDLE as f32 - (point[i] * scale[i]).floor());
    let transform = Transform::from_scale(scale[0], scale[1]).post_translate(origin[0], origin[1]);
    let mut surface = Pixmap::new(MIDDLE * 2 + 1, MIDDLE * 2 + 1)
        .ok_or("Could not allocate the artwork sample.")?;
    let mut symbols = bounds::SymbolBounds::new();
    clip_groups::paint(
        &mut surface,
        objects,
        frame,
        transform,
        origin,
        Some(&mut symbols),
        RenderPurpose::Editor,
    )?;
    let pixel = surface.pixel(MIDDLE, MIDDLE).unwrap();
    Ok([pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()])
}

struct Artboard {
    bounds: IntRect,
    transform: Transform,
}

impl Artboard {
    fn allocate(&self) -> Result<Pixmap, String> {
        Pixmap::new(self.bounds.width(), self.bounds.height())
            .ok_or_else(|| "Could not allocate the rendered image.".into())
    }
}

fn artboard(document: &Document, frame: u32, scale: impl Into<f64>) -> Result<Artboard, String> {
    let scale = scale.into();
    if frame >= document.frames {
        return Err("The requested frame is outside the animation.".into());
    }
    if !scale.is_finite() || scale <= 0.0 || document.width == 0 || document.height == 0 {
        return Err("Rendering requires a positive artboard size and scale.".into());
    }
    let scaled = |value| (f64::from(value) * scale).round().max(1.0) as u32;
    let width = scaled(document.width);
    let height = scaled(document.height);
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("The rendered image exceeds 16 megapixels. Reduce its size or zoom.".into());
    }

    let bounds = IntRect::from_xywh(0, 0, width, height).ok_or("Invalid rendered dimensions.")?;
    let transform = Transform::from_scale(
        width as f32 / document.width as f32,
        height as f32 / document.height as f32,
    );
    Ok(Artboard { bounds, transform })
}

fn paint_objects(
    target: &mut Pixmap,
    objects: &[&VectorObject],
    frame: u32,
    transform: Transform,
) -> Result<(), String> {
    let mut symbols = bounds::SymbolBounds::new();
    clip_groups::paint(
        target,
        clip_groups::runs(objects.iter().copied(), frame),
        frame,
        transform,
        [0.; 2],
        Some(&mut symbols),
        RenderPurpose::Editor,
    )
}

#[cfg(test)]
fn draw_with_appearance(
    surface: &mut Pixmap,
    object: &VectorObject,
    frame: u32,
    transform: Transform,
    page_origin: [f32; 2],
    layer: &mut Option<Pixmap>,
    symbols: Option<&mut bounds::SymbolBounds>,
) -> Result<(), String> {
    draw_member(
        surface,
        object,
        (frame, transform, page_origin),
        layer,
        symbols,
        true,
        RenderPurpose::Editor,
    )
}

fn draw_member(
    surface: &mut Pixmap,
    object: &VectorObject,
    (frame, transform, page_origin): (u32, Transform, [f32; 2]),
    layer: &mut Option<Pixmap>,
    symbols: Option<&mut bounds::SymbolBounds>,
    apply_stencil: bool,
    purpose: RenderPurpose,
) -> Result<(), String> {
    let opacity = object.opacity_at(frame);
    if !object.visible_at(frame) || opacity <= 0. {
        return Ok(());
    }
    // Most visible shapes have their base position on the surface. Only do the
    // extra bounds work when that inexpensive hint suggests they may be outside.
    let mut origin = tiny_skia::Point::from_xy(object.position[0], object.position[1]);
    transform.map_point(&mut origin);
    if origin.x < 0.
        || origin.y < 0.
        || origin.x >= surface.width() as f32
        || origin.y >= surface.height() as f32
    {
        // The page may contain thousands of placements of the same master.
        // A symbol may paint beyond its placement box, so use the master's
        // actual painted extent. Tile plans already made this decision.
        let extent = if object.static_artwork().is_some() {
            symbols.and_then(|cache| bounds::symbol_bounds(cache, object, frame, transform))
        } else {
            bounds::pixel_bounds(object, frame, transform)
        };
        if extent.is_some_and(|b| {
            b.right() <= 0
                || b.bottom() <= 0
                || b.left() >= surface.width() as i32
                || b.top() >= surface.height() as i32
        }) {
            return Ok(());
        }
    }
    let multiple_paints =
        object.is_instance() || object.has_fill_at(frame) && object.has_stroke_at(frame);
    let draw = |target: &mut Pixmap, opacity: f32| -> Result<(), String> {
        if object.is_instance() {
            symbol::draw_symbol_content(target, object, frame, transform, page_origin, purpose)
        } else {
            draw_object(target, object, frame, transform, page_origin, opacity);
            Ok(())
        }
    };
    if (!apply_stencil || object.stencil.is_none())
        && object.clip_instance.is_none()
        && (opacity == 1. || !multiple_paints)
    {
        draw(surface, opacity)?;
        return Ok(());
    }
    let full = IntRect::from_xywh(0, 0, surface.width(), surface.height())
        .expect("a rendered surface has positive dimensions");
    let mut viewport_mask = None;
    let area = if let Some(clip) = &object.clip_instance {
        let (art, _) = clip.sample(frame).ok_or("Missing child clip.")?;
        let local = object
            .symbol_transform(frame)
            .ok_or("Missing clip transform.")?
            .post_concat(transform);
        let Some(path) = tiny_skia::PathBuilder::from_rect(
            Rect::from_xywh(0., 0., art.width as f32, art.height as f32)
                .ok_or("Invalid child viewport.")?,
        )
        .transform(local) else {
            return Ok(());
        };
        let b = path.bounds();
        // For an in-bounds axis-aligned viewport on integer pixel edges,
        // compositing its rectangle supplies the exact same coverage as a mask.
        let pixel_aligned = local.kx == 0.
            && local.ky == 0.
            && b.left() >= 0.
            && b.top() >= 0.
            && b.right() <= full.right() as f32
            && b.bottom() <= full.bottom() as f32
            && [b.left(), b.top(), b.right(), b.bottom()]
                .into_iter()
                .all(|edge| edge.fract() == 0.);
        let margin = if pixel_aligned { 0. } else { 1. };
        let Some(area) = IntRect::from_ltrb(
            (b.left().floor() - margin).max(0.) as i32,
            (b.top().floor() - margin).max(0.) as i32,
            (b.right().ceil() + margin).min(full.right() as f32) as i32,
            (b.bottom().ceil() + margin).min(full.bottom() as f32) as i32,
        ) else {
            return Ok(());
        };
        if !pixel_aligned {
            viewport_mask = Some(path);
        }
        area
    } else if object.is_instance() {
        full
    } else {
        let Some(area) = bounds::pixel_bounds(object, frame, transform)
            .unwrap_or(full)
            .intersect(&full)
        else {
            return Ok(());
        };
        area
    };
    if layer.is_none() {
        *layer = Some(
            Pixmap::new(surface.width(), surface.height())
                .ok_or("Could not allocate an object compositing layer.")?,
        );
    }
    let layer = layer.as_mut().expect("allocated above");
    clear_region(layer, area);
    draw(layer, 1.)?;
    let mut mask = if apply_stencil {
        appearance::mask(surface, object, frame, transform)?
    } else {
        None
    };
    if let Some(path) = viewport_mask {
        if let Some(existing) = &mut mask {
            existing.intersect_path(&path, FillRule::Winding, true, Transform::identity());
        } else {
            let mut viewport = tiny_skia::Mask::new(surface.width(), surface.height())
                .ok_or("Could not allocate the nested clip mask.")?;
            viewport.fill_path(&path, FillRule::Winding, true, Transform::identity());
            mask = Some(viewport);
        }
    }
    let paint = Paint {
        shader: Pattern::new(
            layer.as_ref(),
            SpreadMode::Pad,
            FilterQuality::Nearest,
            opacity,
            Transform::identity(),
        ),
        anti_alias: false,
        ..Default::default()
    };
    surface.fill_rect(area.to_rect(), &paint, Transform::identity(), mask.as_ref());
    Ok(())
}

// The blend reads this rectangle in surface coordinates. Contents outside the
// rectangle may be left by an earlier object and are never sampled here.
fn clear_region(layer: &mut Pixmap, area: IntRect) {
    let stride = layer.width() as usize * 4;
    let rows = &mut layer.data_mut()[area.top() as usize * stride..area.bottom() as usize * stride];
    if area.width() as usize * 4 == stride {
        rows.fill(0);
    } else {
        let columns = area.left() as usize * 4..area.right() as usize * 4;
        for row in rows.chunks_exact_mut(stride) {
            row[columns.clone()].fill(0);
        }
    }
}

fn draw_object(
    pixmap: &mut Pixmap,
    object: &VectorObject,
    frame: u32,
    transform: Transform,
    page_origin: [f32; 2],
    opacity: f32,
) {
    let has_stroke = object.has_stroke_at(frame);
    let mut paints = appearance::ObjectPaint::new(object, frame, opacity, transform, pixmap);
    if object.shape == Shape::Rectangle
        && object.rotation_at(frame) == 0.0
        && object.skew_at(frame) == 0.0
        && !has_stroke
        && transform.kx == 0.
        && transform.ky == 0.
        && [transform.tx, transform.ty] == page_origin
        && transform.sx > 0.
        && transform.sy > 0.
    {
        // Clip before scaling. A legacy rectangle can extend far beyond the
        // artboard and still cover some or all of its pixels.
        let ([x, y], [w, h]) = object.scaled_box_at(frame);
        if let Some(rect) = Rect::from_ltrb(
            x.max(-transform.tx / transform.sx),
            y.max(-transform.ty / transform.sy),
            (x + w).min((pixmap.width() as f32 - transform.tx) / transform.sx),
            (y + h).min((pixmap.height() as f32 - transform.ty) / transform.sy),
        ) {
            if let Some(mut paint) = paints.fill(rect) {
                // At native density the page uses tiny-skia's analytic rectangle
                // coverage. Keep that coverage on translated tiles by moving the
                // rectangle and its paint into the destination pixel grid together.
                if page_origin != [0.; 2] && transform.sx == 1. && transform.sy == 1. {
                    if let Some(rect) = rect.transform(transform) {
                        paint.shader.transform(transform);
                        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                    }
                } else {
                    pixmap.fill_rect(rect, &paint, transform, None);
                }
            }
        }
        return;
    }
    let Some(path) = object.outline(frame) else {
        return;
    };
    if let Some(paint) = paints.fill(path.bounds()) {
        let rule = match object.fill_rule {
            document_core::FillRule::NonZero => FillRule::Winding,
            document_core::FillRule::EvenOdd => FillRule::EvenOdd,
        };
        pixmap.fill_path(&path, &paint, rule, transform, None);
    }
    if has_stroke {
        // The path already includes the object's rotation and size. Only
        // the artboard-to-pixel transform should scale its stroke width.
        if object.stroke_profile.is_empty() {
            let style = object.stroke_style_at(frame);
            // Include the full miter or square cap in the temporary rectangular paint.
            let pad = style.width * 0.5 * style.miter_limit.max(2.);
            if let Some(ink) =
                paints.stroke(path.bounds().outset(pad, pad).unwrap_or(path.bounds()))
            {
                pixmap.stroke_path(&path, &ink, &style, transform, None);
            }
        } else if let Some(outline) = object.pressure_outline(&path, frame) {
            if let Some(ink) = paints.stroke(outline.bounds()) {
                pixmap.fill_path(&outline, &ink, FillRule::Winding, transform, None);
            }
        }
    }
}

fn paint([r, g, b, a]: [u8; 4], opacity: f32) -> Paint<'static> {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color_rgba8(r, g, b, (f32::from(a) * opacity).round() as u8);
    paint
}

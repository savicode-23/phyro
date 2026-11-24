use super::{INKSCAPE_NS, SVG_NS};
use crate::{invalid, write_file, WriteMode};
use document_core::{BlendMode, Document, Layer, LineCap, LineJoin, Shape, VectorObject};
use std::io::{self, Write};
use std::path::Path;

/// Export the evaluated frame as standard SVG geometry with editable layer and group markup.
pub fn export_svg(path: &Path, document: &Document, frame: u32, replace: bool) -> io::Result<()> {
    document.validate().map_err(invalid)?;
    if frame >= document.frames {
        return Err(invalid("The requested frame is outside the animation."));
    }
    write_file(
        path,
        if replace {
            WriteMode::Replace
        } else {
            WriteMode::Create
        },
        |file| {
            let mut out = io::BufWriter::new(file);
            header(&mut out, [0, 0], [document.width, document.height])?;
            for layer in &document.layers {
                write_layer(
                    &mut out,
                    layer,
                    document.objects.iter().filter(|o| o.layer == layer.id),
                    frame,
                )?;
            }
            writeln!(out, "</svg>")?;
            out.flush()
        },
    )
}

fn header(out: &mut impl Write, [x, y]: [i32; 2], [w, h]: [u32; 2]) -> io::Result<()> {
    writeln!(
        out,
        r#"<svg xmlns="{SVG_NS}" xmlns:inkscape="{INKSCAPE_NS}" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}" viewBox="{x} {y} {w} {h}">"#
    )
}

pub(crate) fn encode_objects(
    objects: &[&VectorObject],
    layers: &[Layer],
    frame: u32,
    origin: [i32; 2],
    size: [u32; 2],
) -> io::Result<Vec<u8>> {
    let mut svg = Vec::new();
    header(&mut svg, origin, size)?;
    let mut runs = objects.chunk_by(|a, b| a.layer == b.layer).peekable();
    if let Some(first) = runs.next() {
        if runs.peek().is_none()
            && layers
                .iter()
                .find(|l| l.id == first[0].layer)
                .is_none_or(|l| l.blend_mode.is_normal())
        {
            write_objects(&mut svg, first.iter().copied(), frame, "phyro")?;
        } else {
            // Group IDs are local to each layer. Keep their scopes and paint order
            // when several selected layers share the same cropped SVG viewport.
            let mut layers = layers.iter();
            for run in std::iter::once(first).chain(runs) {
                let layer = layers
                    .find(|layer| layer.id == run[0].layer)
                    .ok_or_else(|| invalid("Invalid selection layer order."))?;
                write_layer(&mut svg, layer, run.iter().copied(), frame)?;
            }
        }
    }
    writeln!(svg, "</svg>")?;
    Ok(svg)
}

fn write_layer<'a>(
    out: &mut impl Write,
    layer: &Layer,
    objects: impl Iterator<Item = &'a VectorObject>,
    frame: u32,
) -> io::Result<()> {
    let visible = if layer.visible {
        ""
    } else {
        r#" display="none""#
    };
    let name = escape_attribute(&layer.name)?;
    write!(
        out,
        r#"  <g inkscape:groupmode="layer" inkscape:label="{name}" data-phyro-locked="{}"{visible}"#,
        layer.locked
    )?;
    blend(out, layer.blend_mode)?;
    writeln!(out, ">")?;
    write_objects(out, objects, frame, "phyro")?;
    writeln!(out, "  </g>")
}

fn escape_attribute(value: &str) -> io::Result<String> {
    if value.chars().any(|c| {
        (c < ' ' && !matches!(c, '\t' | '\n' | '\r')) || matches!(c, '\u{fffe}' | '\u{ffff}')
    }) {
        return Err(invalid("A name contains a character XML cannot encode."));
    }
    Ok(value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
        .replace('\t', "&#9;"))
}

fn write_objects<'a>(
    out: &mut impl Write,
    objects: impl IntoIterator<Item = &'a VectorObject>,
    frame: u32,
    scope: &str,
) -> io::Result<()> {
    let mut objects = objects.into_iter().filter(|o| !o.reference_only).peekable();
    while let Some(first) = objects.next() {
        let captured = first.clip_source.is_some();
        let grouped = first.group.is_some();
        if captured || grouped {
            let key = format!("{scope}-{}-shared", first.id);
            if captured {
                super::defs::write_captured(out, first, frame, &key)?;
            }
            write!(out, "    <g")?;
            if let Some(id) = first.group {
                write!(out, r#" data-phyro-group="{id}""#)?;
                if let Some(name) = &first.group_name {
                    write!(out, r#" inkscape:label="{}""#, escape_attribute(name)?)?;
                }
            } else {
                write!(out, r#" data-phyro-ungrouped="true""#)?;
            }
            if captured {
                super::defs::write_stencil_ref(out, first, &key)?;
            }
            writeln!(out, ">")?;
        }
        write_object(out, first, frame, scope, captured)?;
        if grouped {
            while let Some(next) = objects.next_if(|next| {
                first.layer == next.layer
                    && first.group == next.group
                    && if captured {
                        first.shares_clip_mask(next, frame)
                    } else {
                        next.clip_source.is_none()
                    }
            }) {
                write_object(out, next, frame, scope, captured)?;
            }
        }
        if captured || grouped {
            writeln!(out, "    </g>")?;
        }
    }
    Ok(())
}

fn write_object(
    out: &mut impl Write,
    object: &VectorObject,
    frame: u32,
    scope: &str,
    outer_stencil: bool,
) -> io::Result<()> {
    if !object.visible_at(frame) {
        return Ok(());
    }
    let key = format!("{scope}-{}", object.id);
    if !object.stroke_profile.is_empty() && !object.is_instance() && object.has_stroke_at(frame) {
        return write_pressure_paint(out, object, frame, &key, outer_stencil);
    }
    super::defs::write_defs(out, object, &key, outer_stencil, frame)?;
    if let Some((art, child_frame)) = object.instance_at(frame) {
        let m = object
            .symbol_transform(frame)
            .ok_or_else(|| invalid("Invalid symbol transform."))?;
        write!(
            out,
            r#"    <g transform="matrix({} {} {} {} {} {})" opacity="{}""#,
            m.sx,
            m.ky,
            m.kx,
            m.sy,
            m.tx,
            m.ty,
            object.opacity_at(frame)
        )?;
        blend(out, object.blend_mode)?;
        if object.embedded_artwork.is_some()
            && super::pressure_paint::exported_composite(art, child_frame)
        {
            write!(out, r#" data-phyro-composite="true""#)?;
        }
        if !outer_stencil {
            super::defs::write_stencil_ref(out, object, &key)?;
        }
        writeln!(out, ">")?;
        if object.clip_instance.is_some() {
            writeln!(
                out,
                r#"      <defs><clipPath id="{key}-viewport" clipPathUnits="userSpaceOnUse"><rect x="0" y="0" width="{}" height="{}"/></clipPath></defs><g clip-path="url(#{key}-viewport)">"#,
                art.width, art.height
            )?;
        }
        write_placed_layers(out, art, child_frame, &key)?;
        if object.clip_instance.is_some() {
            writeln!(out, "      </g>")?;
        }
        return writeln!(out, "    </g>");
    }
    if object.image.is_none()
        && (!object.fill_blend_mode.is_normal() || !object.stroke_blend_mode.is_normal())
    {
        return write_paint_pair(out, object, frame, &key, outer_stencil);
    }
    write_geometry_at(out, object, frame, true)?;
    blend(out, object.blend_mode)?;
    let opacity = object.opacity_at(frame);
    if opacity < 1. {
        write!(out, r#" opacity="{opacity}""#)?;
    }
    if !outer_stencil {
        super::defs::write_stencil_ref(out, object, &key)?;
    }
    if object.image.is_some() {
        return writeln!(out, "/>");
    }
    write_fill(out, object, frame, &key)?;
    write_stroke(out, object, frame, &key)?;
    writeln!(out, "/>")
}

fn blend(out: &mut impl Write, mode: BlendMode) -> io::Result<()> {
    if !mode.is_normal() {
        write!(out, r#" style="mix-blend-mode:{}""#, mode.name())?;
    }
    Ok(())
}

fn write_placed_layers(
    out: &mut impl Write,
    art: &Document,
    frame: u32,
    scope: &str,
) -> io::Result<()> {
    let ordered = art.paint_order();
    if art.layers.iter().all(|l| l.blend_mode.is_normal()) {
        return write_objects(out, ordered, frame, scope);
    }
    for layer in art.layers.iter().filter(|layer| layer.visible) {
        if !layer.blend_mode.is_normal() {
            write!(out, "    <g")?;
            blend(out, layer.blend_mode)?;
            writeln!(out, ">")?;
        }
        write_objects(
            out,
            ordered.iter().copied().filter(|o| o.layer == layer.id),
            frame,
            scope,
        )?;
        if !layer.blend_mode.is_normal() {
            writeln!(out, "    </g>")?;
        }
    }
    Ok(())
}

fn write_paint_pair(
    out: &mut impl Write,
    object: &VectorObject,
    frame: u32,
    key: &str,
    outer_stencil: bool,
) -> io::Result<()> {
    let opacity = object.opacity_at(frame);
    let stencil = object.stencil.is_some() && !outer_stencil;
    let has_fill = object.has_fill_at(frame);
    let has_stroke = object.has_stroke_at(frame);
    // With one painted channel, place its blend on the masked group so the
    // CSS blend still sees the scene behind the mask.
    let (pair, mode) = if stencil && object.blend_mode.is_normal() && !(has_fill && has_stroke) {
        if has_stroke {
            ("stroke", object.stroke_blend_mode)
        } else {
            ("fill", object.fill_blend_mode)
        }
    } else {
        ("true", object.blend_mode)
    };
    write!(out, r#"    <g data-phyro-paint-pair="{pair}""#)?;
    blend(out, mode)?;
    let grouped = !mode.is_normal() || stencil || has_fill && has_stroke;
    if grouped && opacity < 1. {
        write!(out, r#" opacity="{opacity}""#)?;
    }
    if !outer_stencil {
        super::defs::write_stencil_ref(out, object, key)?;
    }
    writeln!(out, ">")?;
    for (fill, mode) in [
        (true, object.fill_blend_mode),
        (false, object.stroke_blend_mode),
    ] {
        write_geometry_at(out, object, frame, !fill)?;
        blend(out, mode)?;
        if !grouped && opacity < 1. {
            write!(out, r#" opacity="{opacity}""#)?;
        }
        if fill {
            write_fill(out, object, frame, key)?;
            write!(out, r#" stroke="none""#)?;
        } else {
            write!(out, r#" fill="none""#)?;
            write_stroke(out, object, frame, key)?;
        }
        writeln!(out, "/>")?;
    }
    writeln!(out, "    </g>")
}

fn write_stroke(
    out: &mut impl Write,
    object: &VectorObject,
    frame: u32,
    key: &str,
) -> io::Result<()> {
    stroke_paint(out, "stroke", object, frame, key)?;
    let cap = match object.cap {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    };
    let join = match object.join {
        LineJoin::Miter => "miter",
        LineJoin::Round => "round",
        LineJoin::Bevel => "bevel",
    };
    write!(
        out,
        r#" stroke-width="{}" stroke-linecap="{cap}" stroke-linejoin="{join}""#,
        object.stroke_width_at(frame)
    )
}

/// Keep rectangular pattern tiles aligned in external editors, including
/// when an imported pressure silhouette is exported again. Stroke paint in
/// user space must use the same geometry origin as the element it paints.
pub(super) fn geometry_origin(object: &VectorObject, position: [f32; 2]) -> [f32; 2] {
    if matches!(object.shape, Shape::Rectangle)
        && !object.no_fill
        && object
            .gradient
            .as_ref()
            .is_some_and(|g| matches!(g.kind, document_core::GradientKind::Rectangular { .. }))
    {
        [0., 0.]
    } else {
        position
    }
}

fn write_geometry_at(
    out: &mut impl Write,
    object: &VectorObject,
    frame: u32,
    uniform_stroke: bool,
) -> io::Result<()> {
    let ([x, y], [w, h]) = object.scaled_box_at(frame);
    let [cx, cy] = [x + w / 2., y + h / 2.];
    let shape = object.evaluated_shape_at(frame);
    let [left, top] = geometry_origin(object, [x, y]);
    if let Some(image) = &object.image {
        write!(
            out,
            r#"    <image x="{left}" y="{top}" width="{w}" height="{h}" preserveAspectRatio="none" xlink:href="data:{};base64,{}""#,
            image.mime_type(),
            image.encoded_base64()
        )?;
    } else {
        geometry(out, shape.as_ref(), [left, top], [w, h])?;
    }
    if !object.name.is_empty() {
        write!(
            out,
            r#" inkscape:label="{}""#,
            escape_attribute(&object.name)?
        )?;
    }
    if object.fill_rule == document_core::FillRule::EvenOdd {
        write!(out, r#" fill-rule="evenodd""#)?;
    }
    let rotation = object.rotation_at(frame);
    let sheared = object.skew_at(frame) != 0.;
    if [left, top] != [x, y] || sheared || object.scale_at(frame).iter().any(|s| *s < 0.) {
        let m = object.box_transform(frame);
        write!(
            out,
            r#" transform="matrix({} {} {} {} {} {})""#,
            m.sx,
            m.ky,
            m.kx,
            m.sy,
            m.tx - m.sx * left - m.kx * top,
            m.ty - m.ky * left - m.sy * top
        )?;
        if uniform_stroke && sheared && object.has_stroke_at(frame) {
            write!(out, r#" vector-effect="non-scaling-stroke""#)?;
        }
    } else if rotation != 0. {
        write!(out, r#" transform="rotate({rotation} {cx} {cy})""#)?;
    }
    Ok(())
}

fn write_fill(
    out: &mut impl Write,
    object: &VectorObject,
    frame: u32,
    key: &str,
) -> io::Result<()> {
    if object.no_fill {
        write!(out, r#" fill="none""#)?;
    } else if object.gradient.is_some() {
        write!(out, r#" fill="url(#{key}-fill)""#)?;
    } else {
        paint(out, "fill", object.fill_at(frame))?;
    }
    Ok(())
}

fn stroke_paint(
    out: &mut impl Write,
    name: &str,
    object: &VectorObject,
    frame: u32,
    key: &str,
) -> io::Result<()> {
    if object.no_stroke {
        write!(out, r#" {name}="none""#)
    } else if object.stroke_gradient.is_some() {
        write!(out, r#" {name}="url(#{key}-stroke)""#)
    } else {
        paint(out, name, object.stroke_at(frame))
    }
}

fn write_pressure_paint(
    out: &mut impl Write,
    object: &VectorObject,
    frame: u32,
    key: &str,
    outer_stencil: bool,
) -> io::Result<()> {
    let outline = object
        .outline(frame)
        .and_then(|p| object.pressure_outline(&p, frame));
    // An enabled transparent fill remains an editable paint in the SVG.
    let has_fill = !object.no_fill;
    if !has_fill && outline.is_none() {
        return Ok(());
    }
    let opacity = object.opacity_at(frame);
    let stencil = !outer_stencil && object.stencil.is_some();
    // The pressure silhouette is already in drawing coordinates. Anchor clipping to
    // the centerline's paint space even when the expanded stroke changes its bounds.
    super::defs::write_defs(out, object, key, true, frame)?;
    if stencil {
        super::defs::write_captured(out, object, frame, key)?;
    }
    let multiple_paints = object.has_fill_at(frame) && outline.is_some();
    let mode = if object.blend_mode.is_normal() && stencil && !multiple_paints {
        if outline.is_some() {
            object.stroke_blend_mode
        } else {
            object.fill_blend_mode
        }
    } else {
        object.blend_mode
    };
    let object_blend = !mode.is_normal();
    // Preserve the editable pressure composite for an enabled transparent fill.
    let has_channel_blend =
        !object.fill_blend_mode.is_normal() || !object.stroke_blend_mode.is_normal();
    let grouped_opacity = opacity < 1.
        && (object_blend || multiple_paints || !has_channel_blend && has_fill && outline.is_some());
    let composite = stencil || grouped_opacity || object_blend;
    if composite {
        write!(out, r#"    <g data-phyro-ungrouped="true""#)?;
        blend(out, mode)?;
        if grouped_opacity {
            write!(out, r#" opacity="{opacity}""#)?;
        }
        if stencil {
            super::defs::write_stencil_ref(out, object, key)?;
        }
        writeln!(out, ">")?;
    }
    if has_fill {
        write_geometry_at(out, object, frame, false)?;
        write_fill(out, object, frame, key)?;
        finish_pressure_paint(out, opacity, grouped_opacity, object.fill_blend_mode)?;
    }
    if let Some(path) = outline {
        if let Some(g) = object
            .stroke_gradient
            .as_ref()
            .filter(|g| !matches!(g.kind, document_core::GradientKind::Linear { .. }))
        {
            write_pressure_gradient(out, path, object, frame, key, g)?;
        } else {
            write!(out, r#"    <path d=""#)?;
            path_segments(out, &path)?;
            write!(out, r#"""#)?;
            stroke_paint(out, "fill", object, frame, key)?;
        }
        finish_pressure_paint(out, opacity, grouped_opacity, object.stroke_blend_mode)?;
    }
    if composite {
        writeln!(out, "    </g>")?;
    }
    Ok(())
}

fn path_segments(out: &mut impl Write, path: &tiny_skia::Path) -> io::Result<()> {
    use tiny_skia::PathSegment::*;
    for segment in path.segments() {
        match segment {
            MoveTo(p) => write!(out, "M {} {} ", p.x, p.y)?,
            LineTo(p) => write!(out, "L {} {} ", p.x, p.y)?,
            QuadTo(c, p) => write!(out, "Q {} {} {} {} ", c.x, c.y, p.x, p.y)?,
            CubicTo(a, b, p) => write!(out, "C {} {} {} {} {} {} ", a.x, a.y, b.x, b.y, p.x, p.y)?,
            Close => write!(out, "Z ")?,
        }
    }
    Ok(())
}

// An expanded pressure outline has different bounds from its centerline. Keep
// a radial or rectangular ramp editable by clipping its original paint plane
// to the silhouette. Uniformly expanding the plane leaves its contours intact.
fn write_pressure_gradient(
    out: &mut impl Write,
    path: tiny_skia::Path,
    object: &VectorObject,
    frame: u32,
    key: &str,
    gradient: &document_core::Gradient,
) -> io::Result<()> {
    use document_core::GradientKind::{Radial, Rectangular};
    let invalid_space = || invalid("Degenerate pressure stroke gradient.");
    let world = object.stroke_paint_space(frame).ok_or_else(invalid_space)?;
    let local = path
        .transform(world.invert().ok_or_else(invalid_space)?)
        .ok_or_else(invalid_space)?;
    let b = local.compute_tight_bounds().ok_or_else(invalid_space)?;
    let side = b.width().max(b.height());
    if side <= 0. {
        return Err(invalid_space());
    }
    let x = b.x();
    let y = b.y();
    let (Radial { center, radius } | Rectangular { center, radius }) = gradient.kind else {
        unreachable!()
    };
    let center = [(center[0] - x) / side, (center[1] - y) / side];
    let radius = radius / side;
    let mut paint = gradient.clone();
    paint.kind = if matches!(gradient.kind, Radial { .. }) {
        Radial { center, radius }
    } else {
        Rectangular { center, radius }
    };
    let clip = local
        .transform(tiny_skia::Transform::from_translate(-x, -y).post_scale(1. / side, 1. / side))
        .ok_or_else(invalid_space)?;
    writeln!(out, "    <defs>")?;
    super::defs::write_gradient(out, &format!("{key}-stroke"), &paint, None, 0.)?;
    write!(
        out,
        r#"    <clipPath id="{key}-pressure" clipPathUnits="objectBoundingBox"><path d=""#
    )?;
    path_segments(out, &clip)?;
    writeln!(out, r#""/></clipPath></defs>"#)?;
    // Give the pattern and its silhouette the same unit box. Including the
    // box origin in the transform keeps the tile aligned in Inkscape, too.
    let plane_to_world = tiny_skia::Transform::from_scale(side, side)
        .post_translate(x, y)
        .post_concat(world);
    write!(
        out,
        r#"    <rect x="0" y="0" width="1" height="1" transform="{}" clip-path="url(#{key}-pressure)" fill="url(#{key}-stroke)""#,
        super::defs::matrix(plane_to_world)
    )
}

fn finish_pressure_paint(
    out: &mut impl Write,
    opacity: f32,
    grouped_opacity: bool,
    mode: BlendMode,
) -> io::Result<()> {
    blend(out, mode)?;
    if !grouped_opacity && opacity < 1. {
        write!(out, r#" opacity="{opacity}""#)?;
    }
    writeln!(out, r#" stroke="none"/>"#)
}

fn paint(out: &mut impl Write, name: &str, [r, g, b, a]: [u8; 4]) -> io::Result<()> {
    write!(out, " {name}=\"#{r:02x}{g:02x}{b:02x}\"")?;
    if a != 255 {
        write!(out, r#" {name}-opacity="{}""#, f32::from(a) / 255.)?;
    }
    Ok(())
}

pub(super) fn geometry(
    out: &mut impl Write,
    shape: &Shape,
    [x, y]: [f32; 2],
    [w, h]: [f32; 2],
) -> io::Result<()> {
    let [cx, cy] = [x + w / 2., y + h / 2.];
    match shape {
        Shape::Rectangle => write!(out, r#"    <rect x="{x}" y="{y}" width="{w}" height="{h}""#)?,
        Shape::Ellipse => write!(
            out,
            r#"    <ellipse cx="{cx}" cy="{cy}" rx="{}" ry="{}""#,
            w / 2.,
            h / 2.
        )?,
        Shape::Path { .. } | Shape::Compound { .. } => {
            let closed = !matches!(shape, Shape::Path { closed: false, .. });
            let point = |p: [f32; 2]| [x + p[0] * w, y + p[1] * h];
            write!(out, r#"    <path d=""#)?;
            for nodes in shape.contours() {
                let [start_x, start_y] = point(nodes[0].point);
                write!(out, "M {start_x} {start_y}")?;
                for index in 0..(nodes.len() - 1 + usize::from(closed)) {
                    let a = &nodes[index];
                    let b = &nodes[(index + 1) % nodes.len()];
                    let [x, y] = point(b.point);
                    if a.outgoing == a.point && b.incoming == b.point {
                        if index + 1 < nodes.len() {
                            write!(out, " L {x} {y}")?;
                        }
                    } else {
                        let [ax, ay] = point(a.outgoing);
                        let [bx, by] = point(b.incoming);
                        write!(out, " C {ax} {ay} {bx} {by} {x} {y}")?;
                    }
                }
                if closed {
                    write!(out, " Z")?;
                }
            }
            write!(out, "\"")?;
        }
    }
    Ok(())
}

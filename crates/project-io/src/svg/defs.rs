pub(super) use super::defs_read::Definitions;
use document_core::{Gradient, GradientKind, VectorObject};
use std::io::{self, Write};
use tiny_skia::Transform;

pub(super) fn write_defs(
    out: &mut impl Write,
    object: &VectorObject,
    key: &str,
    outer_stencil: bool,
    frame: u32,
) -> io::Result<()> {
    let stencil = object.stencil.as_ref().filter(|_| !outer_stencil);
    let fill = object.gradient.as_ref().filter(|_| !object.no_fill);
    let stroke = object
        .stroke_gradient
        .as_ref()
        .filter(|_| !object.no_stroke);
    if fill.is_none() && stroke.is_none() && stencil.is_none() {
        return Ok(());
    }
    writeln!(out, "    <defs>")?;
    if let Some(g) = fill {
        write_gradient(out, &format!("{key}-fill"), g, None, 0.)?;
    }
    let pressure = !object.stroke_profile.is_empty() && object.has_stroke_at(frame);
    if let Some(g) = stroke.filter(|_| object.stroke_width_at(frame) == 0.) {
        write_gradient(out, &format!("{key}-stroke"), g, None, 0.)?;
    }
    if let Some(g) = stroke.filter(|g| {
        object.stroke_width_at(frame) > 0.
            && (!pressure || matches!(g.kind, GradientKind::Linear { .. }))
    }) {
        let world = object
            .stroke_paint_space(frame)
            .ok_or_else(|| crate::invalid("Empty stroke paint space."))?;
        let space = if pressure {
            world
        } else {
            let (position, _) = object.scaled_box_at(frame);
            let [x, y] = super::write::geometry_origin(object, position);
            world
                .post_concat(
                    object
                        .box_transform(frame)
                        .invert()
                        .ok_or_else(|| crate::invalid("Degenerate SVG object transform."))?,
                )
                .post_translate(x, y)
        };
        // A pattern repeats beyond its tile. Include the whole painted outline
        // so corners and caps continue to sample the padded color ramp.
        let padding = if matches!(g.kind, GradientKind::Rectangular { .. }) {
            let [l, t, r, b] = object.bounds(frame);
            let mut corners =
                [[l, t], [l, b], [r, t], [r, b]].map(|[x, y]| tiny_skia::Point::from_xy(x, y));
            world
                .invert()
                .ok_or_else(|| crate::invalid("Degenerate stroke gradient."))?
                .map_points(&mut corners);
            corners
                .iter()
                .flat_map(|p| [-p.x, -p.y, p.x - 1., p.y - 1.])
                .fold(0_f32, f32::max)
        } else {
            0.
        };
        write_gradient(out, &format!("{key}-stroke"), g, Some(space), padding)?;
    }
    if let Some(s) = stencil {
        if let Some([a, b]) = s.alpha {
            writeln!(
                out,
                r##"      <linearGradient id="{key}-fade"><stop offset="0" stop-color="#ffffff" stop-opacity="{}"/><stop offset="1" stop-color="#ffffff" stop-opacity="{}"/></linearGradient>"##,
                f32::from(a) / 255.,
                f32::from(b) / 255.
            )?;
            let [x, y] = s.origin;
            let [w, h] = s.size;
            writeln!(
                out,
                r#"      <mask id="{key}-mask" maskUnits="objectBoundingBox" maskContentUnits="objectBoundingBox" x="{x}" y="{y}" width="{w}" height="{h}">"#
            )?;
            super::write::geometry(out, &s.shape, s.origin, s.size)?;
            if !s.fill_rule.is_nonzero() {
                write!(out, r#" fill-rule="evenodd""#)?;
            }
            writeln!(out, r#" fill="url(#{key}-fade)"/></mask>"#)?;
        } else {
            writeln!(
                out,
                r#"      <clipPath id="{key}-clip" clipPathUnits="objectBoundingBox">"#
            )?;
            super::write::geometry(out, &s.shape, s.origin, s.size)?;
            if !s.fill_rule.is_nonzero() {
                write!(out, r#" clip-rule="evenodd""#)?;
            }
            writeln!(out, "/></clipPath>")?;
        }
    }
    writeln!(out, "    </defs>")
}

/// The geometry and the ramp share the cutter's own coordinate system. Applying
/// the reference to the group composites overlapping children before clipping.
pub(super) fn write_captured(
    out: &mut impl Write,
    object: &VectorObject,
    frame: u32,
    key: &str,
) -> io::Result<()> {
    let invalid = || crate::invalid("Cannot export an empty clipping transform.");
    let s = object.stencil.as_ref().ok_or_else(invalid)?;
    let target = object.paint_space(frame).ok_or_else(invalid)?;
    let axis = object.stencil_alpha_space(frame).ok_or_else(invalid)?;
    let relative = s
        .transform(target)
        .post_concat(axis.invert().ok_or_else(invalid)?);
    let mut shape =
        document_core::path_edit::primitive_path(&s.shape).unwrap_or_else(|| s.shape.clone());
    for node in shape.contours_mut().iter_mut().flatten() {
        for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
            *p = [
                relative.sx * p[0] + relative.kx * p[1] + relative.tx,
                relative.ky * p[0] + relative.sy * p[1] + relative.ty,
            ];
        }
    }
    writeln!(out, "    <defs>")?;
    let tag = if let Some([a, b]) = s.alpha {
        let bounds = s
            .outline(target)
            .and_then(|p| p.compute_tight_bounds())
            .ok_or_else(invalid)?;
        writeln!(
            out,
            r##"      <linearGradient id="{key}-fade" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#ffffff" stop-opacity="{}"/><stop offset="1" stop-color="#ffffff" stop-opacity="{}"/></linearGradient>"##,
            f32::from(a) / 255.,
            f32::from(b) / 255.
        )?;
        writeln!(
            out,
            r#"      <mask id="{key}-mask" maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse" x="{}" y="{}" width="{}" height="{}">"#,
            bounds.x(),
            bounds.y(),
            bounds.width(),
            bounds.height()
        )?;
        "mask"
    } else {
        writeln!(
            out,
            r#"      <clipPath id="{key}-clip" clipPathUnits="userSpaceOnUse">"#
        )?;
        "clipPath"
    };
    super::write::geometry(out, &shape, [0.; 2], [1.; 2])?;
    write!(
        out,
        r#" transform="matrix({} {} {} {} {} {})""#,
        axis.sx, axis.ky, axis.kx, axis.sy, axis.tx, axis.ty
    )?;
    if !s.fill_rule.is_nonzero() {
        write!(
            out,
            r#" {}="evenodd""#,
            if s.alpha.is_some() {
                "fill-rule"
            } else {
                "clip-rule"
            }
        )?;
    }
    if s.alpha.is_some() {
        write!(out, r#" fill="url(#{key}-fade)""#)?;
    }
    writeln!(out, "/></{tag}></defs>")
}
pub(super) fn write_stencil_ref(
    out: &mut impl Write,
    object: &VectorObject,
    key: &str,
) -> io::Result<()> {
    if let Some(s) = &object.stencil {
        if s.alpha.is_some() {
            write!(out, r#" mask="url(#{key}-mask)""#)?;
        } else {
            write!(out, r#" clip-path="url(#{key}-clip)""#)?;
        }
    }
    Ok(())
}
pub(super) fn matrix(m: Transform) -> String {
    format!(
        "matrix({} {} {} {} {} {})",
        m.sx, m.ky, m.kx, m.sy, m.tx, m.ty
    )
}
pub(super) fn write_gradient(
    out: &mut impl Write,
    id: &str,
    g: &Gradient,
    space: Option<Transform>,
    padding: f32,
) -> io::Result<()> {
    let mapping = space
        .map(|m| {
            format!(
                r#" gradientUnits="userSpaceOnUse" gradientTransform="{}""#,
                matrix(m)
            )
        })
        .unwrap_or_default();
    let tag = match g.kind {
        GradientKind::Linear {
            start: [x1, y1],
            end: [x2, y2],
        } => {
            writeln!(
                out,
                r#"      <linearGradient id="{id}"{mapping} x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">"#
            )?;
            "linearGradient"
        }
        GradientKind::Radial {
            center: [cx, cy],
            radius,
        } => {
            writeln!(
                out,
                r#"      <radialGradient id="{id}"{mapping} cx="{cx}" cy="{cy}" r="{radius}">"#
            )?;
            "radialGradient"
        }
        GradientKind::Rectangular { center, radius } => {
            return super::rectangular::write_pattern(
                out, id, center, radius, &g.stops, space, padding,
            );
        }
    };
    for stop in &g.stops {
        let [r, g, b, a] = stop.color;
        writeln!(
            out,
            r##"        <stop offset="{}" stop-color="#{r:02x}{g:02x}{b:02x}" stop-opacity="{}"/>"##,
            stop.offset,
            f32::from(a) / 255.
        )?;
    }
    writeln!(out, "      </{tag}>")
}

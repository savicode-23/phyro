use super::defs_read::{stops, Element};
use crate::invalid;
use document_core::{Gradient, GradientKind, GradientStop};
use std::collections::HashSet;
use std::io::{self, Write};
use tiny_skia::Transform;
use xml::writer::{EmitterConfig, EventWriter, XmlEvent};

fn element(tag: &str, attrs: &[(&str, &str)], children: Vec<Element>) -> Element {
    Element {
        tag: tag.into(),
        attrs: attrs.iter().map(|&(k, v)| (k.into(), v.into())).collect(),
        children,
    }
}

// SVG has no rectangular gradient. Paint |x| across the tile, then cover the
// two sectors where |y| is larger. The ramps agree along their shared edges.
// Render color and coverage independently so antialiasing an internal edge
// cannot blend transparent paint twice or leave a hairline between sectors.
fn pattern(
    id: &str,
    center: [f32; 2],
    radius: f32,
    rgb: Vec<Element>,
    alpha: Vec<Element>,
    space: Option<Transform>,
    padding: f32,
) -> Element {
    let [x, y] = center.map(|v| -v - padding);
    let side = 1. + 2. * padding;
    let view_box = format!("{x} {y} {side} {side}");
    let extent = 1. + f64::from(side) + f64::from(x.abs().max(y.abs()));
    let color_id = format!("{id}-color");
    let alpha_id = format!("{id}-alpha");
    let mask_id = format!("{id}-mask");
    let ramp = |key: &str, children| {
        element(
            "linearGradient",
            &[
                ("id", key),
                ("gradientUnits", "userSpaceOnUse"),
                ("x1", "0"),
                ("y1", "0"),
                ("x2", &radius.to_string()),
                ("y2", "0"),
            ],
            children,
        )
    };
    let field = |key: &str| {
        let paint = format!("url(#{key})");
        let triangle = format!("M0 0L{extent} -{extent}V{extent}Z");
        [
            (
                format!("M-{extent} -{extent}H{extent}V{extent}H-{extent}Z"),
                "matrix(1 0 0 1 0 0)",
            ),
            (
                format!("M0 -{extent}H{extent}V{extent}H0Z"),
                "matrix(-1 0 0 -1 0 0)",
            ),
            (triangle.clone(), "matrix(0 1 -1 0 0 0)"),
            (triangle, "matrix(0 -1 1 0 0 0)"),
        ]
        .into_iter()
        .map(|(d, t)| {
            element(
                "path",
                &[("d", &d), ("transform", t), ("fill", &paint)],
                vec![],
            )
        })
        .collect()
    };
    let mask = element(
        "mask",
        &[
            ("id", &mask_id),
            ("maskUnits", "userSpaceOnUse"),
            ("maskContentUnits", "userSpaceOnUse"),
            ("x", &x.to_string()),
            ("y", &y.to_string()),
            ("width", &side.to_string()),
            ("height", &side.to_string()),
        ],
        field(&alpha_id),
    );
    let defs = element(
        "defs",
        &[],
        vec![ramp(&color_id, rgb), ramp(&alpha_id, alpha), mask],
    );
    let group = element(
        "g",
        &[("mask", &format!("url(#{mask_id})"))],
        field(&color_id),
    );
    let mut result = element(
        "pattern",
        &[
            ("id", id),
            ("data-phyro-gradient", "rectangular"),
            ("patternUnits", "objectBoundingBox"),
            ("width", &side.to_string()),
            ("height", &side.to_string()),
            ("viewBox", &view_box),
            ("preserveAspectRatio", "none"),
        ],
        vec![defs, group],
    );
    if let Some(space) = space {
        for (k, v) in [
            ("x", (-padding).to_string()),
            ("y", (-padding).to_string()),
            ("patternUnits", "userSpaceOnUse".into()),
            ("patternTransform", super::defs::matrix(space)),
            ("data-phyro-center", format!("{} {}", center[0], center[1])),
        ] {
            result.attrs.insert(k.into(), v);
        }
    }
    result
}

fn emit(out: &mut EventWriter<impl Write>, e: &Element) -> io::Result<()> {
    let mut start = XmlEvent::start_element(e.tag.as_str());
    for (k, v) in &e.attrs {
        start = start.attr(k.as_str(), v);
    }
    out.write(start).map_err(io::Error::other)?;
    for child in &e.children {
        emit(out, child)?;
    }
    out.write(XmlEvent::end_element()).map_err(io::Error::other)
}

pub(super) fn write_pattern(
    out: &mut impl Write,
    id: &str,
    center: [f32; 2],
    radius: f32,
    stops: &[GradientStop],
    space: Option<Transform>,
    padding: f32,
) -> io::Result<()> {
    let (rgb, alpha) = stops
        .iter()
        .map(|s| {
            let [r, g, b, a] = s.color;
            let stop = |color: String| {
                element(
                    "stop",
                    &[("offset", &s.offset.to_string()), ("stop-color", &color)],
                    vec![],
                )
            };
            (
                stop(format!("#{r:02x}{g:02x}{b:02x}")),
                stop(format!("#{a:02x}{a:02x}{a:02x}")),
            )
        })
        .unzip();
    emit(
        &mut EmitterConfig::new()
            .write_document_declaration(false)
            .create_writer(out),
        &pattern(id, center, radius, rgb, alpha, space, padding),
    )
}

// Inkscape's Plain SVG writer gives IDs to anonymous paths, stops and
// containers. Required IDs and paint references still belong to the template.
fn matches_pattern<'a>(
    actual: &'a Element,
    expected: &Element,
    ids: &mut HashSet<&'a str>,
) -> bool {
    if let Some(id) = actual.get("id") {
        if id.is_empty() || id.chars().any(char::is_whitespace) || !ids.insert(id) {
            return false;
        }
    }
    actual.tag == expected.tag
        && expected
            .attrs
            .iter()
            .all(|(k, v)| actual.attrs.get(k) == Some(v))
        && actual
            .attrs
            .keys()
            .all(|k| k == "id" || expected.attrs.contains_key(k))
        && actual.children.len() == expected.children.len()
        && actual
            .children
            .iter()
            .zip(&expected.children)
            .all(|(a, e)| matches_pattern(a, e, ids))
}

/// Only the generated pattern represents one editable gradient. Check the
/// actual SVG paths and mask as well as its marker before collapsing them.
pub(super) fn read_pattern(e: &Element) -> io::Result<(Gradient, Option<Transform>)> {
    let unsupported = || invalid("Unsupported SVG pattern or modified rectangular gradient.");
    if e.get("data-phyro-gradient") != Some("rectangular") {
        return Err(unsupported());
    }
    let id = e.get("id").ok_or_else(unsupported)?;
    let viewport = super::pathdata::numbers(e.get("viewBox").ok_or_else(unsupported)?)?;
    let [x, y, _, _] = viewport.as_slice() else {
        return Err(unsupported());
    };
    let (center, space, padding) = if let Some(matrix) = e.get("patternTransform") {
        let coords = super::pathdata::numbers(e.get("data-phyro-center").ok_or_else(unsupported)?)?;
        let [cx, cy] = coords.as_slice() else {
            return Err(unsupported());
        };
        let padding = -e.number("x", 0.)?;
        if padding < 0. {
            return Err(unsupported());
        }
        (
            [*cx, *cy],
            Some(super::transform::Matrix::parse(matrix)?.transform()),
            padding,
        )
    } else {
        ([-x, -y], None, 0.)
    };
    let [defs, _] = e.children.as_slice() else {
        return Err(unsupported());
    };
    let [color, alpha, _] = defs.children.as_slice() else {
        return Err(unsupported());
    };
    if !(2..=256).contains(&color.children.len()) || alpha.children.len() != color.children.len() {
        return Err(unsupported());
    }
    let radius = color.number("x2", 0.)?;
    let expected = pattern(
        id,
        center,
        radius,
        color.children.clone(),
        alpha.children.clone(),
        space,
        padding,
    );
    if !matches_pattern(e, &expected, &mut HashSet::new()) {
        return Err(unsupported());
    }
    let mut paint = stops(color)?;
    for (s, a) in paint.iter_mut().zip(stops(alpha)?) {
        let [r, g, b, opacity] = a.color;
        if s.offset != a.offset || s.color[3] != 255 || r != g || g != b || opacity != 255 {
            return Err(unsupported());
        }
        s.color[3] = r;
    }
    let gradient = Gradient {
        kind: GradientKind::Rectangular { center, radius },
        stops: paint,
    };
    if !gradient.valid() {
        return Err(unsupported());
    }
    Ok((gradient, space))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inkscape_ids_preserve_the_rectangular_gradient_and_its_paint_references() {
        let stops = vec![
            element(
                "stop",
                &[("offset", "0"), ("stop-color", "#000000")],
                vec![],
            ),
            element(
                "stop",
                &[("offset", "1"), ("stop-color", "#ffffff")],
                vec![],
            ),
        ];
        let mut e = pattern("box", [0.5; 2], 0.75, stops.clone(), stops, None, 0.);
        let expected = read_pattern(&e).unwrap();
        fn add_ids(e: &mut Element, next: &mut usize) {
            if e.get("id").is_none() {
                *next += 1;
                e.attrs.insert("id".into(), format!("{}{next}", e.tag));
            }
            for child in &mut e.children {
                add_ids(child, next);
            }
        }
        add_ids(&mut e, &mut 0);
        assert_eq!(read_pattern(&e).unwrap(), expected);

        let mut duplicate = e.clone();
        let defs_id = duplicate.children[0].get("id").unwrap().to_owned();
        duplicate.children[1].attrs.insert("id".into(), defs_id);
        assert!(read_pattern(&duplicate).is_err());
        let mut shadow = e.clone();
        shadow.children[0].children[0].children[0]
            .attrs
            .insert("id".into(), "box-mask".into());
        assert!(read_pattern(&shadow).is_err());
        e.children[1].children[0]
            .attrs
            .insert("fill".into(), "url(#box-alpha)".into());
        assert!(read_pattern(&e).is_err());
    }
}

use super::{pathdata, style, transform::Matrix, SVG_NS};
use crate::invalid;
use document_core::{
    ClipBinding, ClipSource, Document, Gradient, GradientKind, GradientStop, Shape, Stencil,
    VectorObject,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use xml::reader::{EventReader, XmlEvent};

#[derive(Default)]
pub(super) struct Definitions {
    gradients: BTreeMap<String, Gradient>,
    user_gradients: BTreeSet<String>,
    paint_transforms: BTreeMap<String, tiny_skia::Transform>,
    stencils: BTreeMap<String, SvgStencil>,
    expanded_nodes: usize,
}
struct SvgStencil {
    stencil: Stencil,
    user_space: bool,
    transform: Matrix,
    unit_ramp: bool,
}
#[derive(Clone, PartialEq)]
pub(super) struct Element {
    pub(super) tag: String,
    pub(super) attrs: BTreeMap<String, String>,
    pub(super) children: Vec<Element>,
}
impl Element {
    pub(super) fn get(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str)
    }
    pub(super) fn number(&self, key: &str, default: f32) -> io::Result<f32> {
        let Some(raw) = self.get(key) else {
            return Ok(default);
        };
        let value = if let Some(p) = raw.strip_suffix('%') {
            p.parse::<f32>().ok().map(|v| v / 100.)
        } else {
            raw.parse::<f32>().ok()
        };
        value
            .filter(|v| v.is_finite())
            .ok_or_else(|| invalid(format!("Invalid SVG definition coordinate '{raw}'.")))
    }
    fn check(&self, allowed: &[&str]) -> io::Result<()> {
        for key in self.attrs.keys() {
            if matches!(
                self.tag.as_str(),
                "path" | "rect" | "circle" | "ellipse" | "clipPath" | "mask"
            ) && matches!(key.as_str(), "clip-rule" | "fill-rule")
            {
                if matches!(self.get(key), Some("evenodd" | "nonzero")) {
                    continue;
                }
                return Err(invalid("Invalid SVG stencil fill rule."));
            }
            if key != "id" && !allowed.contains(&key.as_str()) {
                return Err(invalid(format!(
                    "Unsupported attribute '{key}' on SVG <{}>.",
                    self.tag
                )));
            }
        }
        Ok(())
    }
}
impl Definitions {
    /// Resolve local references independently of definition order, including definitions following their first use.
    pub(super) fn parse(source: &str) -> io::Result<Self> {
        let mut stack: Vec<Element> = Vec::new();
        let mut entries = Vec::new();
        let mut count = 0;
        for event in EventReader::new(source.as_bytes()) {
            match event.map_err(|e| invalid(e.to_string()))? {
                XmlEvent::StartElement {
                    name, attributes, ..
                } if !stack.is_empty()
                    || (name.local_name == "defs"
                        && matches!(name.namespace.as_deref(), None | Some(SVG_NS))) =>
                {
                    if stack.len() > 4
                        || count >= 65536
                        || !matches!(name.namespace.as_deref(), None | Some(SVG_NS))
                    {
                        return Err(invalid("Unsupported or oversized SVG definitions."));
                    }
                    count += 1;
                    let mut attrs = BTreeMap::new();
                    for a in attributes {
                        if a.name.namespace.as_deref() == Some(super::INKSCAPE_NS)
                            && a.name.local_name == "collect"
                            && a.value == "always"
                        {
                            continue;
                        }
                        let xlink = a.name.namespace.as_deref()
                            == Some("http://www.w3.org/1999/xlink")
                            && a.name.local_name == "href";
                        if a.name.namespace.is_some() && !xlink {
                            return Err(invalid(
                                "Namespaced definition attributes are unsupported.",
                            ));
                        }
                        let key = super::canonical_attribute(a.name.local_name);
                        if attrs.insert(key, a.value).is_some() {
                            return Err(invalid("Duplicate SVG definition attribute."));
                        }
                    }
                    stack.push(Element {
                        tag: name.local_name,
                        attrs,
                        children: Vec::new(),
                    });
                }
                XmlEvent::EndElement { .. } if !stack.is_empty() => {
                    let node = stack.pop().unwrap();
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    } else {
                        node.check(&[])?;
                        entries.extend(node.children);
                    }
                }
                XmlEvent::CData(text) | XmlEvent::Characters(text)
                    if !stack.is_empty() && !text.trim().is_empty() =>
                {
                    return Err(invalid("Unexpected text in SVG definitions."))
                }
                _ => {}
            }
        }
        let mut result = Self::default();
        let mut by_id = BTreeMap::new();
        for entry in &entries {
            let id = entry
                .get("id")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("An SVG definition needs an id."))?;
            if by_id.insert(id, entry).is_some() {
                return Err(invalid(format!("Duplicate SVG definition id '{id}'.")));
            }
        }
        let mut inherited_stops = 0;
        for (&id, &entry) in &by_id {
            if entry.tag == "pattern" {
                if ["color", "alpha", "mask"]
                    .iter()
                    .any(|suffix| by_id.contains_key(format!("{id}-{suffix}").as_str()))
                {
                    return Err(invalid("Duplicate SVG rectangular fill definition id."));
                }
                let (g, space) = super::rectangular::read_pattern(entry)?;
                if let Some(m) = space {
                    result.user_gradients.insert(id.into());
                    result.paint_transforms.insert(id.into(), m);
                }
                result.gradients.insert(id.into(), g);
            }
            if matches!(entry.tag.as_str(), "linearGradient" | "radialGradient") {
                let mut resolved = resolve_gradient(entry, &by_id, &mut inherited_stops)?;
                if resolved.get("gradientUnits") == Some("userSpaceOnUse") {
                    result.user_gradients.insert(id.into());
                    resolved.attrs.remove("gradientUnits");
                    let matrix = resolved
                        .attrs
                        .remove("gradientTransform")
                        .map(|s| Matrix::parse(&s))
                        .transpose()?
                        .unwrap_or(Matrix::IDENTITY);
                    result
                        .paint_transforms
                        .insert(id.into(), matrix.transform());
                }
                result.gradients.insert(id.into(), gradient(&resolved)?);
            }
        }
        for entry in &entries {
            match entry.tag.as_str() {
                "linearGradient" | "radialGradient" | "pattern" => {}
                "clipPath" | "mask" => {
                    let shape = stencil(
                        entry,
                        &result.gradients,
                        &result.user_gradients,
                        &result.paint_transforms,
                    )?;
                    result
                        .stencils
                        .insert(entry.get("id").unwrap().into(), shape);
                }
                _ => {
                    return Err(invalid(format!(
                        "Unsupported SVG definition <{}>.",
                        entry.tag
                    )))
                }
            }
        }
        Ok(result)
    }
    pub(super) fn apply_paint(
        &mut self,
        object: &mut VectorObject,
        style: &style::Style,
        element_space: Matrix,
    ) -> io::Result<()> {
        for (id, opacity, stroke) in [
            (&style.fill_ref, style.fill_opacity(), false),
            (&style.stroke_ref, style.stroke_opacity(), true),
        ] {
            let Some(id) = id else { continue };
            let g = self
                .gradients
                .get(id)
                .ok_or_else(|| invalid(format!("Unknown gradient '{id}'.")))?;
            self.expanded_nodes += g.stops.len();
            if self.expanded_nodes > 1_000_000 {
                return Err(invalid("SVG expanded paint data exceeds the import limit."));
            }
            let mut g = g.clone();
            if let Some(m) = self.paint_transforms.get(id) {
                let bounds = if stroke {
                    object.stroke_paint_space(0)
                } else {
                    object.paint_space(0)
                };
                let inverse = bounds
                    .and_then(|t| t.invert())
                    .ok_or_else(|| invalid("Degenerate SVG gradient paint box."))?;
                let m = m
                    .post_concat(element_space.transform())
                    .post_concat(inverse);
                if !m.is_identity() {
                    g.kind = normalized_gradient(&g.kind, m)?;
                }
            }
            for s in &mut g.stops {
                s.color[3] = (f32::from(s.color[3]) * opacity).round() as u8;
            }
            if stroke {
                object.stroke_gradient = Some(g);
            } else {
                object.gradient = Some(g);
            }
        }
        Ok(())
    }

    pub(super) fn apply_stencil(
        &mut self,
        doc: &mut Document,
        start: usize,
        id: &str,
        mask: bool,
        transform: Matrix,
        group: bool,
    ) -> io::Result<()> {
        let def = self
            .stencils
            .get(id)
            .filter(|d| d.stencil.alpha.is_some() == mask)
            .ok_or_else(|| invalid(format!("Unknown or mismatched stencil '{id}'.")))?;
        let count = doc.objects.len() - start;
        let nodes = def
            .stencil
            .shape
            .contours()
            .iter()
            .map(Vec::len)
            .sum::<usize>();
        self.expanded_nodes = self
            .expanded_nodes
            .saturating_add(count.saturating_mul(nodes));
        if self.expanded_nodes > 1_000_000 {
            return Err(invalid("SVG expanded paint data exceeds the import limit."));
        }
        if count == 0 {
            return Ok(());
        }
        if doc.objects[start..].iter().any(|o| o.stencil.is_some()) {
            return Err(invalid("Nested clipping or masking is unsupported."));
        }
        if !def.user_space {
            if group {
                return Err(invalid("Group clips require user-space stencil geometry."));
            }
            doc.objects[start].stencil = Some(def.stencil.clone());
            return Ok(());
        }
        let coords = transform.append(def.transform);
        let (pos, size, shape) = def.outline_in(coords)?;
        let layer = doc.objects[start].layer;
        doc.add_shape(pos, size, [0, 0, 0, 255], shape, layer);
        let mut object = doc.objects.pop().unwrap();
        object.fill_rule = def.stencil.fill_rule;
        let alpha_box = if def.unit_ramp {
            coords
        } else {
            coords
                .append(Matrix::translate(
                    def.stencil.origin[0],
                    def.stencil.origin[1],
                ))
                .append(Matrix::scale(def.stencil.size[0], def.stencil.size[1]))
        };
        let source = Arc::new(ClipSource {
            object,
            stack_index: start + count,
            grouped: count > 1,
            mask_space: alpha_box.0,
        });
        for target in &mut doc.objects[start..] {
            let s = Stencil::capture(&source.object, target, 0, def.stencil.alpha)
                .ok_or_else(|| invalid("An SVG clipping operation needs a nonempty paint box."))?;
            let p = target.paint_space(0).unwrap();
            target.stencil = Some(s);
            target.clip_source = Some(ClipBinding {
                source: source.clone(),
                paint_space: [p.sx, p.ky, p.kx, p.sy, p.tx, p.ty],
                original_layer: layer,
                original_target: target.id,
            });
        }
        Ok(())
    }
}

fn normalized_gradient(kind: &GradientKind, m: tiny_skia::Transform) -> io::Result<GradientKind> {
    let identity_error = [m.sx - 1., m.sy - 1., m.kx, m.ky, m.tx, m.ty]
        .into_iter()
        .map(f32::abs)
        .fold(0., f32::max);
    if identity_error <= 2e-5 {
        return Ok(kind.clone());
    }
    let point = |[x, y]: [f32; 2]| [m.sx * x + m.kx * y + m.tx, m.ky * x + m.sy * y + m.ty];
    match *kind {
        GradientKind::Linear { start, end } => {
            // Transform the normal to the isocolor lines by the inverse transpose.
            let inv = m
                .invert()
                .ok_or_else(|| invalid("Invalid SVG gradient transform."))?;
            let [dx, dy] = [end[0] - start[0], end[1] - start[1]];
            let [nx, ny] = [inv.sx * dx + inv.ky * dy, inv.kx * dx + inv.sy * dy];
            let factor = (dx * dx + dy * dy) / (nx * nx + ny * ny);
            let start = point(start);
            let end = [start[0] + nx * factor, start[1] + ny * factor];
            if start == end || !end.into_iter().chain(start).all(f32::is_finite) {
                return Err(invalid("Invalid SVG linear gradient geometry."));
            }
            Ok(GradientKind::Linear { start, end })
        }
        GradientKind::Radial { center, radius } | GradientKind::Rectangular { center, radius } => {
            let sx = m.sx.hypot(m.ky);
            let sy = m.kx.hypot(m.sy);
            let eps = 1e-5 * sx.max(sy);
            if (sx - sy).abs() > eps
                || (m.sx * m.kx + m.ky * m.sy).abs() > eps * sx
                || matches!(kind, GradientKind::Rectangular { .. })
                    && m.sx.abs() > eps
                    && m.ky.abs() > eps
            {
                return Err(invalid(
                    "This SVG gradient transform cannot be represented in the object's paint box.",
                ));
            }
            let center = point(center);
            let radius = radius * sx;
            Ok(if matches!(kind, GradientKind::Radial { .. }) {
                GradientKind::Radial { center, radius }
            } else {
                GradientKind::Rectangular { center, radius }
            })
        }
    }
}

impl SvgStencil {
    fn outline_in(&self, transform: Matrix) -> io::Result<([f32; 2], [f32; 2], Shape)> {
        let s = &self.stencil;
        let unit = transform
            .append(Matrix::translate(s.origin[0], s.origin[1]))
            .append(Matrix::scale(s.size[0], s.size[1]));
        let mut shape =
            document_core::path_edit::primitive_path(&s.shape).unwrap_or_else(|| s.shape.clone());
        for node in shape.contours_mut().iter_mut().flatten() {
            for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                *p = unit.point(*p);
            }
        }
        let b = shape
            .local_outline([1.; 2])
            .and_then(|p| p.compute_tight_bounds()?.to_non_zero_rect())
            .ok_or_else(|| invalid("Empty SVG clipping path."))?;
        let pos = [b.x(), b.y()];
        let size = [b.width(), b.height()];
        for node in shape.contours_mut().iter_mut().flatten() {
            for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                *p = [(p[0] - pos[0]) / size[0], (p[1] - pos[1]) / size[1]];
            }
        }
        Ok((pos, size, shape))
    }
}

fn resolve_gradient(
    entry: &Element,
    by_id: &BTreeMap<&str, &Element>,
    expanded: &mut usize,
) -> io::Result<Element> {
    let mut attrs = BTreeMap::new();
    let mut stops = None;
    let mut current = entry;
    let mut chain = Vec::new();
    loop {
        let id = current.get("id").unwrap();
        if chain.contains(&id) || chain.len() >= 32 {
            return Err(invalid(
                "Cyclic or excessively nested SVG gradient references.",
            ));
        }
        chain.push(id);
        for (k, v) in &current.attrs {
            let foreign_axis = current.tag != entry.tag
                && matches!(
                    k.as_str(),
                    "x1" | "y1" | "x2" | "y2" | "cx" | "cy" | "r" | "fx" | "fy"
                );
            if !foreign_axis && k != "href" {
                attrs.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        if stops.is_none() && !current.children.is_empty() {
            stops = Some(&current.children);
        }
        let Some(href) = current.get("href") else {
            break;
        };
        current = href
            .strip_prefix('#')
            .and_then(|k| by_id.get(k).copied())
            .filter(|e| matches!(e.tag.as_str(), "linearGradient" | "radialGradient"))
            .ok_or_else(|| invalid("Unknown local SVG gradient template."))?;
    }
    let mut children = Vec::new();
    if let Some(s) = stops {
        *expanded += s.len();
        if *expanded > 1_000_000 || s.len() > 256 {
            return Err(invalid(
                "Expanded SVG gradient templates exceed the import limit.",
            ));
        }
        children.clone_from(s);
    }
    Ok(Element {
        tag: entry.tag.clone(),
        attrs,
        children,
    })
}
fn gradient(e: &Element) -> io::Result<Gradient> {
    if e.get("gradientUnits").unwrap_or("objectBoundingBox") != "objectBoundingBox"
        || e.get("spreadMethod").unwrap_or("pad") != "pad"
    {
        return Err(invalid(
            "Only padded object-bounding-box gradients are supported.",
        ));
    }
    let kind = if e.tag == "linearGradient" {
        e.check(&["x1", "y1", "x2", "y2", "gradientUnits", "spreadMethod"])?;
        GradientKind::Linear {
            start: [e.number("x1", 0.)?, e.number("y1", 0.)?],
            end: [e.number("x2", 1.)?, e.number("y2", 0.)?],
        }
    } else {
        e.check(&["cx", "cy", "r", "fx", "fy", "gradientUnits", "spreadMethod"])?;
        let center = [e.number("cx", 0.5)?, e.number("cy", 0.5)?];
        if [e.number("fx", center[0])?, e.number("fy", center[1])?] != center {
            return Err(invalid("Offset radial focal points are unsupported."));
        }
        GradientKind::Radial {
            center,
            radius: e.number("r", 0.5)?,
        }
    };
    let g = Gradient {
        kind,
        stops: stops(e)?,
    };
    if !g.valid() {
        return Err(invalid("Invalid gradient geometry or stops."));
    }
    Ok(g)
}

pub(super) fn stops(e: &Element) -> io::Result<Vec<GradientStop>> {
    let mut stops = Vec::new();
    for stop in &e.children {
        if stop.tag != "stop" || !stop.children.is_empty() {
            return Err(invalid("Expected an SVG gradient stop."));
        }
        stop.check(&["offset", "stop-color", "stop-opacity", "style"])?;
        let mut color = stop.get("stop-color").unwrap_or("black");
        let mut opacity = stop.get("stop-opacity").unwrap_or("1");
        for decl in stop
            .get("style")
            .unwrap_or("")
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let (key, value) = decl
                .split_once(':')
                .ok_or_else(|| invalid("Malformed SVG stop style."))?;
            match key.trim() {
                "stop-color" => color = value.trim(),
                "stop-opacity" => opacity = value.trim(),
                _ => return Err(invalid("Unknown SVG stop style.")),
            }
        }
        let mut color = style::color(color)?;
        color[3] = (f32::from(color[3]) * style::opacity(opacity)?).round() as u8;
        stops.push(GradientStop {
            offset: stop.number("offset", 0.)?,
            color,
        });
    }
    Ok(stops)
}
fn stencil(
    e: &Element,
    gradients: &BTreeMap<String, Gradient>,
    user_gradients: &BTreeSet<String>,
    transforms: &BTreeMap<String, tiny_skia::Transform>,
) -> io::Result<SvgStencil> {
    let clip = e.tag == "clipPath";
    if clip {
        e.check(&["clipPathUnits"])?;
    } else {
        e.check(&["maskUnits", "maskContentUnits", "x", "y", "width", "height"])?;
    }
    let units = e
        .get(if clip {
            "clipPathUnits"
        } else {
            "maskContentUnits"
        })
        .unwrap_or("userSpaceOnUse");
    let user_space = match units {
        "userSpaceOnUse" => true,
        "objectBoundingBox" => false,
        _ => return Err(invalid("Unknown SVG stencil coordinate system.")),
    };
    if !clip && e.get("maskUnits").unwrap_or("objectBoundingBox") != units {
        return Err(invalid(
            "SVG mask content and bounds must use the same coordinate system.",
        ));
    }
    let [child] = e.children.as_slice() else {
        return Err(invalid("A stencil needs one geometric silhouette."));
    };
    if !child.children.is_empty() {
        return Err(invalid("Nested stencil geometry is unsupported."));
    }
    let mut child = child.clone();
    let transform = if let Some(t) = child.attrs.remove("transform") {
        if !user_space {
            return Err(invalid(
                "Transformed object-bounding-box stencils are unsupported.",
            ));
        }
        Matrix::parse(&t)?
    } else {
        Matrix::IDENTITY
    };
    let (origin, size, shape) = match child.tag.as_str() {
        "rect" => {
            child.check(&[
                "x",
                "y",
                "width",
                "height",
                "fill",
                "fill-opacity",
                "opacity",
            ])?;
            (
                [child.number("x", 0.)?, child.number("y", 0.)?],
                [child.number("width", 0.)?, child.number("height", 0.)?],
                Shape::Rectangle,
            )
        }
        "circle" | "ellipse" => {
            child.check(if child.tag == "circle" {
                &["cx", "cy", "r", "fill", "fill-opacity", "opacity"]
            } else {
                &["cx", "cy", "rx", "ry", "fill", "fill-opacity", "opacity"]
            })?;
            let (rx, ry) = if child.tag == "circle" {
                let r = child.number("r", 0.)?;
                (r, r)
            } else {
                (child.number("rx", 0.)?, child.number("ry", 0.)?)
            };
            (
                [child.number("cx", 0.)? - rx, child.number("cy", 0.)? - ry],
                [2. * rx, 2. * ry],
                Shape::Ellipse,
            )
        }
        "path" => {
            child.check(&["d", "fill", "fill-opacity", "opacity"])?;
            let mut s = pathdata::path_data(child.get("d").unwrap_or(""))?;
            let b = s
                .local_outline([1.; 2])
                .and_then(|p| p.compute_tight_bounds())
                .ok_or_else(|| invalid("Empty stencil path."))?;
            let origin = [b.x(), b.y()];
            let size = [b.width(), b.height()];
            if size.contains(&0.) {
                return Err(invalid("Zero-area stencil path."));
            }
            for node in s.contours_mut().iter_mut().flatten() {
                for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                    for i in 0..2 {
                        p[i] = (p[i] - origin[i]) / size[i];
                    }
                }
            }
            if let Shape::Path { closed, .. } = &mut s {
                *closed = true;
            }
            (origin, size, s)
        }
        _ => return Err(invalid("Unsupported stencil geometry.")),
    };
    let mut unit_ramp = false;
    let alpha = if clip {
        None
    } else {
        let (x, y, w, h) = (
            e.number("x", -0.1)?,
            e.number("y", -0.1)?,
            e.number("width", 1.2)?,
            e.number("height", 1.2)?,
        );
        if !user_space
            && (x > origin[0] + 1e-5
                || y > origin[1] + 1e-5
                || x + w + 1e-5 < origin[0] + size[0]
                || y + h + 1e-5 < origin[1] + size[1])
        {
            return Err(invalid(
                "An independently cropped SVG mask cannot be edited.",
            ));
        }
        let fill = child.get("fill").unwrap_or("black");
        let mask_alpha = |color: [u8; 4]| -> io::Result<u8> {
            match color {
                [255, 255, 255, a] => Ok(a),
                [0, 0, 0, _] => Ok(0),
                _ => Err(invalid("Masks require white alpha or black silhouettes.")),
            }
        };
        let a = if let Some(id) = style::local_reference(fill)? {
            if transforms.get(&id).is_some_and(|m| !m.is_identity()) {
                return Err(invalid("Transformed SVG mask ramps are unsupported."));
            }
            unit_ramp = user_gradients.contains(&id);
            if unit_ramp && !user_space {
                return Err(invalid(
                    "User-space mask ramps require a user-space silhouette.",
                ));
            }
            let g = gradients
                .get(&id)
                .ok_or_else(|| invalid("Missing mask gradient."))?;
            if !matches!(
                g.kind,
                GradientKind::Linear {
                    start: [0., 0.],
                    end: [1., 0.]
                }
            ) || g.stops.len() != 2
                || g.stops[0].offset != 0.
                || g.stops[1].offset != 1.
            {
                return Err(invalid("Unsupported mask ramp."));
            }
            [mask_alpha(g.stops[0].color)?, mask_alpha(g.stops[1].color)?]
        } else {
            [mask_alpha(style::color(fill)?)?; 2]
        };
        let factor = style::opacity(child.get("fill-opacity").unwrap_or("1"))?
            * style::opacity(child.get("opacity").unwrap_or("1"))?;
        Some(a.map(|v| (f32::from(v) * factor).round() as u8))
    };
    let s = Stencil {
        shape,
        fill_rule: {
            let attr = if clip { "clip-rule" } else { "fill-rule" };
            if child.get(attr).or(e.get(attr)) == Some("evenodd") {
                document_core::FillRule::EvenOdd
            } else {
                document_core::FillRule::NonZero
            }
        },
        origin,
        size,
        alpha,
    };
    if !s.valid() {
        return Err(invalid("Invalid stencil bounds."));
    }
    let result = SvgStencil {
        stencil: s,
        transform,
        unit_ramp,
        user_space,
    };
    if !clip && user_space {
        let ([x, y], [w, h], _) = result.outline_in(transform)?;
        let left = e.number("x", 0.)?;
        let top = e.number("y", 0.)?;
        let tolerance = 0.0001 * w.max(h).max(1.);
        if left > x + tolerance
            || top > y + tolerance
            || left + e.number("width", 0.)? < x + w - tolerance
            || top + e.number("height", 0.)? < y + h - tolerance
        {
            return Err(invalid(
                "An independently cropped SVG mask cannot be edited.",
            ));
        }
    }
    Ok(result)
}

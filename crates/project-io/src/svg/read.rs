use super::pathdata::{numbers, path_data};
use super::style::Style;
use super::transform::Matrix;
use super::{INKSCAPE_NS, SVG_NS};
use crate::invalid;
use document_core::{Document, Layer, PathNode, Shape};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;
use xml::attribute::OwnedAttribute;
use xml::reader::{EventReader, XmlEvent};

const MAX_BYTES: usize = 16 * 1024 * 1024;
type Attrs = BTreeMap<String, String>;
const SODIPODI_NS: &str = "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd";

fn artwork_name(attrs: &Attrs) -> Option<&str> {
    ["inkscape:label", "id"].iter().find_map(|key| {
        attrs.get(*key).map(|s| s.trim()).filter(|s| {
            !s.is_empty() && s.chars().count() <= 80 && !s.chars().any(char::is_control)
        })
    })
}

#[derive(Clone, Copy)]
enum Auxiliary {
    NamedView,
    Metadata,
    PaintDefs,
}

impl Auxiliary {
    fn check_attributes(self, attrs: &[OwnedAttribute]) -> io::Result<()> {
        for attr in attrs {
            let key = attr.name.local_name.as_str();
            // RDF resource identifiers are inert strings. SVG links, event handlers
            // and CSS are outside the metadata subset that this importer skips.
            if key.starts_with("on") || matches!(key, "href" | "style" | "src") {
                return Err(invalid(format!(
                    "Unsupported active attribute '{key}' in SVG metadata."
                )));
            }
        }
        Ok(())
    }

    fn child(self, name: &xml::name::OwnedName) -> bool {
        match self {
            Self::NamedView => matches!(
                (name.namespace.as_deref(), name.local_name.as_str()),
                (Some(INKSCAPE_NS), "page" | "grid") | (Some(SODIPODI_NS), "guide")
            ),
            Self::Metadata => matches!(
                name.namespace.as_deref(),
                Some(
                    "http://www.w3.org/1999/02/22-rdf-syntax-ns#"
                        | "http://creativecommons.org/ns#"
                        | "http://web.resource.org/cc/"
                        | "http://purl.org/dc/elements/1.1/"
                        | "http://purl.org/dc/terms/"
                )
            ),
            Self::PaintDefs => matches!(name.namespace.as_deref(), None | Some(SVG_NS)),
        }
    }
}

#[derive(Clone)]
struct Context {
    tag: String,
    preserve_space: bool,
    transform: Matrix,
    style: Style,
    layer: Option<u64>,
    group: Option<u64>,
    // Group effects apply to the composited children when the element closes.
    group_paint: Option<GroupPaint>,
}

#[derive(Clone)]
struct GroupPaint {
    start: usize,
    stencil: Option<(String, bool)>,
    opacity: f32,
    blend_mode: document_core::BlendMode,
    ungrouped: bool,
    composite: bool,
    paint_pair: Option<PaintPair>,
}

#[derive(Clone, Copy)]
enum PaintPair {
    Separate,
    BlendedFill,
    BlendedStroke,
}

struct PendingText {
    attrs: Attrs,
    outer: String,
    line: Option<(Context, String)>,
}

#[derive(Default)]
struct Importer {
    doc: Document,
    stack: Vec<Context>,
    groups: BTreeMap<(u64, Option<u64>, String), u64>,
    group_names: BTreeMap<u64, Arc<str>>,
    next_group: u64,
    plain_layer: Option<u64>,
    started: bool,
    used_first_layer: bool,
    definitions: super::defs::Definitions,
    images: super::bitmap::Images,
    pending_text: Option<PendingText>,
}

pub fn import_svg(path: &Path) -> io::Result<Document> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("SVG exceeds the 16 MiB import limit."));
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| invalid("SVG must use UTF-8."))?;
    if source.contains("<!DOCTYPE") || source.contains("<!ENTITY") {
        return Err(invalid(
            "SVG document types and custom entities are unsupported.",
        ));
    }
    let mut importer = Importer {
        definitions: super::defs::Definitions::parse(source)?,
        ..Importer::default()
    };
    let mut auxiliary: Option<(Auxiliary, usize)> = None;
    let mut page_count = 0;
    for event in EventReader::new(source.as_bytes()) {
        match event.map_err(|e| invalid(format!("Invalid SVG XML: {e}")))? {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                let ns = name.namespace.as_deref();
                let tag = name.local_name.as_str();
                if let Some((mode, depth)) = &mut auxiliary {
                    if *depth >= 64 || !mode.child(&name) {
                        return Err(invalid(format!("Unsupported content <{tag}> inside SVG definitions or editor metadata.")));
                    }
                    if !matches!(mode, Auxiliary::PaintDefs) {
                        mode.check_attributes(&attributes)?;
                    }
                    if matches!(mode, Auxiliary::NamedView) && tag == "page" {
                        page_count += 1;
                        if page_count > 1 {
                            return Err(invalid(
                                "SVG files with multiple Inkscape pages are unsupported.",
                            ));
                        }
                    }
                    *depth += 1;
                    continue;
                }
                if importer
                    .stack
                    .last()
                    .is_some_and(|c| matches!(c.tag.as_str(), "svg" | "g"))
                {
                    let mode = match (ns, tag) {
                        (Some(SODIPODI_NS), "namedview") => Some(Auxiliary::NamedView),
                        (None | Some(SVG_NS), "metadata") => Some(Auxiliary::Metadata),
                        (None | Some(SVG_NS), "defs") => Some(Auxiliary::PaintDefs),
                        _ => None,
                    };
                    if let Some(mode) = mode {
                        mode.check_attributes(&attributes)?;
                        auxiliary = Some((mode, 1));
                        continue;
                    }
                }
                if name.namespace.as_deref().is_some_and(|ns| ns != SVG_NS) {
                    return Err(invalid(format!(
                        "Unsupported SVG namespace on <{}>.",
                        name.local_name
                    )));
                }
                importer.start(
                    &name.local_name,
                    collect_attributes(&name.local_name, attributes)?,
                )?;
            }
            XmlEvent::EndElement { .. } => {
                if let Some((_, depth)) = &mut auxiliary {
                    *depth -= 1;
                    if *depth == 0 {
                        auxiliary = None;
                    }
                } else if let Some(ctx) = importer.stack.pop() {
                    if ctx.tag == "text" {
                        let pending = importer.pending_text.take().expect("text started");
                        let (run, raw) = if let Some(line) = pending.line {
                            if !pending.outer.trim().is_empty() {
                                return Err(invalid(
                                    "SVG text mixing a tspan and other text is unsupported.",
                                ));
                            }
                            line
                        } else {
                            (ctx.clone(), pending.outer)
                        };
                        let content = if run.preserve_space {
                            raw.replace(['\r', '\n', '\t'], " ")
                        } else {
                            raw.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
                        };
                        if !content.trim().is_empty() {
                            let text = run.style.font.lettering(content)?;
                            importer.shape(
                                "text",
                                &pending.attrs,
                                &run,
                                run.layer.expect("text layer"),
                                Some(text),
                            )?;
                        }
                    }
                    if let Some(paint) = ctx.group_paint {
                        importer.finish_group(paint, ctx.transform)?;
                    }
                }
            }
            XmlEvent::ProcessingInstruction { .. } => {
                return Err(invalid("SVG processing instructions are unsupported."))
            }
            XmlEvent::Characters(s) | XmlEvent::CData(s) | XmlEvent::Whitespace(s)
                if importer.pending_text.is_some() =>
            {
                let pending = importer.pending_text.as_mut().expect("text started");
                if importer.stack.last().is_some_and(|ctx| ctx.tag == "tspan") {
                    pending.line.as_mut().expect("tspan started").1.push_str(&s);
                } else {
                    pending.outer.push_str(&s);
                }
            }
            XmlEvent::Characters(s) | XmlEvent::CData(s)
                if !s.trim().is_empty()
                    && !matches!(auxiliary, Some((Auxiliary::Metadata, _)))
                    && !importer
                        .stack
                        .last()
                        .is_some_and(|ctx| matches!(ctx.tag.as_str(), "title" | "desc")) =>
            {
                return Err(invalid("Unexpected SVG text content."));
            }
            _ => {}
        }
    }
    if !importer.started || !importer.stack.is_empty() {
        return Err(invalid("Incomplete SVG document."));
    }
    if !importer.group_names.is_empty() {
        for object in &mut importer.doc.objects {
            object.group_name = object
                .group
                .and_then(|id| importer.group_names.get(&id).cloned());
        }
    }
    importer.doc.validate().map_err(invalid)?;
    Ok(importer.doc)
}

fn collect_attributes(tag: &str, attributes: Vec<OwnedAttribute>) -> io::Result<Attrs> {
    let mut result = Attrs::new();
    for attr in attributes {
        let key = match attr.name.namespace.as_deref() {
            None => super::canonical_attribute(attr.name.local_name),
            Some(SODIPODI_NS)
                if tag == "tspan" && attr.name.local_name == "role" && attr.value == "line" =>
            {
                continue
            }
            Some("http://www.w3.org/1999/xlink")
                if tag == "image" && attr.name.local_name == "href" =>
            {
                "xlink:href".to_owned()
            }
            Some(SODIPODI_NS)
                if tag == "path"
                    && matches!(
                        attr.name.local_name.as_str(),
                        "type" | "sides" | "cx" | "cy" | "r1" | "r2" | "arg1" | "arg2"
                    ) =>
            {
                continue
            }
            Some(INKSCAPE_NS)
                if tag == "path"
                    && matches!(
                        attr.name.local_name.as_str(),
                        "flatsided" | "rounded" | "randomized"
                    ) =>
            {
                continue
            }
            Some(INKSCAPE_NS) if matches!(attr.name.local_name.as_str(), "groupmode" | "label") => {
                format!("inkscape:{}", attr.name.local_name)
            }
            Some(SODIPODI_NS) if attr.name.local_name == "insensitive" => {
                "data-phyro-locked".to_owned()
            }
            Some(SODIPODI_NS)
                if matches!(
                    attr.name.local_name.as_str(),
                    "docname" | "version" | "nodetypes"
                ) =>
            {
                continue
            }
            Some(INKSCAPE_NS)
                if matches!(
                    attr.name.local_name.as_str(),
                    "version"
                        | "export-filename"
                        | "export-xdpi"
                        | "export-ydpi"
                        | "export-batch-name"
                        | "export-batch-path"
                        | "connector-curvature"
                        | "transform-center-x"
                        | "transform-center-y"
                ) =>
            {
                continue
            }
            Some("http://www.w3.org/XML/1998/namespace") if attr.name.local_name == "space" => {
                "xml:space".to_owned()
            }
            _ => {
                return Err(invalid(format!(
                    "Unsupported SVG attribute '{}'.",
                    attr.name
                )))
            }
        };
        result.insert(key, attr.value);
    }
    Ok(result)
}

fn check_attrs(tag: &str, attrs: &Attrs) -> io::Result<()> {
    let allowed: &[&str] = match tag {
        "svg" => &[
            "width",
            "height",
            "viewBox",
            "version",
            "preserveAspectRatio",
            "x",
            "y",
        ],
        "g" => &[
            "inkscape:groupmode",
            "inkscape:label",
            "data-phyro-group",
            "data-phyro-ungrouped",
            "data-phyro-composite",
            "data-phyro-paint-pair",
            "data-phyro-locked",
        ],
        "rect" => &["x", "y", "width", "height", "rx", "ry"],
        "image" => &[
            "x",
            "y",
            "width",
            "height",
            "preserveAspectRatio",
            "href",
            "xlink:href",
        ],
        "ellipse" => &["cx", "cy", "rx", "ry"],
        "circle" => &["cx", "cy", "r"],
        "polygon" | "polyline" => &["points"],
        "path" => &["d"],
        "line" => &["x1", "y1", "x2", "y2"],
        "text" | "tspan" => &["x", "y"],
        "title" | "desc" => &[],
        _ => return Err(invalid(format!("Unsupported SVG element <{tag}>."))),
    };
    for key in attrs.keys() {
        if matches!(
            key.as_str(),
            "id" | "inkscape:label" | "transform" | "style" | "xml:space"
        ) || super::style::PRESENTATION.contains(&key.as_str())
            || allowed.contains(&key.as_str())
        {
            continue;
        }
        return Err(invalid(format!(
            "Unsupported SVG attribute '{key}' on <{tag}>."
        )));
    }
    Ok(())
}

impl Importer {
    fn start(&mut self, tag: &str, attrs: Attrs) -> io::Result<()> {
        if self.stack.len() >= 64 {
            return Err(invalid("SVG nesting exceeds 64 levels."));
        }
        check_attrs(tag, &attrs)?;
        let root = self.stack.is_empty();
        if root && (tag != "svg" || self.started) {
            return Err(invalid("Expected one SVG root element."));
        }
        if !root
            && match (tag, self.stack.last().unwrap().tag.as_str()) {
                ("tspan", parent) => parent != "text",
                ("svg", _) => true,
                (_, parent) => !matches!(parent, "svg" | "g"),
            }
        {
            return Err(invalid(format!("Unsupported placement of <{tag}>.")));
        }
        let mut ctx = self.stack.last().cloned().unwrap_or_else(|| Context {
            tag: String::new(),
            preserve_space: false,
            transform: Matrix::IDENTITY,
            style: Style::default(),
            layer: None,
            group: None,
            group_paint: None,
        });
        ctx.tag = tag.into();
        ctx.group_paint = None;
        if let Some(space) = attrs.get("xml:space") {
            ctx.preserve_space = match space.as_str() {
                "preserve" => true,
                "default" => false,
                _ => return Err(invalid("Invalid SVG xml:space value.")),
            };
        }
        if root {
            self.started = true;
            let (width, height, transform) = viewport(&attrs)?;
            self.doc.width = width;
            self.doc.height = height;
            ctx.transform = transform;
        }
        if let Some(t) = attrs.get("transform") {
            let transform = Matrix::parse(t)?;
            ctx.transform = if root {
                transform.append(ctx.transform)
            } else {
                ctx.transform.append(transform)
            };
        }
        let layer_group = tag == "g"
            && attrs
                .get("inkscape:groupmode")
                .is_some_and(|v| v == "layer");
        let ungrouped = attrs
            .get("data-phyro-ungrouped")
            .is_some_and(|v| v == "true");
        let composite = attrs
            .get("data-phyro-composite")
            .is_some_and(|v| v == "true");
        let paint_pair = match attrs.get("data-phyro-paint-pair").map(String::as_str) {
            Some("true") => Some(PaintPair::Separate),
            Some("fill") => Some(PaintPair::BlendedFill),
            Some("stroke") => Some(PaintPair::BlendedStroke),
            _ => None,
        };
        if tag == "g" && !layer_group {
            ctx.style.apply_presentation(&attrs)?;
        } else {
            ctx.style.apply(&attrs, root || layer_group)?;
        }
        if ctx.style.clip_ref.is_some() && ctx.style.mask_ref.is_some() {
            return Err(invalid("Combined clipping and masking are unsupported."));
        }
        if matches!(tag, "g" | "svg") {
            let object_blend = !layer_group && !ctx.style.blend_mode.is_normal();
            let mask = ctx.style.mask_ref.is_some();
            let stencil = ctx
                .style
                .clip_ref
                .take()
                .or_else(|| ctx.style.mask_ref.take())
                .map(|id| (id, mask));
            if stencil.is_some()
                || ctx.style.opacity != 1.
                || composite
                || paint_pair.is_some()
                || object_blend
            {
                if tag != "g" || layer_group {
                    return Err(invalid("Unsupported SVG compositing on a root or layer."));
                }
                ctx.group_paint = Some(GroupPaint {
                    start: self.doc.objects.len(),
                    stencil,
                    opacity: ctx.style.opacity,
                    blend_mode: ctx.style.blend_mode,
                    ungrouped,
                    composite,
                    paint_pair,
                });
            }
        }
        if layer_group {
            if self.stack.len() != 1 {
                return Err(invalid("Only top-level SVG layers can be imported."));
            }
            let name = attrs
                .get("inkscape:label")
                .or(attrs.get("id"))
                .map(String::as_str)
                .unwrap_or("Layer");
            let id = self.new_layer(
                name,
                !ctx.style.hidden,
                attrs
                    .get("data-phyro-locked")
                    .is_some_and(|v| v == "true" || v == "1"),
            );
            self.plain_layer = None;
            ctx.layer = Some(id);
            self.doc.layers.last_mut().unwrap().blend_mode = ctx.style.blend_mode;
            ctx.style.hidden = false;
        } else if ctx.style.hidden {
            return Err(invalid("SVG display:none is supported on layers only."));
        } else if tag == "g" {
            let layer = ctx.layer.unwrap_or_else(|| self.plain_layer());
            ctx.layer = Some(layer);
            let new_id = self.next_group + 1;
            let id = attrs
                .get("data-phyro-group")
                .map(|key| {
                    *self
                        .groups
                        .entry((layer, ctx.group, key.clone()))
                        .or_insert(new_id)
                })
                .unwrap_or(new_id);
            self.next_group = self.next_group.max(id);
            if !ungrouped && !composite && paint_pair.is_none() && ctx.style.blend_mode.is_normal()
            {
                // A native object has one logical group. Assign the nearest
                // authoring group, keeping paint order and editable leaves.
                // The context stack restores the parent for later siblings.
                ctx.group = Some(id);
                if let Some(name) = artwork_name(&attrs) {
                    self.group_names
                        .entry(id)
                        .or_insert_with(|| Arc::from(name));
                }
            }
        } else if tag == "text" {
            ctx.layer = Some(ctx.layer.unwrap_or_else(|| self.plain_layer()));
            self.pending_text = Some(PendingText {
                attrs,
                outer: String::new(),
                line: None,
            });
        } else if tag == "tspan" {
            let pending = self.pending_text.as_mut().expect("inside text");
            if pending.line.is_some() {
                return Err(invalid(
                    "SVG text with multiple styled spans is unsupported.",
                ));
            }
            // Inkscape gives a single line its own baseline and font style.
            // The text element's ID/name still identifies the whole object.
            for key in ["x", "y"] {
                if let Some(value) = attrs.get(key) {
                    pending.attrs.insert(key.into(), value.clone());
                }
            }
            ctx.style.opacity *= self.stack.last().expect("parent text").style.opacity;
            // A single tspan supplies the text run; the outer text still owns compositing.
            if ctx.style.blend_mode.is_normal() {
                ctx.style.blend_mode = self.stack.last().expect("parent text").style.blend_mode;
            }
            pending.line = Some((ctx.clone(), String::new()));
        } else if !matches!(tag, "svg" | "title" | "desc") {
            let layer = ctx.layer.unwrap_or_else(|| self.plain_layer());
            self.shape(tag, &attrs, &ctx, layer, None)?;
        }
        self.stack.push(ctx);
        Ok(())
    }

    fn finish_group(&mut self, paint: GroupPaint, transform: Matrix) -> io::Result<()> {
        use super::pressure_paint;
        let children = &self.doc.objects[paint.start..];
        let blended = !paint.blend_mode.is_normal();
        if paint.stencil.is_some()
            && !blended
            && children
                .windows(2)
                .any(|pair| pair[0].group != pair[1].group)
        {
            return Err(invalid(
                "A shared clip or mask across nested SVG object groups is unsupported.",
            ));
        }
        let pressure = pressure_paint::paints(children, paint.ungrouped || paint.composite);
        if let Some(pair) = paint.paint_pair {
            let [fill, stroke] = children else {
                return Err(invalid("An SVG paint pair needs a fill and a stroke."));
            };
            if fill.layer != stroke.layer
                || fill.group != stroke.group
                || fill.shape != stroke.shape
                || fill.position != stroke.position
                || fill.size != stroke.size
                || fill.rotation != stroke.rotation
                || fill.scale != stroke.scale
                || fill.opacity != stroke.opacity
                || fill.is_instance()
                || stroke.is_instance()
                || fill.image.is_some()
                || stroke.image.is_some()
                || fill.stencil.is_some()
                || stroke.stencil.is_some()
                || !fill.no_stroke
                || !stroke.no_fill
            {
                return Err(invalid("The SVG fill and stroke must share one outline."));
            }
            let stroke = self.doc.objects.pop().unwrap();
            let fill = &mut self.doc.objects[paint.start];
            fill.fill_blend_mode = fill.blend_mode;
            fill.stroke_blend_mode = stroke.blend_mode;
            fill.blend_mode = paint.blend_mode;
            match pair {
                PaintPair::Separate => {}
                PaintPair::BlendedFill => {
                    fill.fill_blend_mode = paint.blend_mode;
                    fill.blend_mode = Default::default();
                }
                PaintPair::BlendedStroke => {
                    fill.stroke_blend_mode = paint.blend_mode;
                    fill.blend_mode = Default::default();
                }
            }
            fill.opacity *= paint.opacity;
            fill.no_stroke = stroke.no_stroke;
            fill.stroke = stroke.stroke;
            fill.stroke_gradient = stroke.stroke_gradient;
            fill.stroke_width = stroke.stroke_width;
            fill.cap = stroke.cap;
            fill.join = stroke.join;
        } else if blended {
            if children.is_empty() {
                return Ok(());
            }
            let children = self.doc.objects.split_off(paint.start);
            let mut object = pressure_paint::composite(children, paint.opacity)?;
            object.blend_mode = paint.blend_mode;
            self.doc.objects.push(object);
        } else if paint.composite || paint.opacity != 1. || paint.ungrouped && pressure {
            if !pressure {
                return Err(invalid("Unsupported SVG group paint or opacity."));
            }
            let children = self.doc.objects.split_off(paint.start);
            self.doc
                .objects
                .push(pressure_paint::composite(children, paint.opacity)?);
        } else if children.iter().any(pressure_paint::plane) {
            // For a clipped group of several authored objects, keep its
            // editable members and isolate just the nested pressure planes.
            let children = self.doc.objects.split_off(paint.start);
            for o in children {
                self.doc.objects.push(if pressure_paint::plane(&o) {
                    pressure_paint::composite(vec![o], 1.)?
                } else {
                    o
                });
            }
        }
        if let Some((id, mask)) = paint.stencil {
            self.definitions.apply_stencil(
                &mut self.doc,
                paint.start,
                &id,
                mask,
                transform,
                true,
            )?;
        }
        Ok(())
    }

    fn new_layer(&mut self, name: &str, visible: bool, locked: bool) -> u64 {
        if !self.used_first_layer {
            self.doc.layers.clear();
            self.used_first_layer = true;
        }
        let id = self.doc.layers.len() as u64 + 1;
        self.doc.layers.push(Layer {
            id,
            name: name.into(),
            visible,
            locked,
            blend_mode: Default::default(),
        });
        id
    }

    // Preserve paint order if ordinary SVG geometry occurs between explicit layers.
    fn plain_layer(&mut self) -> u64 {
        if let Some(id) = self.plain_layer {
            return id;
        }
        let id = self.new_layer("Artwork", true, false);
        self.plain_layer = Some(id);
        id
    }
}

impl Importer {
    fn shape(
        &mut self,
        tag: &str,
        attrs: &Attrs,
        ctx: &Context,
        layer: u64,
        text: Option<super::text::Lettering>,
    ) -> io::Result<()> {
        let n = |key| number(attrs, key);
        let stroke = ctx.style.stroke_rgba();
        let (sx, sy, orthogonal) = ctx.transform.axes();
        if !(sx > 0. && sy > 0.) {
            return Err(invalid("Degenerate SVG transform."));
        }
        let bbox = ctx.style.uses_object_bbox()
            || self
                .stack
                .last()
                .and_then(|parent| parent.group_paint.as_ref())
                .is_some_and(|paint| paint.paint_pair.is_some());
        let [a, b, c, d, _, _] = ctx.transform.0;
        if bbox && !orthogonal {
            return Err(invalid("Sheared SVG paint bounds are unsupported."));
        }
        let place = |p: [f32; 2], size: [f32; 2], shape| {
            let center = ctx
                .transform
                .point([p[0] + size[0] / 2., p[1] + size[1] / 2.]);
            let size = [sx * size[0], sy * size[1]];
            (
                [center[0] - size[0] / 2., center[1] - size[1] / 2.],
                size,
                shape,
                b.atan2(a).to_degrees(),
            )
        };
        if tag != "image"
            && !ctx.style.no_stroke
            && (stroke[3] != 0 || ctx.style.stroke_ref.is_some())
            && ctx.style.stroke_width > 0.
            && (!orthogonal || (sx - sy).abs() > 1e-5 * sx.max(sy))
        {
            return Err(invalid("A sheared or nonuniformly scaled SVG stroke cannot be represented as an editable uniform stroke."));
        }
        let mut bitmap = None;
        let (layout, editable) = text
            .map(|v| (Some(v.layout), v.editable))
            .unwrap_or_default();
        let primitive = match tag {
            "image" => {
                let href = attrs
                    .get("href")
                    .or_else(|| attrs.get("xlink:href"))
                    .ok_or_else(|| invalid("An SVG image needs embedded PNG or JPEG data."))?;
                let image = self.images.read(href)?;
                let size = |k: &str| {
                    attrs
                        .get(k)
                        .filter(|s| s.as_str() != "auto")
                        .map(|_| number(attrs, k))
                        .transpose()
                };
                let placement = super::bitmap::placement(
                    &image,
                    [n("x")?, n("y")?],
                    [size("width")?, size("height")?],
                    attrs
                        .get("preserveAspectRatio")
                        .map_or("xMidYMid meet", String::as_str),
                )?;
                let primitive = (placement.position, placement.size, Shape::Rectangle);
                bitmap = Some((image, placement));
                Some(primitive)
            }
            "text" => {
                if !orthogonal {
                    return Err(invalid("Sheared editable SVG text is unsupported."));
                }
                let layout = layout.expect("text is finalized at its closing tag");
                use document_core::typography::TextAlign;
                let shift = match ctx.style.font.anchor {
                    TextAlign::Left => 0.,
                    TextAlign::Center => 0.5,
                    TextAlign::Right => 1.,
                };
                Some((
                    [n("x")? - shift * layout.size[0], n("y")? - layout.baseline],
                    layout.size,
                    layout.shape,
                ))
            }
            "rect" => {
                let size = [n("width")?, n("height")?];
                let rx = attrs.get("rx").map(|_| n("rx")).transpose()?;
                let ry = attrs.get("ry").map(|_| n("ry")).transpose()?;
                let shape =
                    rounded_rectangle(size, rx.or(ry).unwrap_or(0.), ry.or(rx).unwrap_or(0.))?;
                Some(([n("x")?, n("y")?], size, shape))
            }
            "circle" | "ellipse" => {
                let [rx, ry] = if tag == "circle" {
                    [n("r")?; 2]
                } else {
                    [n("rx")?, n("ry")?]
                };
                Some((
                    [n("cx")? - rx, n("cy")? - ry],
                    [2. * rx, 2. * ry],
                    Shape::Ellipse,
                ))
            }
            _ => None,
        };
        // Plain SVG paths bake their matrix into their nodes below. Primitives
        // and object-bounding-box paint keep their local coordinate system.
        let reflected = (primitive.is_some() || bbox) && a * d - b * c < 0.;
        let (position, size, shape, rotation) = if let Some((p, size, shape)) = primitive {
            if size.iter().any(|&v| v < 0.) {
                return Err(invalid("SVG shape dimensions must be nonnegative."));
            }
            if size.contains(&0.) {
                return Ok(());
            }
            if !orthogonal {
                return Err(invalid(
                    "Sheared SVG rectangles and ellipses are unsupported.",
                ));
            }
            place(p, size, shape)
        } else {
            let mut shape = match tag {
                "line" => Shape::Path {
                    nodes: vec![corner([n("x1")?, n("y1")?]), corner([n("x2")?, n("y2")?])],
                    closed: false,
                },
                "polygon" | "polyline" => {
                    let points =
                        numbers(attrs.get("points").map(String::as_str).unwrap_or_default())?;
                    let (pairs, remainder) = points.as_chunks::<2>();
                    if !remainder.is_empty() {
                        return Err(invalid("An SVG point requires both x and y."));
                    }
                    let nodes = pairs.iter().copied().map(corner).collect();
                    Shape::Path {
                        nodes,
                        closed: tag == "polygon",
                    }
                }
                "path" => path_data(attrs.get("d").map(String::as_str).unwrap_or_default())?,
                _ => return Err(invalid(format!("Unexpected SVG shape <{tag}>."))),
            };
            if shape.contours().iter().any(|nodes| nodes.len() < 2) {
                return Err(invalid("An editable SVG path needs at least two nodes."));
            }
            let mut min = [f32::INFINITY; 2];
            let mut max = [f32::NEG_INFINITY; 2];
            for node in shape.contours_mut().iter_mut().flatten() {
                for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                    if !bbox {
                        *p = ctx.transform.point(*p);
                    }
                    for i in 0..2 {
                        min[i] = min[i].min(p[i]);
                        max[i] = max[i].max(p[i]);
                    }
                }
            }
            if bbox {
                // SVG objectBoundingBox uses actual curve extrema in the element's local frame.
                let bounds = shape
                    .local_outline([1.; 2])
                    .and_then(|path| path.compute_tight_bounds())
                    .ok_or_else(|| invalid("SVG paint requires finite path bounds."))?;
                min = [bounds.left(), bounds.top()];
                max = [bounds.right(), bounds.bottom()];
            }
            // A horizontal or vertical segment still needs an invertible editing frame.
            let size = std::array::from_fn(|i| {
                if min[i] == max[i] {
                    1.
                } else {
                    max[i] - min[i]
                }
            });
            for node in shape.contours_mut().iter_mut().flatten() {
                for p in [&mut node.point, &mut node.incoming, &mut node.outgoing] {
                    for i in 0..2 {
                        p[i] = (p[i] - min[i]) / size[i];
                    }
                }
            }
            if bbox {
                place(min, size, shape)
            } else {
                (min, size, shape, 0.)
            }
        };
        self.doc
            .add_shape(position, size, ctx.style.fill_rgba(), shape, layer);
        let object = self
            .doc
            .objects
            .last_mut()
            .expect("add_shape appends the new object");
        if let Some(name) = artwork_name(attrs) {
            object.name = name.to_owned();
        }
        object.rotation = rotation;
        if reflected {
            object.scale[1] = -1.;
        }
        object.fill_rule = ctx.style.fill_rule;
        object.text = editable;
        object.opacity = ctx.style.opacity;
        object.blend_mode = ctx.style.blend_mode;
        object.stroke = stroke;
        object.stroke_width = ctx.style.stroke_width * sx;
        object.cap = ctx.style.cap;
        object.join = ctx.style.join;
        if let Some((image, _)) = &bitmap {
            object.image = Some(Arc::clone(image));
            object.fill = [0; 4];
            object.stroke = [0; 4];
            object.stroke_width = 0.;
        } else {
            object.no_fill = ctx.style.no_fill;
            object.no_stroke = ctx.style.no_stroke;
            self.definitions
                .apply_paint(object, &ctx.style, ctx.transform)?;
        }
        object.group = ctx.group;
        if let Some(id) = ctx.style.clip_ref.as_ref().or(ctx.style.mask_ref.as_ref()) {
            let start = self.doc.objects.len() - 1;
            self.definitions.apply_stencil(
                &mut self.doc,
                start,
                id,
                ctx.style.mask_ref.is_some(),
                ctx.transform,
                false,
            )?;
            let target = &mut self.doc.objects[start];
            if let (Some((_, layout)), Some(stencil), None) =
                (bitmap, &mut target.stencil, &target.clip_source)
            {
                layout.map_viewport_stencil(stencil);
            }
        }
        Ok(())
    }
}

fn corner(point: [f32; 2]) -> PathNode {
    PathNode::corner(point)
}

/// SVG radii are clamped independently to half the side length. Store the
/// straight edges and each elliptical quarter as editable cubic segments.
fn rounded_rectangle(size: [f32; 2], rx: f32, ry: f32) -> io::Result<Shape> {
    if rx < 0. || ry < 0. {
        return Err(invalid("SVG corner radii must be nonnegative."));
    }
    if rx == 0. || ry == 0. || size.iter().any(|&s| s <= 0.) {
        return Ok(Shape::Rectangle);
    }
    let x = (rx / size[0]).min(0.5);
    let y = (ry / size[1]).min(0.5);
    // Cubic control distance for a quarter circle, 4/3 * tan(pi/8).
    let dx = x * 0.552_284_8;
    let dy = y * 0.552_284_8;
    let corners = [
        ([x, 0.], [-dx, 0.], [0., 0.]),
        ([1. - x, 0.], [0., 0.], [dx, 0.]),
        ([1., y], [0., -dy], [0., 0.]),
        ([1., 1. - y], [0., 0.], [0., dy]),
        ([1. - x, 1.], [dx, 0.], [0., 0.]),
        ([x, 1.], [0., 0.], [-dx, 0.]),
        ([0., 1. - y], [0., dy], [0., 0.]),
        ([0., y], [0., 0.], [0., -dy]),
    ];
    let nodes = corners
        .into_iter()
        .map(|(point, a, b)| PathNode {
            handle_style: Default::default(),
            point,
            incoming: [point[0] + a[0], point[1] + a[1]],
            outgoing: [point[0] + b[0], point[1] + b[1]],
        })
        .collect();
    Ok(Shape::Path {
        nodes,
        closed: true,
    })
}

pub(super) fn length(s: &str) -> io::Result<f32> {
    let s = s.trim();
    // SVG absolute lengths are CSS pixels at 96 dpi.
    let (number, scale) = [
        ("px", 1.),
        ("mm", 96. / 25.4),
        ("cm", 96. / 2.54),
        ("in", 96.),
        ("pt", 96. / 72.),
        ("pc", 16.),
        ("Q", 96. / 101.6),
    ]
    .into_iter()
    .find_map(|(unit, scale)| s.strip_suffix(unit).map(|v| (v, scale)))
    .unwrap_or((s, 1.));
    number
        .trim()
        .parse::<f32>()
        .ok()
        .map(|n| n * scale)
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid(format!("Unsupported SVG length '{s}'.")))
}

fn number(attrs: &Attrs, key: &str) -> io::Result<f32> {
    attrs
        .get(key)
        .map(|s| length(s))
        .transpose()
        .map(|v| v.unwrap_or(0.))
}

fn viewport(attrs: &Attrs) -> io::Result<(u32, u32, Matrix)> {
    let view = attrs.get("viewBox").map(|v| numbers(v)).transpose()?;
    let (x, y, w, h) = match view.as_deref() {
        Some(&[x, y, w, h]) if w > 0. && h > 0. => (x, y, w, h),
        None => (0., 0., 300., 150.),
        _ => {
            return Err(invalid(
                "SVG viewBox requires four numbers and positive dimensions.",
            ))
        }
    };
    let width = attrs
        .get("width")
        .map(|s| length(s))
        .transpose()?
        .unwrap_or(w);
    let height = attrs
        .get("height")
        .map(|s| length(s))
        .transpose()?
        .unwrap_or(h);
    if width <= 0.
        || height <= 0.
        || width.round() > document_core::MAX_DIMENSION as f32
        || height.round() > document_core::MAX_DIMENSION as f32
    {
        return Err(invalid(
            "SVG artboard dimensions must be positive and within the project size limit.",
        ));
    }
    let transform = if view.is_some() {
        let sx = width / w;
        let sy = height / h;
        match attrs
            .get("preserveAspectRatio")
            .map(String::as_str)
            .unwrap_or("xMidYMid")
        {
            "xMidYMid" | "xMidYMid meet" => {
                let s = sx.min(sy);
                Matrix::translate((width - w * s) / 2., (height - h * s) / 2.)
                    .append(Matrix::scale(s, s))
            }
            "none" => Matrix::scale(sx, sy),
            value => {
                return Err(invalid(format!(
                    "Unsupported preserveAspectRatio '{value}'."
                )))
            }
        }
        .append(Matrix::translate(-x, -y))
    } else {
        Matrix::IDENTITY
    };
    // Retain the physical scale of the geometry and round the raster artboard only.
    Ok((
        width.round().max(1.) as u32,
        height.round().max(1.) as u32,
        transform,
    ))
}

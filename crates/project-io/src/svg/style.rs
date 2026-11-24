use crate::invalid;
use document_core::{BlendMode, LineCap, LineJoin};
use std::collections::BTreeMap;
use std::io;

pub(super) fn local_reference(value: &str) -> io::Result<Option<String>> {
    if !value.starts_with("url(") {
        return Ok(None);
    }
    let id = value
        .strip_prefix("url(#")
        .and_then(|s| s.strip_suffix(')'))
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
        })
        .ok_or_else(|| invalid("Only local SVG paint references are supported."))?;
    Ok(Some(id.to_owned()))
}

pub(super) const PRESENTATION: &[&str] = &[
    "fill",
    "stroke",
    "fill-opacity",
    "stroke-opacity",
    "opacity",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "fill-rule",
    "stroke-dasharray",
    "stroke-dashoffset",
    "vector-effect",
    "display",
    "visibility",
    "paint-order",
    "marker",
    "marker-start",
    "marker-mid",
    "marker-end",
    "clip-path",
    "clip-rule",
    "mask",
    "filter",
    "mix-blend-mode",
    "isolation",
    "color",
    "color-rendering",
    "shape-rendering",
    "enable-background",
    "overflow",
    "font-size",
    "font-family",
    "font-weight",
    "font-style",
    "font-variation-settings",
    "font-variant",
    "font-stretch",
    "writing-mode",
    "direction",
    "-inkscape-font-specification",
    "-inkscape-stroke",
    "stop-color",
    "stop-opacity",
    "text-anchor",
];

#[derive(Clone)]
pub(super) struct Style {
    pub(super) fill_rule: document_core::FillRule,
    pub(super) font: super::text::Font,
    pub(super) fill: [u8; 4],
    pub(super) stroke: [u8; 4],
    pub(super) no_fill: bool,
    pub(super) no_stroke: bool,
    pub(super) stroke_width: f32,
    pub(super) cap: LineCap,
    pub(super) join: LineJoin,
    pub(super) hidden: bool,
    fill_opacity: f32,
    stroke_opacity: f32,
    pub(super) opacity: f32,
    pub(super) blend_mode: BlendMode,
    pub(super) fill_ref: Option<String>,
    pub(super) stroke_ref: Option<String>,
    pub(super) clip_ref: Option<String>,
    pub(super) mask_ref: Option<String>,
    current_color: [u8; 4],
    fill_uses_current: bool,
    stroke_uses_current: bool,
    stroke_first: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill_rule: document_core::FillRule::NonZero,
            font: Default::default(),
            fill: [0, 0, 0, 255],
            stroke: [0, 0, 0, 255],
            no_fill: false,
            no_stroke: true,
            stroke_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            hidden: false,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            fill_ref: None,
            stroke_ref: None,
            clip_ref: None,
            mask_ref: None,
            current_color: [0, 0, 0, 255],
            fill_uses_current: false,
            stroke_uses_current: false,
            stroke_first: false,
        }
    }
}

impl Style {
    pub(super) fn uses_object_bbox(&self) -> bool {
        self.fill_ref.is_some()
            || self.stroke_ref.is_some()
            || self.clip_ref.is_some()
            || self.mask_ref.is_some()
    }
    pub(super) fn fill_opacity(&self) -> f32 {
        self.fill_opacity
    }
    pub(super) fn stroke_opacity(&self) -> f32 {
        self.stroke_opacity
    }
    pub(super) fn fill_rgba(&self) -> [u8; 4] {
        let paint = if self.fill_uses_current {
            self.current_color
        } else {
            self.fill
        };
        with_alpha(paint, self.fill_opacity)
    }

    pub(super) fn stroke_rgba(&self) -> [u8; 4] {
        let paint = if self.stroke_uses_current {
            self.current_color
        } else {
            self.stroke
        };
        with_alpha(paint, self.stroke_opacity)
    }

    /// Apply the element's presentation attributes, with inline CSS taking precedence.
    /// Clone the parent style before calling so siblings inherit independently.
    pub(super) fn apply(
        &mut self,
        attrs: &BTreeMap<String, String>,
        is_container: bool,
    ) -> io::Result<()> {
        self.apply_presentation(attrs)?;
        if is_container && self.opacity != 1.0 {
            return Err(invalid("Unsupported SVG opacity on a root or layer."));
        }
        let both_paints = !self.no_fill
            && !self.no_stroke
            && (self.fill_ref.is_some() || self.fill_rgba()[3] > 0)
            && (self.stroke_ref.is_some() || self.stroke_rgba()[3] > 0)
            && self.stroke_width > 0.0;
        if !is_container && both_paints && self.stroke_first {
            return Err(invalid(
                "SVG stroke-before-fill painting on objects with both paints is unsupported.",
            ));
        }
        Ok(())
    }

    pub(super) fn apply_presentation(
        &mut self,
        attrs: &BTreeMap<String, String>,
    ) -> io::Result<()> {
        let mut properties: BTreeMap<&str, &str> = PRESENTATION
            .iter()
            .filter_map(|&name| attrs.get(name).map(|v| (name, v.as_str())))
            .collect();
        if let Some(css) = attrs.get("style") {
            for declaration in css.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                let (name, value) = declaration
                    .split_once(':')
                    .ok_or_else(|| invalid("Malformed inline SVG style."))?;
                let name = name.trim();
                if !PRESENTATION.contains(&name) {
                    return Err(invalid(format!("Unsupported SVG style property '{name}'.")));
                }
                properties.insert(name, value.trim());
            }
        }
        // Opacity composites an element once. Only explicit CSS inheritance
        // carries the parent's computed opacity into its children.
        if !properties.contains_key("opacity") {
            self.opacity = 1.;
        }
        if !properties.contains_key("mix-blend-mode") {
            self.blend_mode = BlendMode::Normal;
        }
        for (name, raw) in properties {
            let value = if raw.trim() == "initial" {
                initial_value(name)
            } else {
                raw.trim()
            };
            if value == "inherit" {
                continue;
            }
            match (name, value) {
                ("color", _) if value.eq_ignore_ascii_case("currentcolor") => (),
                ("color", _) => self.current_color = color(value)?,
                ("fill", _) => {
                    self.no_fill = value.eq_ignore_ascii_case("none");
                    self.fill_ref = local_reference(value)?;
                    if self.fill_ref.is_some() || self.no_fill {
                        self.fill_uses_current = false;
                        continue;
                    }
                    self.fill_uses_current = value.eq_ignore_ascii_case("currentcolor");
                    if !self.fill_uses_current {
                        self.fill = color(value)?;
                    }
                }
                ("stroke", _) => {
                    self.no_stroke = value.eq_ignore_ascii_case("none");
                    self.stroke_ref = local_reference(value)?;
                    if self.stroke_ref.is_some() || self.no_stroke {
                        self.stroke_uses_current = false;
                        continue;
                    }
                    self.stroke_uses_current = value.eq_ignore_ascii_case("currentcolor");
                    if !self.stroke_uses_current {
                        self.stroke = color(value)?;
                    }
                }
                ("fill-opacity", _) => self.fill_opacity = opacity(value)?,
                ("stroke-opacity", _) => self.stroke_opacity = opacity(value)?,
                ("opacity", _) => self.opacity = opacity(value)?,
                ("mix-blend-mode", _) => {
                    self.blend_mode = BlendMode::ALL
                        .into_iter()
                        .find(|m| m.name() == value)
                        .ok_or_else(|| invalid(format!("Unsupported SVG blend mode '{value}'.")))?;
                }
                ("mask" | "clip-path", _) => {
                    let reference =
                        if value == "none" {
                            None
                        } else {
                            Some(local_reference(value)?.ok_or_else(|| {
                                invalid("Expected a local clip or mask reference.")
                            })?)
                        };
                    if name == "mask" {
                        self.mask_ref = reference;
                    } else {
                        self.clip_ref = reference;
                    }
                }
                ("stroke-width", _) => {
                    self.stroke_width = number(value.strip_suffix("px").unwrap_or(value))?;
                    if self.stroke_width < 0.0 {
                        return Err(invalid("SVG stroke-width must be nonnegative."));
                    }
                }
                ("stroke-linecap", "round") => self.cap = LineCap::Round,
                ("stroke-linecap", "butt") => self.cap = LineCap::Butt,
                ("stroke-linecap", "square") => self.cap = LineCap::Square,
                ("stroke-linejoin", "miter") => self.join = LineJoin::Miter,
                ("stroke-linejoin", "round") => self.join = LineJoin::Round,
                ("stroke-linejoin", "bevel") => self.join = LineJoin::Bevel,
                ("display", "none") => self.hidden = true,
                ("display", "inline")
                | ("visibility", "visible")
                | (
                    "stroke-dasharray" | "vector-effect" | "marker" | "marker-start" | "marker-mid"
                    | "marker-end" | "filter",
                    "none",
                )
                | ("font-variation-settings", "normal")
                | ("font-variant" | "font-stretch", "normal")
                | ("writing-mode", "horizontal-tb" | "lr" | "lr-tb")
                | ("direction", "ltr")
                | ("-inkscape-font-specification", _)
                | ("-inkscape-stroke", "none")
                | ("stop-color", "black" | "#000" | "#000000")
                | ("isolation" | "color-rendering" | "shape-rendering", "auto")
                | ("enable-background", "accumulate")
                | ("overflow", "visible")
                | ("clip-rule", "nonzero" | "evenodd") => (),
                ("fill-rule", "nonzero") => self.fill_rule = document_core::FillRule::NonZero,
                ("fill-rule", "evenodd") => self.fill_rule = document_core::FillRule::EvenOdd,
                ("font-size", _) if is_font_size(value) => self.font.set_size(value),
                ("font-family", _) => self.font.family = value.to_owned(),
                ("font-weight", _) => self.font.weight = value.to_owned(),
                ("font-style", _) => self.font.style = value.to_owned(),
                ("text-anchor", "start") => {
                    self.font.anchor = document_core::typography::TextAlign::Left
                }
                ("text-anchor", "middle") => {
                    self.font.anchor = document_core::typography::TextAlign::Center
                }
                ("text-anchor", "end") => {
                    self.font.anchor = document_core::typography::TextAlign::Right
                }
                ("paint-order", _) => self.stroke_first = stroke_before_fill(value)?,
                ("stroke-miterlimit", _) if number(value)? == 4.0 => (),
                ("stop-opacity", _) if opacity(value)? == 1.0 => (),
                ("stroke-dashoffset", _)
                    if number(value.strip_suffix("px").unwrap_or(value))? == 0.0 => {}
                _ => return Err(invalid(format!("Unsupported SVG style '{name}: {value}'."))),
            }
        }
        Ok(())
    }
}

fn initial_value(name: &str) -> &'static str {
    match name {
        "color" | "fill" | "stop-color" => "black",
        "fill-opacity" | "stroke-opacity" | "opacity" | "stroke-width" | "stop-opacity" => "1",
        "fill-rule" | "clip-rule" => "nonzero",
        "stroke-linecap" => "butt",
        "stroke-linejoin" => "miter",
        "stroke-miterlimit" => "4",
        "stroke-dashoffset" => "0",
        "display" => "inline",
        "visibility" | "overflow" => "visible",
        "paint-order" | "mix-blend-mode" => "normal",
        "enable-background" => "accumulate",
        "isolation" | "color-rendering" | "shape-rendering" => "auto",
        "font-size" => "medium",
        "font-family" => "serif",
        "font-weight"
        | "font-style"
        | "font-variation-settings"
        | "font-variant"
        | "font-stretch" => "normal",
        "writing-mode" => "horizontal-tb",
        "direction" => "ltr",
        "text-anchor" => "start",
        _ => "none",
    }
}

fn is_font_size(value: &str) -> bool {
    if matches!(
        value,
        "xx-small"
            | "x-small"
            | "small"
            | "medium"
            | "large"
            | "x-large"
            | "xx-large"
            | "xxx-large"
            | "smaller"
            | "larger"
    ) {
        return true;
    }
    let numeric = ["rem", "em", "ex", "px", "pt", "pc", "mm", "cm", "in", "%"]
        .iter()
        .find_map(|suffix| value.strip_suffix(suffix))
        .unwrap_or(value);
    number(numeric).is_ok_and(|n| n >= 0.0)
}

/// Unspecified paint operations follow in their normal order. Marker references are
/// rejected independently, so only the order of fill and stroke matters.
fn stroke_before_fill(value: &str) -> io::Result<bool> {
    if value == "normal" {
        return Ok(false);
    }
    let mut order = [3, 4, 5];
    let mut tokens = value.split_ascii_whitespace().peekable();
    if tokens.peek().is_none() {
        return Err(invalid("Empty SVG paint-order."));
    }
    for (position, token) in tokens.enumerate() {
        let slot = match token {
            "fill" => 0,
            "stroke" => 1,
            "markers" => 2,
            _ => return Err(invalid(format!("Invalid SVG paint-order '{value}'."))),
        };
        if order[slot] < 3 {
            return Err(invalid(format!(
                "Repeated SVG paint-order component '{token}'."
            )));
        }
        order[slot] = position;
    }
    Ok(order[1] < order[0])
}

fn number(text: &str) -> io::Result<f32> {
    text.parse::<f32>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid(format!("Invalid numeric SVG style value '{text}'.")))
}

pub(super) fn opacity(text: &str) -> io::Result<f32> {
    let value = if let Some(percent) = text.strip_suffix('%') {
        number(percent)? / 100.0
    } else {
        number(text)?
    };
    Ok(value.clamp(0.0, 1.0))
}

fn with_alpha(mut color: [u8; 4], opacity: f32) -> [u8; 4] {
    color[3] = (f32::from(color[3]) * opacity).round() as u8;
    color
}

pub(super) fn color(text: &str) -> io::Result<[u8; 4]> {
    let lower = text.to_ascii_lowercase();
    let invalid_color = || {
        invalid(format!(
            "Unsupported SVG paint '{text}'. Use a solid color or none."
        ))
    };
    if let Some(hex) = lower.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) || !matches!(hex.len(), 3 | 4 | 6 | 8) {
            return Err(invalid_color());
        }
        let mut rgba = [255; 4];
        let width = if hex.len() <= 4 { 1 } else { 2 };
        for (i, channel) in rgba.iter_mut().enumerate().take(hex.len() / width) {
            let start = i * width;
            let component =
                u8::from_str_radix(&hex[start..start + width], 16).map_err(|_| invalid_color())?;
            *channel = if width == 1 {
                component * 17
            } else {
                component
            };
        }
        return Ok(rgba);
    }
    let function = lower
        .strip_prefix("rgb(")
        .or_else(|| lower.strip_prefix("rgba("));
    if let Some(args) = function.and_then(|s| s.strip_suffix(')')) {
        let components: Vec<_> = if args.contains(',') {
            args.split(',').map(str::trim).collect()
        } else {
            let (values, alpha) = args
                .split_once('/')
                .map_or((args, None), |(v, a)| (v, Some(a.trim())));
            let mut parts = values.split_ascii_whitespace().collect::<Vec<_>>();
            if parts.len() != 3 {
                return Err(invalid_color());
            }
            parts.extend(alpha);
            parts
        };
        if !(3..=4).contains(&components.len()) {
            return Err(invalid_color());
        }
        let mut rgba = [255; 4];
        for (i, &value) in components[..3].iter().enumerate() {
            let channel = if let Some(pct) = value.strip_suffix('%') {
                number(pct)? / 100.0 * 255.0
            } else {
                number(value)?
            };
            rgba[i] = channel.round().clamp(0.0, 255.0) as u8;
        }
        if let Some(value) = components.get(3) {
            rgba[3] = (opacity(value)? * 255.0).round() as u8;
        }
        return Ok(rgba);
    }
    let rgb: [u8; 3] = match lower.as_str() {
        "none" | "transparent" => return Ok([0; 4]),
        "aqua" | "cyan" => [0, 255, 255],
        "black" => [0, 0, 0],
        "blue" => [0, 0, 255],
        "fuchsia" | "magenta" => [255, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "green" => [0, 128, 0],
        "lime" => [0, 255, 0],
        "maroon" => [128, 0, 0],
        "navy" => [0, 0, 128],
        "olive" => [128, 128, 0],
        "orange" => [255, 165, 0],
        "purple" => [128, 0, 128],
        "red" => [255, 0, 0],
        "silver" => [192, 192, 192],
        "teal" => [0, 128, 128],
        "white" => [255, 255, 255],
        "yellow" => [255, 255, 0],
        _ => return Err(invalid_color()),
    };
    Ok([rgb[0], rgb[1], rgb[2], 255])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|&(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    #[test]
    fn applies_cascade_and_inherited_paint_opacity_independently() {
        let mut parent = Style::default();
        parent
            .apply(&attrs(&[("fill-opacity", "50%"), ("stroke", "red")]), true)
            .unwrap();
        let mut child = parent.clone();
        child
            .apply(
                &attrs(&[
                    ("fill", "gray"),
                    (
                        "style",
                        "fill:#3afc;stroke:rgba(50% 0% 100% / 20%);stroke-width:1.5px",
                    ),
                ]),
                false,
            )
            .unwrap();
        assert_eq!(child.fill_rgba(), [51, 170, 255, 102]);
        assert_eq!(child.stroke_rgba(), [128, 0, 255, 51]);
        assert_eq!(child.stroke_width, 1.5);
        assert_eq!(parent.fill_rgba(), [0, 0, 0, 128]);
    }

    #[test]
    fn unsupported_compositing_fails_without_flattening_paints() {
        let mut style = Style::default();
        assert!(style.apply(&attrs(&[("opacity", "0.5")]), true).is_err());
        let mut object = Style::default();
        object
            .apply(
                &attrs(&[("fill", "red"), ("stroke", "blue"), ("opacity", ".2")]),
                false,
            )
            .unwrap();
        assert_eq!(object.opacity, 0.2);
        assert_eq!(object.fill_rgba()[3], 255);
        assert_eq!(object.stroke_rgba()[3], 255);
        assert!(Style::default()
            .apply(&attrs(&[("fill", "url(external.svg#gradient)")]), false)
            .is_err());
    }

    #[test]
    fn consumes_inkscape_defaults_without_accepting_referenced_effects() {
        let mut parent = Style::default();
        parent.apply(&attrs(&[("style", "color:#347755;fill:currentColor;marker:none;marker-start:none;marker-mid:none;marker-end:none;filter:none;font-size:12px;enable-background:accumulate;overflow:visible;paint-order:markers fill stroke")]), true).unwrap();
        let mut child = parent.clone();
        child
            .apply(&attrs(&[("color", "navy"), ("stroke", "red")]), false)
            .unwrap();
        assert_eq!(child.fill_rgba(), [0, 0, 128, 255]);
        assert_eq!(child.stroke_rgba(), [255, 0, 0, 255]);
        assert!(child
            .apply(&attrs(&[("paint-order", "stroke markers")]), false)
            .is_err());
        assert!(parent
            .apply(&attrs(&[("marker-end", "url(#arrow)")]), false)
            .is_err());
        assert!(parent
            .apply(&attrs(&[("filter", "url(#shadow)")]), false)
            .is_err());
    }
}

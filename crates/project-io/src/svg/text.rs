use crate::invalid;
use document_core::typography::{TextAlign, TextLayout, TextStyle, Typeface};
use std::io;
use std::sync::OnceLock;

pub(super) struct Lettering {
    pub layout: TextLayout,
    pub editable: Option<TextStyle>,
}

#[derive(Clone)]
pub(super) struct Font {
    pub family: String,
    pub weight: String,
    pub style: String,
    pub size: Option<f32>,
    pub anchor: TextAlign,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: "serif".into(),
            weight: "normal".into(),
            style: "normal".into(),
            size: Some(16.),
            anchor: TextAlign::Left,
        }
    }
}

impl Font {
    pub fn set_size(&mut self, value: &str) {
        self.size = if value == "medium" {
            Some(16.)
        } else if let Some(em) = value.strip_suffix("em") {
            em.parse::<f32>()
                .ok()
                .zip(self.size)
                .map(|(factor, parent)| factor * parent)
        } else if let Some(percent) = value.strip_suffix('%') {
            percent
                .parse::<f32>()
                .ok()
                .zip(self.size)
                .map(|(factor, parent)| factor * parent / 100.)
        } else {
            super::read::length(value).ok()
        };
    }

    pub fn lettering(&self, content: String) -> io::Result<Lettering> {
        let first = self
            .family
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(['"', '\'']);
        let weight = self.weight.as_str();
        let mut style = TextStyle {
            content,
            size: self
                .size
                .ok_or_else(|| invalid("Unsupported SVG font-size for editable text."))?,
            align: self.anchor,
            ..Default::default()
        };
        let face = match (first.to_ascii_lowercase().as_str(), weight) {
            ("hack" | "hack regular", "normal" | "400") => Some(Typeface::HackRegular),
            ("ubuntu light", "normal" | "300" | "400") | ("ubuntu", "300") => {
                Some(Typeface::UbuntuLight)
            }
            ("hack" | "hack regular" | "ubuntu light" | "ubuntu", _) => {
                return Err(invalid("Unsupported weight for a bundled SVG typeface."));
            }
            _ => None,
        };
        if let Some(face) = face {
            if self.style != "normal" {
                return Err(invalid("The bundled SVG typefaces support upright text."));
            }
            style.face = face;
            return Ok(Lettering {
                layout: style.layout().map_err(invalid)?,
                editable: Some(style),
            });
        }
        let requested = match self.style.as_str() {
            "normal" => fontdb::Style::Normal,
            "italic" => fontdb::Style::Italic,
            "oblique" => fontdb::Style::Oblique,
            _ => return Err(invalid("Unsupported SVG font style.")),
        };
        let weight = match weight {
            "normal" => 400,
            "bold" => 700,
            value => value
                .parse::<u16>()
                .ok()
                .filter(|v| (1..=1000).contains(v))
                .ok_or_else(|| invalid("Unsupported SVG font weight."))?,
        };
        static FONTS: OnceLock<fontdb::Database> = OnceLock::new();
        let fonts = FONTS.get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            db
        });
        let unavailable = || {
            invalid(format!("Install {first} at the requested style and weight, or convert this SVG text to paths."))
        };
        let id = fonts
            .query(&fontdb::Query {
                families: &[fontdb::Family::Name(first)],
                weight: fontdb::Weight(weight),
                style: requested,
                ..Default::default()
            })
            .ok_or_else(unavailable)?;
        if fonts.face(id).is_none_or(|info| info.style != requested) {
            return Err(unavailable());
        }
        let layout = fonts
            .with_face_data(id, |data, index| style.outline_font(data, index, weight))
            .ok_or_else(unavailable)?
            .map_err(invalid)?;
        Ok(Lettering {
            layout,
            editable: None,
        })
    }
}

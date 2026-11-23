use crate::invalid;
use document_core::{RasterImage, Stencil};
use std::{collections::HashMap, io, sync::Arc};

#[derive(Default)]
pub(super) struct Images {
    payloads: HashMap<String, Arc<RasterImage>>,
    pixels: u64,
}

impl Images {
    pub fn read(&mut self, href: &str) -> io::Result<Arc<RasterImage>> {
        let (mime, payload) = href
            .trim()
            .strip_prefix("data:")
            .and_then(|data| data.split_once(";base64,"))
            .filter(|(mime, _)| matches!(*mime, "image/png" | "image/jpeg"))
            .ok_or_else(|| invalid("SVG images must contain an embedded base64 PNG or JPEG."))?;
        let encoded: String = payload
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        let image = if let Some(image) = self.payloads.get(&encoded) {
            Arc::clone(image)
        } else {
            let image = Arc::new(RasterImage::from_base64(&encoded).map_err(invalid)?);
            self.pixels += u64::from(image.width()) * u64::from(image.height());
            if self.pixels > 32 * 1024 * 1024 {
                return Err(invalid(
                    "The embedded images exceed the SVG import pixel limit.",
                ));
            }
            self.payloads.insert(encoded, Arc::clone(&image));
            image
        };
        if image.mime_type() != mime {
            return Err(invalid(
                "The SVG image type differs from its embedded data.",
            ));
        }
        Ok(image)
    }
}

pub(super) struct Placement {
    pub position: [f32; 2],
    pub size: [f32; 2],
    viewport_origin: [f32; 2],
    viewport_size: [f32; 2],
}

impl Placement {
    /// SVG objectBoundingBox includes the letterboxing; Phyro stores the
    /// visible bitmap bounds. Keep the same clip after changing paint space.
    pub fn map_viewport_stencil(&self, stencil: &mut Stencil) {
        if self.size != self.viewport_size {
            for axis in 0..2 {
                stencil.origin[axis] = (self.viewport_origin[axis] - self.position[axis]
                    + stencil.origin[axis] * self.viewport_size[axis])
                    / self.size[axis];
                stencil.size[axis] *= self.viewport_size[axis] / self.size[axis];
            }
        }
    }
}

/// Map the bitmap into its viewport in SVG element coordinates.
pub(super) fn placement(
    image: &RasterImage,
    origin: [f32; 2],
    viewport: [Option<f32>; 2],
    aspect: &str,
) -> io::Result<Placement> {
    let intrinsic = [image.width() as f32, image.height() as f32];
    let [w, h] = match viewport {
        [Some(w), Some(h)] => [w, h],
        [Some(w), None] => [w, w * intrinsic[1] / intrinsic[0]],
        [None, Some(h)] => [h * intrinsic[0] / intrinsic[1], h],
        [None, None] => intrinsic,
    };
    let viewport = Placement {
        position: origin,
        size: [w, h],
        viewport_origin: origin,
        viewport_size: [w, h],
    };
    if aspect.trim() == "none" || w <= 0. || h <= 0. {
        return Ok(viewport);
    }
    let mut tokens = aspect.split_ascii_whitespace();
    let align = tokens.next().unwrap_or("");
    let Some((horizontal, vertical)) = align.strip_prefix('x').and_then(|s| s.split_once('Y'))
    else {
        return Err(invalid("Invalid SVG image aspect alignment."));
    };
    if !matches!(tokens.next(), None | Some("meet")) || tokens.next().is_some() {
        return Err(invalid("Unsupported SVG image aspect ratio mode."));
    }
    let fraction = |v| match v {
        "Min" => Ok(0.),
        "Mid" => Ok(0.5),
        "Max" => Ok(1.),
        _ => Err(invalid("Invalid SVG image aspect alignment.")),
    };
    let scale = (w / intrinsic[0]).min(h / intrinsic[1]);
    let size = intrinsic.map(|v| v * scale);
    Ok(Placement {
        position: [
            origin[0] + (w - size[0]) * fraction(horizontal)?,
            origin[1] + (h - size[1]) * fraction(vertical)?,
        ],
        size,
        ..viewport
    })
}

#[cfg(test)]
mod tests {
    use crate::{export_svg, import_svg};
    use std::{fs, sync::Arc};

    #[test]
    fn embedded_pngs_share_pixels_and_reopen_with_their_placement_and_opacity() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEUlEQVR4nGP4z8DwnyHqfxQAEGIDsiRLubIAAAAASUVORK5CYII=";
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("images.svg");
        let svg = format!(
            r#"<svg width="20" height="12" xmlns:xlink="http://www.w3.org/1999/xlink">
            <defs><clipPath id="clip" clipPathUnits="objectBoundingBox"><rect width="1" height=".4"/></clipPath></defs>
            <g transform="translate(2 1)"><image id="panel" xlink:href="data:image/png;base64,{png}" width="10" height="10" opacity=".5" clip-path="url(#clip)"/></g>
            <image href="data:image/png;base64,{png}" x="18" y="0" width="2" height="5" preserveAspectRatio="none"/></svg>"#
        );
        fs::write(&source, svg).unwrap();
        let original = import_svg(&source).unwrap();
        let [a, b] = original.objects.as_slice() else {
            panic!()
        };
        assert!(Arc::ptr_eq(
            a.image.as_ref().unwrap(),
            b.image.as_ref().unwrap()
        ));
        assert_eq!(
            (a.name.as_str(), a.position, a.size, a.opacity),
            ("panel", [2., 3.5], [10., 5.], 0.5)
        );
        assert_eq!(b.size, [2., 5.]);
        let image = renderer::render(&original, 0).unwrap();
        assert_eq!(image.pixel(3, 1).unwrap().alpha(), 0);
        assert!(image.pixel(3, 4).unwrap().red() > 110);
        assert_eq!(image.pixel(3, 5).unwrap().alpha(), 0);
        let dest = dir.path().join("written.svg");
        export_svg(&dest, &original, 0, false).unwrap();
        let imported = import_svg(&dest).unwrap();
        assert_eq!(renderer::render(&imported, 0).unwrap().data(), image.data());
    }
}

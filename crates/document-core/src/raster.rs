use base64::{engine::general_purpose::STANDARD, Engine};
use image::{
    codecs::{jpeg::JpegDecoder, png::PngEncoder},
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder,
};
use std::{borrow::Cow, fmt, io::Cursor};
use tiny_skia::{IntSize, Pixmap};

/// An immutable embedded image with its original encoding and render-ready pixels.
/// The native image table shares this allocation across drawings.
pub struct RasterImage {
    encoded: Box<str>,
    mime: &'static str,
    pixels: Pixmap,
    has_ink: bool,
}

impl RasterImage {
    pub const MAX_FILE_BYTES: usize = 12 * 1024 * 1024;

    pub fn from_base64(source: &str) -> Result<Self, String> {
        if source.len() > Self::MAX_FILE_BYTES / 3 * 4 {
            return Err("An embedded image exceeds the 16 MiB base64 limit.".into());
        }
        let encoded = if source.bytes().any(|b| b.is_ascii_whitespace()) {
            Cow::Owned(
                source
                    .chars()
                    .filter(|c| !c.is_ascii_whitespace())
                    .collect(),
            )
        } else {
            Cow::Borrowed(source)
        };
        let bytes = STANDARD
            .decode(encoded.as_bytes())
            .map_err(|_| "Invalid base64 in embedded image.")?;
        Self::decode(&bytes, encoded.into_owned())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > Self::MAX_FILE_BYTES {
            return Err("An embedded image must be 12 MiB or smaller.".into());
        }
        Self::decode(bytes, STANDARD.encode(bytes))
    }

    /// Capture straight-alpha clipboard pixels as a portable PNG. Keep the
    /// original RGBA values in the file and premultiply only the render cache.
    pub fn from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Result<Self, String> {
        check_dimensions(width, height)?;
        if rgba.len() != width as usize * height as usize * 4 {
            return Err("Invalid clipboard pixel buffer.".into());
        }
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
            .map_err(|e| format!("Could not encode the clipboard image: {e}"))?;
        if png.len() > Self::MAX_FILE_BYTES {
            return Err("The pasted image exceeds the 12 MiB embedded image limit.".into());
        }
        let mut has_ink = false;
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let alpha = u16::from(pixel[3]);
            has_ink |= alpha != 0;
            if alpha < 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                }
            }
        }
        let size = IntSize::from_wh(width, height).ok_or("Empty clipboard image.")?;
        Ok(Self {
            encoded: STANDARD.encode(png).into_boxed_str(),
            mime: "image/png",
            pixels: Pixmap::from_vec(rgba, size).ok_or("Invalid clipboard dimensions.")?,
            has_ink,
        })
    }

    fn decode(bytes: &[u8], encoded: String) -> Result<Self, String> {
        let (mime, pixels, has_ink) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            // Check dimensions before the PNG decoder allocates its pixel buffer.
            let header = bytes.get(8..24).ok_or("Incomplete embedded PNG header.")?;
            if &header[..8] != b"\0\0\0\rIHDR" {
                return Err("Invalid embedded PNG header.".into());
            }
            let width = u32::from_be_bytes(header[8..12].try_into().unwrap());
            let height = u32::from_be_bytes(header[12..16].try_into().unwrap());
            check_dimensions(width, height)?;
            let pixels = Pixmap::decode_png(bytes).map_err(|e| format!("Invalid PNG: {e}"))?;
            let ink = pixels.pixels().iter().any(|p| p.alpha() != 0);
            ("image/png", pixels, ink)
        } else if bytes.starts_with(&[0xff, 0xd8]) {
            let message = |e| format!("Invalid JPEG: {e}");
            let mut decoder = JpegDecoder::new(Cursor::new(bytes)).map_err(message)?;
            let (w, h) = decoder.dimensions();
            check_dimensions(w, h)?;
            let orientation = decoder.orientation().map_err(message)?;
            let mut image = DynamicImage::from_decoder(decoder).map_err(message)?;
            image.apply_orientation(orientation);
            let rgba = image.into_rgba8(); // JPEG is opaque, so RGB is already premultiplied.
            let size = IntSize::from_wh(rgba.width(), rgba.height()).ok_or("Empty JPEG.")?;
            let pixels = Pixmap::from_vec(rgba.into_raw(), size).ok_or("Invalid JPEG size.")?;
            ("image/jpeg", pixels, true)
        } else {
            return Err("Choose a PNG or JPEG image.".into());
        };
        Ok(Self {
            encoded: encoded.into_boxed_str(),
            mime,
            pixels,
            has_ink,
        })
    }
    pub fn encoded_base64(&self) -> &str {
        &self.encoded
    }
    pub fn mime_type(&self) -> &'static str {
        self.mime
    }
    pub fn width(&self) -> u32 {
        self.pixels.width()
    }
    pub fn height(&self) -> u32 {
        self.pixels.height()
    }
    pub fn rgba(&self) -> &[u8] {
        self.pixels.data()
    }
    pub(crate) fn has_ink(&self) -> bool {
        self.has_ink
    }
    pub(crate) fn visible_at(&self, uv: [f32; 2]) -> bool {
        if !uv.into_iter().all(|v| (0.0..=1.0).contains(&v)) {
            return false;
        }
        let x = ((uv[0] * self.width() as f32) as u32).min(self.width() - 1);
        let y = ((uv[1] * self.height() as f32) as u32).min(self.height() - 1);
        self.pixels.pixel(x, y).is_some_and(|p| p.alpha() != 0)
    }
}

fn check_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16 * 1024 * 1024 {
        return Err("An embedded image needs 1–16,777,216 pixels.".into());
    }
    Ok(())
}

impl PartialEq for RasterImage {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other) || self.encoded == other.encoded
    }
}
impl Eq for RasterImage {}

impl fmt::Debug for RasterImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RasterImage")
            .field("width", &self.width())
            .field("height", &self.height())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;

    #[test]
    fn camera_orientation_survives_native_encoding_with_original_jpeg_bytes() {
        let mut jpg = Vec::new();
        let mut encoder = JpegEncoder::new(&mut jpg);
        // TIFF orientation 6 rotates a landscape capture 90 degrees clockwise.
        let exif = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        encoder.set_exif_metadata(exif.to_vec()).unwrap();
        encoder
            .write_image(&[0; 18], 3, 2, ExtendedColorType::Rgb8)
            .unwrap();
        let image = RasterImage::from_bytes(&jpg).unwrap();
        assert_eq!(
            (image.width(), image.height(), image.mime_type()),
            (2, 3, "image/jpeg")
        );
        assert_eq!(STANDARD.decode(image.encoded_base64()).unwrap(), jpg);
        assert_eq!(
            RasterImage::from_base64(image.encoded_base64())
                .unwrap()
                .rgba(),
            image.rgba()
        );
    }

    #[test]
    fn clipboard_png_reopens_with_the_same_alpha_and_checks_size_before_encoding() {
        let image = RasterImage::from_rgba(2, 1, vec![190, 34, 5, 0, 84, 185, 14, 140]).unwrap();
        let restored = RasterImage::from_base64(image.encoded_base64()).unwrap();
        assert_eq!(image.rgba(), restored.rgba());
        assert!(!restored.visible_at([0.1, 0.5]));
        assert!(restored.visible_at([0.9, 0.5]));
        assert!(RasterImage::from_rgba(1, 1, vec![1; 3]).is_err());
        assert!(RasterImage::from_rgba(65_536, 65_536, Vec::new()).is_err());
    }
}

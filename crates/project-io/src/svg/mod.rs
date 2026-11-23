mod bitmap;
mod defs;
mod defs_read;
mod pathdata;
mod pressure_paint;
mod read;
mod rectangular;
mod style;
#[cfg(test)]
mod tests;
mod text;
mod transform;
mod write;

pub use read::import_svg;
pub(crate) use write::encode_objects;
pub use write::export_svg;

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const INKSCAPE_NS: &str = "http://www.inkscape.org/namespaces/inkscape";

// Older SVGs carry the same editor metadata under the original product name.
fn canonical_attribute(name: String) -> String {
    if let Some(suffix) = name.strip_prefix("data-loopline-") {
        format!("data-phyro-{suffix}")
    } else {
        name
    }
}

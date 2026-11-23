use crate::invalid;
use document_core::RasterImage;
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    sync::Arc,
};

/// Read an embedded reference by its signature, including JPEG camera orientation.
pub fn import_reference_image(path: &Path) -> io::Result<Arc<RasterImage>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(RasterImage::MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    RasterImage::from_bytes(&bytes)
        .map(Arc::new)
        .map_err(invalid)
}

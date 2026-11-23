use crate::invalid;
use document_core::{Document, RasterImage};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};

fn count_pixels(pixels: &mut u64, image: &RasterImage) -> io::Result<()> {
    *pixels += u64::from(image.width()) * u64::from(image.height());
    if *pixels > 64 * 1024 * 1024 {
        return Err(invalid(
            "Embedded images exceed the 64-megapixel project limit.",
        ));
    }
    Ok(())
}

/// Keep one encoded image in the archive and one shared pixel buffer on reload,
/// including when multiple frame-by-frame drawings contain the same image.
pub(super) fn pack<'a>(
    drawings: impl Iterator<Item = &'a mut Arc<Document>>,
) -> io::Result<BTreeMap<u64, String>> {
    let mut images = BTreeMap::new();
    let mut ids = HashMap::new();
    let mut pixels = 0;
    for drawing in drawings {
        if !drawing.objects.iter().any(|o| o.image.is_some()) {
            continue;
        }
        for object in &mut Arc::make_mut(drawing).objects {
            if let Some(image) = object.image.take() {
                let key = Arc::as_ptr(&image);
                let id = if let Some(id) = ids.get(&key) {
                    *id
                } else {
                    count_pixels(&mut pixels, &image)?;
                    let id = images.len() as u64 + 1;
                    images.insert(id, image.encoded_base64().to_owned());
                    ids.insert(key, id);
                    id
                };
                object.image_resource = Some(id);
            }
        }
    }
    Ok(images)
}

pub(super) fn unpack<'a>(
    mut encoded: BTreeMap<u64, String>,
    drawings: impl Iterator<Item = &'a mut Arc<Document>>,
) -> io::Result<()> {
    let mut decoded = HashMap::new();
    let mut pixels = 0_u64;
    for drawing in drawings {
        if !drawing.objects.iter().any(|o| o.image_resource.is_some()) {
            continue;
        }
        for object in &mut Arc::make_mut(drawing).objects {
            let Some(id) = object.image_resource.take() else {
                continue;
            };
            if let std::collections::hash_map::Entry::Vacant(entry) = decoded.entry(id) {
                let source = encoded
                    .remove(&id)
                    .filter(|_| id != 0)
                    .ok_or_else(|| invalid("Missing embedded image."))?;
                let image = RasterImage::from_base64(&source).map_err(invalid)?;
                count_pixels(&mut pixels, &image)?;
                entry.insert(Arc::new(image));
            }
            object.image = Some(Arc::clone(&decoded[&id]));
        }
    }
    if !encoded.is_empty() {
        return Err(invalid("Unreferenced embedded image."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{load_animation, load_project, save_animation, save_project};
    use document_core::AnimationProject;

    #[test]
    fn duplicated_images_roundtrip_with_one_resource_and_transparent_hit_testing() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEUlEQVR4nGNgYGBg4K/efgkAA/YCFEgjF+4AAAAASUVORK5CYII=";
        let mut doc = Document::default();
        doc.add_rectangle([4., 8.], [40., 20.], [255; 4]);
        doc.objects[0].image = Some(Arc::new(RasterImage::from_base64(PNG).unwrap()));
        let mut project = AnimationProject::from_document(doc.clone());
        project.clips[0].duplicate_frame(0).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("images.vanim");
        save_animation(&path, &project, false).unwrap();
        let (version, payload) = super::super::read_archive(&path, 20).unwrap();
        assert_eq!(version, 20);
        assert_eq!(
            std::str::from_utf8(&payload).unwrap().matches(PNG).count(),
            1
        );
        let loaded = load_animation(&path).unwrap();
        assert_eq!(loaded, project);
        let clip = &loaded.clips[0];
        let image = |d: &Document| Arc::clone(d.objects[0].image.as_ref().unwrap());
        assert!(Arc::ptr_eq(
            &image(clip.drawing_at(0)),
            &image(clip.drawing_at(1))
        ));
        let still = dir.path().join("reference.vanim");
        save_project(&still, &doc).unwrap();
        let reference = load_project(&still).unwrap();
        assert_eq!(reference.hit_test([8., 12.], 0), None);
        assert_eq!(reference.hit_test([37., 12.], 0), Some(doc.objects[0].id));
        assert_eq!(reference, doc);
        assert!(super::super::decode_animation(19, &payload).is_err());
    }
}

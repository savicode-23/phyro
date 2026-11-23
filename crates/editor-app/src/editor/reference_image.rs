use super::{commands, diagnostics, Action, Editor, FileDialog, Tool};
use document_core::{Document, RasterImage, Shape};
use std::{io, path::Path, sync::Arc};

#[derive(Clone, Copy)]
enum Destination {
    ReferenceLayer,
    ActiveLayer,
}

impl Editor {
    pub(super) fn choose_reference_image(&mut self) {
        self.timeline.stop();
        let path = diagnostics::native_dialog(|| {
            FileDialog::new()
                .set_title("Import Reference Image")
                .add_filter("PNG or JPEG image", &["png", "jpg", "jpeg"])
                .pick_file()
        });
        if let Some(path) = path {
            self.import_reference_from(&path);
        }
    }

    pub(super) fn import_reference_from(&mut self, path: &Path) {
        self.note(Action::Open);
        self.timeline.stop();
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        match project_io::import_reference_image(path)
            .and_then(|image| self.place_image(image, &name, Destination::ReferenceLayer))
        {
            Ok(_) => {
                let layer = &self.scene().layers[0].name;
                self.status =
                    format!("Reference added to {layer}. Unlock the layer to move or resize it.");
            }
            Err(error) => self.report_error(
                "Could not import reference image",
                format!("{}\n{error}", path.display()),
            ),
        }
    }

    pub(crate) fn paste_raster_image(&mut self, image: Arc<RasterImage>) -> io::Result<()> {
        let id = self.place_image(image, "Pasted image", Destination::ActiveLayer)?;
        self.selection = vec![id];
        self.set_tool(Tool::Select);
        Ok(())
    }

    fn place_image(
        &mut self,
        image: Arc<RasterImage>,
        name: &str,
        destination: Destination,
    ) -> io::Result<u64> {
        if matches!(destination, Destination::ActiveLayer)
            && !commands::is_editable(self.scene(), self.style.layer)
        {
            return Err(io::Error::other(
                "Choose a visible, unlocked layer to paste the image.",
            ));
        }
        self.timeline.stop();
        self.finish_edit();
        let mut next = self.project.clone();
        let clip = &mut next.clips[self.timeline.clip];
        let layer = match destination {
            Destination::ReferenceLayer => {
                let id = clip.next_layer_id();
                let next_object = clip.next_shared_object_id().map_err(io::Error::other)?;
                let drawing = clip.drawing_at_mut(self.timeline.frame);
                drawing.next_id = next_object;
                reference_layer(drawing, id)
            }
            Destination::ActiveLayer => self.style.layer,
        };
        let drawing = clip.drawing_at_mut(self.timeline.frame);
        let id = insert(drawing, image, name, layer, destination);
        clip.sync_layer_edits(&self.project.clips[self.timeline.clip], self.timeline.frame)
            .map_err(io::Error::other)?;
        project_io::validate_image_import(&next)?;
        self.project = next;
        self.record_synced_edit(Ok(()));
        Ok(id)
    }
}

fn reference_layer(doc: &mut Document, id: u64) -> u64 {
    // Reuse the backmost reference layer when possible. A layer that someone
    // hid, unlocked, or repurposed for ordinary artwork keeps its settings.
    let existing = doc.layers.first().is_some_and(|layer| {
        let mut objects = doc
            .objects
            .iter()
            .filter(|o| o.layer == layer.id)
            .peekable();
        layer.locked
            && layer.visible
            && objects.peek().is_some()
            && objects.all(|o| o.reference_only)
    });
    if !existing {
        let mut name = String::from("Reference");
        let mut suffix = 2;
        while doc.layers.iter().any(|l| l.name == name) {
            name = format!("Reference {suffix}");
            suffix += 1;
        }
        commands::add_layer_with_id(doc, id);
        doc.layers.rotate_right(1);
        doc.layers[0].name = name;
        doc.layers[0].locked = true;
    }
    doc.layers[0].id
}

fn insert(
    doc: &mut Document,
    image: Arc<RasterImage>,
    name: &str,
    layer: u64,
    destination: Destination,
) -> u64 {
    let image = doc
        .objects
        .iter()
        .filter_map(|o| o.image.as_ref())
        .find(|old| old.as_ref() == image.as_ref())
        .cloned()
        .unwrap_or(image);
    let width = image.width() as f32;
    let height = image.height() as f32;
    let page = [doc.width as f32, doc.height as f32];
    let scale = 1_f32.min(page[0] / width).min(page[1] / height);
    let size = [width * scale, height * scale];
    let position = [(page[0] - size[0]) * 0.5, (page[1] - size[1]) * 0.5];
    let id = doc.add_shape(position, size, [0; 4], Shape::Rectangle, layer);
    let object = doc.objects.last_mut().expect("add_shape appends an object");
    object.image = Some(image);
    let clean_name: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
    object.name = if clean_name.trim().is_empty() {
        String::from("Reference image")
    } else {
        clean_name
    };
    object.reference_only = matches!(destination, Destination::ReferenceLayer);
    id
}

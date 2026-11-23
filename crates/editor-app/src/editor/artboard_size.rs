use super::Editor;
use eframe::egui;

impl Editor {
    pub(super) fn open_artboard_size(&mut self) {
        self.timeline.stop();
        self.artboard_size = Some([self.scene().width, self.scene().height]);
    }

    pub(super) fn artboard_size_dialog(&mut self, ctx: &egui::Context) {
        let Some(size) = self.artboard_size.as_mut() else {
            return;
        };
        let response = egui::Modal::new("artboard-size".into()).show(ctx, |ui| {
            ui.set_width(260.);
            ui.heading("Artboard size");
            ui.label("Applies to all clips. Artwork keeps its position and scale.");
            egui::Grid::new("artboard-dimensions").show(ui, |ui| {
                for (label, value) in ["Width", "Height"].into_iter().zip(size.iter_mut()) {
                    let text = ui.label(label);
                    ui.add(
                        egui::DragValue::new(value)
                            .range(1..=document_core::MAX_DIMENSION)
                            .suffix(" px"),
                    )
                    .labelled_by(text.id);
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| (ui.button("Apply").clicked(), ui.button("Cancel").clicked()))
                .inner
        });
        let (apply, cancel) = response.inner;
        if apply {
            let size = *size;
            self.artboard_size = None;
            self.finish_edit();
            match self.history.resize_artboard(
                &mut self.project,
                self.timeline.clip,
                self.timeline.frame,
                size,
            ) {
                Ok(()) => {
                    self.capture_baseline();
                    self.status = format!("Artboard set to {} × {} px.", size[0], size[1]);
                }
                Err(error) => self.report_error("Could not resize artboard", error),
            }
        } else if cancel || response.should_close() {
            self.artboard_size = None;
        }
    }
}

use document_core::ExportPreset;
use eframe::egui;
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

/// Width, height and percentage edit one ratio, preserving the artwork's proportions.
pub(super) fn controls(ui: &mut egui::Ui, preset: &mut ExportPreset, native: [u32; 2]) {
    ui.horizontal_wrapped(|ui| {
        for (axis, name) in ["Width", "Height"].into_iter().enumerate() {
            let units = native[axis].max(1);
            let mut pixels = (f64::from(units) * preset.raster_scale()).round().max(1.) as u32;
            let label = ui.label(name);
            if ui
                .add(
                    egui::DragValue::new(&mut pixels)
                        .range(1..=units.saturating_mul(32))
                        .suffix(" px"),
                )
                .labelled_by(label.id)
                .changed()
            {
                preset.set_raster_ratio(pixels, units);
            }
        }
    });
    scale_control(ui, preset);
}

pub(super) fn scale_control(ui: &mut egui::Ui, preset: &mut ExportPreset) {
    ui.horizontal_wrapped(|ui| {
        let label = ui.label("Scale");
        let mut percent = preset.raster_scale() * 100.;
        if ui
            .add(
                egui::DragValue::new(&mut percent)
                    .speed(0.5)
                    .range(0.001..=3200.)
                    .max_decimals(3)
                    .suffix("%"),
            )
            .labelled_by(label.id)
            .changed()
        {
            preset.set_raster_ratio((percent * 1000.).round() as u32, 100_000);
        }
        if ui
            .small_button("100%")
            .on_hover_text("Use the original pixel size")
            .clicked()
        {
            preset.set_raster_ratio(1, 1);
        }
    });
}

/// A dialog measures one snapshot at a time. Superseded poses stop between render calls.
pub(super) struct Measurement<K, T> {
    key: Option<K>,
    pub result: Option<Result<T, String>>,
    worker: Option<JoinHandle<io::Result<T>>>,
    cancel: Arc<AtomicBool>,
}

impl<K, T> Default for Measurement<K, T> {
    fn default() -> Self {
        Self {
            key: None,
            result: None,
            worker: None,
            cancel: Arc::default(),
        }
    }
}

impl<K: PartialEq, T: Send + 'static> Measurement<K, T> {
    pub fn update<F>(&mut self, ctx: &egui::Context, key: K, build: impl FnOnce() -> F)
    where
        F: FnOnce(Arc<AtomicBool>) -> io::Result<T> + Send + 'static,
    {
        if self.key.as_ref() != Some(&key) {
            self.result = None;
            self.cancel.store(true, Ordering::Relaxed);
        }
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            let result = self
                .worker
                .take()
                .unwrap()
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("Could not measure PNG dimensions.")));
            if !self.cancel.load(Ordering::Relaxed) {
                self.result = Some(result.map_err(|e| e.to_string()));
            }
        }
        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if self.result.is_some() {
            return;
        }
        self.key = Some(key);
        self.cancel = Arc::default();
        let cancel = self.cancel.clone();
        let ctx = ctx.clone();
        let work = build();
        match std::thread::Builder::new()
            .name("png-dimensions".into())
            .spawn(move || {
                let result = work(cancel);
                ctx.request_repaint();
                result
            }) {
            Ok(worker) => self.worker = Some(worker),
            Err(e) => self.result = Some(Err(e.to_string())),
        }
    }
}

impl<K, T> Drop for Measurement<K, T> {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

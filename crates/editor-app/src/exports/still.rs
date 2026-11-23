use super::{choose_folder, Completed, Exports};
use document_core::{Clip, ExportPreset};
use eframe::egui;
use project_io::{StillFormat, StillOptions, StillTarget};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Artboard,
    Selection,
    Batch,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Self::Artboard => "Artboard",
            Self::Selection => "Selection",
            Self::Batch => "Batch selected objects",
        }
    }

    fn stem(self) -> &'static str {
        match self {
            Self::Artboard => "artboard",
            Self::Selection => "selection",
            Self::Batch => "objects",
        }
    }
}

pub(super) fn label(format: StillFormat) -> &'static str {
    match format {
        StillFormat::Png => "PNG",
        StillFormat::Svg => "SVG",
        StillFormat::PngAndSvg => "PNG and SVG",
    }
}

#[derive(Clone)]
pub(super) struct Image {
    clip: Clip,
    scope: Scope,
    frame: u32,
    ids: Vec<u64>,
    scale: f64,
}

impl Image {
    pub(super) fn new(
        clip: &Clip,
        frame: u32,
        scope: Scope,
        ids: &[u64],
        scale: impl Into<f64>,
    ) -> Self {
        Self {
            clip: clip.clone(),
            frame,
            scope,
            ids: ids.to_vec(),
            scale: scale.into(),
        }
    }
}

impl Image {
    fn target(&self) -> StillTarget<'_> {
        match self.scope {
            Scope::Artboard => StillTarget::Artboard,
            Scope::Selection => StillTarget::Selection(&self.ids),
            Scope::Batch => StillTarget::Objects(&self.ids),
        }
    }
}

pub(super) struct Settings {
    image: Image,
    format: StillFormat,
    preset: ExportPreset,
    native: Option<[u32; 2]>,
    preview: super::resolution::Measurement<f64, project_io::PngDimensions>,
}

impl Settings {
    fn new(image: Image, format: StillFormat, preset: ExportPreset) -> Self {
        let doc = image.clip.drawing_at(image.frame);
        let native = (image.scope != Scope::Selection).then_some([doc.width, doc.height]);
        Self {
            image,
            format,
            preset,
            native,
            preview: Default::default(),
        }
    }

    fn update(&mut self, ctx: &egui::Context) {
        let scale = self.preset.raster_scale();
        if self.image.scope == Scope::Batch {
            self.preview.update(ctx, scale, || {
                let image = self.image.clone();
                move |cancel| {
                    project_io::png_dimensions(
                        &image.clip,
                        image.frame,
                        image.target(),
                        scale,
                        || !cancel.load(Ordering::Relaxed),
                    )
                }
            });
            return;
        }
        if self.native.is_none() {
            self.preview.update(ctx, 1., || {
                let image = self.image.clone();
                move |_| {
                    let native = project_io::selection_png_native_size(
                        &image.clip,
                        image.frame,
                        &image.ids,
                    )?;
                    Ok(project_io::PngDimensions {
                        native,
                        assets: Vec::new(),
                    })
                }
            });
            self.native = self
                .preview
                .result
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(|r| r.native);
        }
        if let Some(native) = self.native {
            self.preview.result =
                Some(
                    self.preset
                        .raster_size(native)
                        .map(|size| project_io::PngDimensions {
                            native,
                            assets: vec![(self.image.scope.label().into(), size)],
                        }),
                );
        }
    }
}

impl Exports {
    pub(super) fn still_menu(
        &mut self,
        ui: &mut egui::Ui,
        clip: &Clip,
        frame: u32,
        selected: &[u64],
        preset: ExportPreset,
    ) -> Option<String> {
        let mut action = None;
        for scope in [Scope::Artboard, Scope::Selection, Scope::Batch] {
            ui.add_enabled_ui(scope == Scope::Artboard || !selected.is_empty(), |ui| {
                ui.menu_button(scope.label(), |ui| {
                    for format in [StillFormat::Png, StillFormat::Svg, StillFormat::PngAndSvg] {
                        if ui.button(format!("{}…", label(format))).clicked() {
                            action = Some((scope, format));
                            ui.close_menu();
                        }
                    }
                });
            });
        }
        let (scope, format) = action?;
        let image = Image::new(clip, frame, scope, selected, preset.raster_scale());
        self.destination_error = None;
        if format != StillFormat::Svg {
            self.still_export = Some(Settings::new(image, format, preset));
            return None;
        }
        self.choose_still(image, format, ui.ctx())
    }

    fn choose_still(
        &mut self,
        image: Image,
        format: StillFormat,
        ctx: &egui::Context,
    ) -> Option<String> {
        let scope = image.scope;
        let destination = if scope == Scope::Batch || format == StillFormat::PngAndSvg {
            choose_folder(scope.stem()).map(|path| path.map(|p| (p, false)))
        } else {
            choose_file(
                scope,
                if format == StillFormat::Png {
                    "png"
                } else {
                    "svg"
                },
            )
        };
        match destination {
            Ok(Some((path, replace))) => {
                self.still_export = None;
                self.write_still(image, path, format, replace, ctx)
            }
            Ok(None) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    pub(super) fn still_window(&mut self, ctx: &egui::Context) -> Option<String> {
        let state = self.still_export.as_mut()?;
        let modal = egui::Modal::new("still-png-resolution".into()).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.).clamp(180., 350.));
            ui.heading(format!("Export {}", state.image.scope.label()));
            if state.image.scope == Scope::Batch {
                ui.label("Artboard at export scale");
                ui.weak("Each object uses the same scale and its own crop.");
            } else {
                ui.label(if state.image.scope == Scope::Selection {
                    "Selection PNG resolution"
                } else {
                    "PNG resolution"
                });
            }
            if let Some(native) = state.native {
                super::resolution::controls(ui, &mut state.preset, native);
            } else {
                ui.add_sized(
                    [ui.available_width(), ui.spacing().interact_size.y],
                    egui::Label::new("Measuring selection bounds…"),
                );
                super::resolution::scale_control(ui, &mut state.preset);
            }
            state.update(ctx);
            if state.format == StillFormat::PngAndSvg {
                ui.weak("SVG keeps its size in document units.");
            }
            let height = if state.image.scope == Scope::Batch {
                108.
            } else {
                ui.spacing().interact_size.y * 2.
            };
            let ready = ui
                .scope(|ui| {
                    // Reserve room as measurements finish so the destination button stays in place.
                    ui.set_min_height(height);
                    match &state.preview.result {
                        Some(Ok(result)) => {
                            egui::ScrollArea::vertical().max_height(height).show_rows(
                                ui,
                                ui.text_style_height(&egui::TextStyle::Body),
                                result.assets.len(),
                                |ui, rows| {
                                    for (name, [w, h]) in &result.assets[rows] {
                                        let text = format!("{name} · {w} × {h} px");
                                        ui.add(egui::Label::new(&text).truncate())
                                            .on_hover_text(text);
                                    }
                                },
                            );
                            true
                        }
                        Some(Err(error)) => {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                            false
                        }
                        None => {
                            ui.label("Measuring PNG dimensions…");
                            false
                        }
                    }
                })
                .inner;
            ui.separator();
            if let Some(error) = &self.destination_error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                (
                    ui.add_enabled(ready, egui::Button::new("Choose destination…"))
                        .clicked(),
                    ui.button("Cancel").clicked(),
                )
            })
            .inner
        });
        let (export, close) = modal.inner;
        if export {
            let mut image = state.image.clone();
            image.scale = state.preset.raster_scale();
            let format = state.format;
            self.destination_error = None;
            let error = self.choose_still(image, format, ctx);
            if self.still_export.is_none() {
                return error;
            }
            self.destination_error =
                error.map(|error| format!("Could not choose an export destination. {error}"));
        }
        if close || modal.should_close() {
            self.still_export = None;
        }
        None
    }

    pub(super) fn write_still(
        &mut self,
        image: Image,
        path: PathBuf,
        format: StillFormat,
        replace: bool,
        ctx: &egui::Context,
    ) -> Option<String> {
        let multiple = image.scope == Scope::Batch || matches!(format, StillFormat::PngAndSvg);
        self.spawn(1, ctx, move |progress| {
            let target = image.target();
            let options = StillOptions {
                format,
                scale: image.scale,
                replace,
            };
            let files = project_io::export_still(
                &path,
                &image.clip,
                image.frame,
                target,
                options,
                |done, total| {
                    progress.total.store(total, Ordering::Relaxed);
                    progress.done.store(done, Ordering::Relaxed);
                    !progress.cancel.load(Ordering::Relaxed)
                },
            )?;
            Ok(if multiple {
                Completed::Objects(path, files.len(), format)
            } else {
                Completed::Frame(path)
            })
        })
    }
}

fn choose_file(scope: Scope, extension: &str) -> io::Result<Option<(PathBuf, bool)>> {
    let Some(mut path) = crate::diagnostics::native_dialog(|| {
        FileDialog::new()
            .set_title(format!("Export {}", scope.stem()))
            .add_filter(extension.to_uppercase(), &[extension])
            .set_file_name(format!("{}.{extension}", scope.stem()))
            .save_file()
    }) else {
        return Ok(None);
    };
    path.set_extension(extension);
    let replace = match std::fs::symlink_metadata(&path) {
        Ok(_) => {
            let confirmed = crate::diagnostics::native_dialog(|| {
                MessageDialog::new()
                    .set_title(format!("Replace {}?", extension.to_uppercase()))
                    .set_description(format!("Replace {}?", path.display()))
                    .set_buttons(MessageButtons::YesNo)
                    .show()
            });
            if confirmed != MessageDialogResult::Yes {
                return Ok(None);
            }
            true
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(e),
    };
    Ok(Some((path, replace)))
}

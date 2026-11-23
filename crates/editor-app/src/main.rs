#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod canvas;
mod color_picker;
mod commands;
mod composition_panel;
mod crash_report_ui;
mod curve_editor;
mod diagnostics;
mod editor;
mod exports;
mod inspector;
mod keymap;
mod library;
mod path_editor;
mod path_ops;
mod profile;
mod recovery;
mod rig_panel;
mod theme;
mod timeline;
mod tool_keys;
mod type_editor;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let diagnostics = diagnostics::Diagnostics::install(env!("PHYRO_BUILD"));
    let editor_diagnostics = diagnostics.clone();
    let paths = std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/phyro-icon.png"))
                    .expect("valid embedded Phyro icon"),
            )
            .with_drag_and_drop(true)
            .with_inner_size([1100.0, 740.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "Phyro",
        options,
        Box::new(move |cc| {
            if let (Some(reports), Some(gl)) = (&editor_diagnostics, &cc.gl) {
                use eframe::glow::{self, HasContext};
                // eframe calls the creator on the thread owning the current GL context.
                unsafe {
                    reports.graphics(
                        &gl.get_parameter_string(glow::VENDOR),
                        &gl.get_parameter_string(glow::RENDERER),
                        &gl.get_parameter_string(glow::VERSION),
                    );
                }
            }
            let mut editor = editor::Editor::new_native(&cc.egui_ctx);
            if let Some(gl) = &cc.gl {
                editor.install_canvas_filter(&cc.egui_ctx, gl);
            }
            editor.attach_diagnostics(editor_diagnostics);
            editor.open_startup_paths(&cc.egui_ctx, paths);
            Ok(Box::new(editor))
        }),
    );
    if let Err(error) = &result {
        startup_error(diagnostics.as_ref(), error);
    }
    if let Some(store) = &diagnostics {
        store.end_session();
    }
    result
}

fn startup_error(store: Option<&diagnostics::Diagnostics>, error: &eframe::Error) {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    let (category, code) = match error {
        eframe::Error::Winit(_) => ("window", None),
        eframe::Error::WinitEventLoop(_) => ("event-loop", None),
        eframe::Error::NoGlutinConfigs(..) => ("graphics-config", None),
        eframe::Error::Glutin(error) => ("graphics", error.raw_code()),
        eframe::Error::OpenGL(_) => ("renderer", None),
        _ => ("startup", None),
    };
    // Use an OS dialog here. The editor's own window may never have opened.
    let report = store.and_then(|s| {
        let r = s.record_startup_failure(category, code).ok()?;
        Some((s.read(&r).ok()?, s.directory().join(r.filename)))
    });
    let fallback = format!(
        "Phyro {}\nBuild {}\nTarget {}\nFailure starting the desktop editor: {category}\n",
        env!("CARGO_PKG_VERSION"),
        env!("PHYRO_BUILD"),
        env!("PHYRO_TARGET")
    );
    let text = report.as_ref().map_or(fallback.as_str(), |(body, _)| body);
    let mut description = format!(
        "The editor encountered a {category} error. Copy the crash report to the clipboard?"
    );
    if let Some((_, path)) = &report {
        description.push_str(&format!("\n\nSaved report\n{}", path.display()));
    }
    let answer = MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title("Phyro could not start")
        .set_description(description)
        .set_buttons(MessageButtons::YesNo)
        .show();
    if answer == MessageDialogResult::Yes
        && arboard::Clipboard::new()
            .and_then(|mut c| c.set_text(text))
            .is_err()
    {
        let message = if let Some((_, path)) = &report {
            format!(
                "The clipboard is unavailable. You can open the saved report at {}.",
                path.display()
            )
        } else {
            format!("The clipboard is unavailable.\n\n{text}")
        };
        MessageDialog::new()
            .set_title("Crash report")
            .set_description(message)
            .show();
    }
}

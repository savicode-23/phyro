use crate::canvas::Tool;
use crate::commands::Arrange;
use crate::keymap::{self, Action};
use eframe::egui::{Context, Event};

pub(crate) const DEFAULT_PENCIL_SMOOTHING: u8 = 50;

pub(crate) fn pencil_controls(ui: &mut eframe::egui::Ui, smoothing: &mut u8, pressure: &mut bool) {
    ui.label("Smoothing");
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = 78.;
        ui.add(eframe::egui::Slider::new(smoothing, 0..=200).suffix("%"))
            .on_hover_text(
                "Lower values follow fine movements. Higher values smooth out hand jitter.",
            );
    });
    ui.checkbox(pressure, "Pressure width")
        .on_hover_text("Vary stroke width with tablet pen pressure.");
}

pub(crate) const STACKING: [(Action, Arrange, &str); 4] = [
    (Action::Front, Arrange::ToFront, "Bring to front"),
    (Action::Raise, Arrange::Forward, "Raise one step"),
    (Action::Lower, Arrange::Backward, "Lower one step"),
    (Action::Back, Arrange::ToBack, "Send to back"),
];

pub(crate) const FLIPS: [(Action, bool, &str); 2] = [
    (Action::FlipH, true, "Flip horizontally"),
    (Action::FlipV, false, "Flip vertically"),
];

pub(crate) fn flip_menu(
    ui: &mut eframe::egui::Ui,
    document: &document_core::Document,
    selection: &[u64],
    available: bool,
) -> Option<bool> {
    let mut flip = None;
    ui.menu_button("Transform", |ui| {
        let enabled = available
            && !selection.is_empty()
            && document_core::arrange::available_units(document, selection) > 0;
        for (action, horizontal, label) in FLIPS {
            let button =
                eframe::egui::Button::new(label).shortcut_text(keymap::label(ui.ctx(), action));
            if ui.add_enabled(enabled, button).clicked() {
                flip = Some(horizontal);
                ui.close_menu();
            }
        }
    });
    flip
}

pub(crate) fn selection_shortcuts<T: Copy>(
    ctx: &Context,
    bindings: &[(Action, T, &str)],
    repeatable: bool,
) -> Vec<T> {
    let mut commands = Vec::new();
    let map = keymap::active(ctx);
    ctx.input_mut(|input| {
        if input.focused {
            input.events.retain(|event| {
                if let Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    repeat,
                    ..
                } = event
                {
                    let action = map.resolve(*key, *modifiers);
                    if let Some((_, command, _)) =
                        bindings.iter().find(|(a, _, _)| Some(*a) == action)
                    {
                        if repeatable || !repeat {
                            commands.push(*command);
                        }
                        return false;
                    }
                }
                true
            });
        }
    });
    commands
}

impl Tool {
    pub(crate) fn label_and_action(self) -> (&'static str, Action) {
        match self {
            Self::Select => ("Select", Action::SelectTool),
            Self::Node => ("Nodes", Action::NodeTool),
            Self::Rectangle => ("Rectangle", Action::RectangleTool),
            Self::Ellipse => ("Ellipse", Action::EllipseTool),
            Self::Pen => ("Pen", Action::PenTool),
            Self::Pencil => ("Pencil", Action::PencilTool),
            Self::Text => ("Text", Action::TextTool),
            Self::Eyedropper => ("Eyedropper", Action::EyedropperTool),
            Self::Gradient => ("Gradient", Action::GradientTool),
        }
    }
}

pub(crate) fn pressed(ctx: &Context) -> Option<Tool> {
    let map = keymap::active(ctx);
    ctx.input(|input| {
        if !input.focused {
            return None;
        }
        input.events.iter().find_map(|event| {
            let Event::Key {
                key,
                modifiers,
                pressed: true,
                repeat: false,
                ..
            } = event
            else {
                return None;
            };
            match map.resolve(*key, *modifiers)? {
                Action::SelectTool => Some(Tool::Select),
                Action::NodeTool => Some(Tool::Node),
                Action::RectangleTool => Some(Tool::Rectangle),
                Action::EllipseTool => Some(Tool::Ellipse),
                Action::PenTool => Some(Tool::Pen),
                Action::PencilTool => Some(Tool::Pencil),
                Action::TextTool => Some(Tool::Text),
                Action::EyedropperTool => Some(Tool::Eyedropper),
                Action::GradientTool => Some(Tool::Gradient),
                _ => None,
            }
        })
    })
}

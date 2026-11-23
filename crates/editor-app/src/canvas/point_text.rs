use super::View;
use document_core::typography::{TextAlign, TextStyle, Typeface};
use document_core::Document;
use eframe::egui::{self, Color32, FontFamily, FontId, Pos2, Rect, TextFormat, Ui};
use std::sync::Arc;
mod live;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Editing,
    Commit,
    Cancel,
}

pub(super) struct PointText {
    pub(super) target: Option<u64>,
    pub(super) layer: u64,
    pub(super) frame: u32,
    pub(super) anchor: [f32; 2],
    pub(super) style: TextStyle,
    pub(super) fill: [u8; 4],
    pub(super) new_object_style: Option<super::CanvasStyle>,
    pub(super) error: Option<String>,
    preview_error: Option<String>,
    /// Includes the old lettering after the user shortens or clears the draft.
    pub(super) original_bounds: Option<Rect>,
    pub(super) opening_local: Option<[f32; 2]>,
    prepared: Option<TextStyle>,
    preview: Option<Arc<Document>>,
    pub(super) controls: Option<Rect>,
    pub(super) number_focused: bool,
    feedback: Vec<egui::Shape>,
    opening: bool,
    rect: Rect,
    id: Option<egui::Id>,
    reveal: Option<Rect>,
}

impl PointText {
    pub(super) fn new(
        target: Option<u64>,
        layer: u64,
        anchor: [f32; 2],
        style: TextStyle,
        fill: [u8; 4],
    ) -> Self {
        Self {
            target,
            layer,
            frame: 0,
            anchor,
            style,
            fill,
            new_object_style: None,
            error: None,
            preview_error: None,
            original_bounds: None,
            opening_local: None,
            prepared: None,
            preview: None,
            controls: None,
            number_focused: false,
            feedback: Vec::new(),
            opening: true,
            rect: Rect::NOTHING,
            id: None,
            reveal: None,
        }
    }

    pub(super) fn insert(&self, doc: &mut Document) -> Result<u64, String> {
        let id = self
            .style
            .clone()
            .insert(doc, self.layer, self.anchor, self.fill)?;
        if let Some(style) = &self.new_object_style {
            let object = doc
                .objects
                .last_mut()
                .expect("text insertion appends the glyph outlines");
            style.paint_new_object(object);
        }
        Ok(id)
    }

    pub(super) fn hit(&self, pos: Pos2) -> bool {
        self.rect.contains(pos) || self.controls.is_some_and(|rect| rect.contains(pos))
    }

    pub(super) fn surrender_focus(&self, ctx: &egui::Context) {
        if let Some(id) = self.id {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
    }

    pub(super) fn take_caret_reveal(&mut self) -> Option<Rect> {
        self.reveal.take()
    }
}

fn lettering(
    ui: &Ui,
    text: &str,
    format: TextFormat,
    align: TextAlign,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0., format.clone());
    let galley = ui.fonts(|fonts| fonts.layout_job(job));
    if align == TextAlign::Left || galley.rows.len() < 2 {
        return galley;
    }
    // Point text aligns each explicit line to the widest line. Keeping row
    // coordinates relative to the top-left also keeps pointer-to-caret mapping exact.
    let mut job = egui::text::LayoutJob::default();
    let fraction = if align == TextAlign::Center { 0.5 } else { 1. };
    for (i, (line, row)) in text.split('\n').zip(&galley.rows).enumerate() {
        if i != 0 {
            job.append("\n", 0., format.clone());
        }
        let left = (galley.size().x - row.rect.width()) * fraction;
        job.append(line, left, format.clone());
    }
    ui.fonts(|fonts| fonts.layout_job(job))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, ImeEvent, Key, Modifiers, PointerButton};

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn run(
        ctx: &egui::Context,
        session: &mut PointText,
        view: View,
        events: Vec<Event>,
    ) -> (Outcome, egui::FullOutput) {
        let mut result = Outcome::Editing;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    result = session.show(ui, view, ui.clip_rect(), &Document::default(), 0);
                });
            },
        );
        (result, out)
    }

    fn view() -> View {
        View {
            board: Rect::from_min_size(egui::pos2(95., 72.), egui::vec2(256., 256.)),
            scale: 2.,
        }
    }

    #[test]
    fn inline_editor_handles_caret_clipboard_ime_newlines_and_explicit_commit() {
        let ctx = egui::Context::default();
        let style = TextStyle {
            content: String::new(),
            ..Default::default()
        };
        let mut s = PointText::new(None, 1, [8., 10.], style, [8, 16, 24, 255]);
        let command = Modifiers::CTRL | Modifiers::COMMAND;
        run(&ctx, &mut s, view(), vec![Event::Text("AB".into())]);
        assert_eq!(s.style.content, "AB");
        run(
            &ctx,
            &mut s,
            view(),
            vec![
                key(Key::ArrowLeft, Modifiers::NONE),
                Event::Paste("ä".into()),
            ],
        );
        assert_eq!(s.style.content, "AäB");
        run(&ctx, &mut s, view(), vec![key(Key::Enter, Modifiers::NONE)]);
        assert_eq!(s.style.content, "Aä\nB");
        run(
            &ctx,
            &mut s,
            view(),
            vec![
                Event::Ime(ImeEvent::Enabled),
                Event::Ime(ImeEvent::Preedit("e".into())),
            ],
        );
        run(
            &ctx,
            &mut s,
            view(),
            vec![
                Event::Ime(ImeEvent::Commit("é".into())),
                Event::Ime(ImeEvent::Disabled),
            ],
        );
        assert_eq!(s.style.content, "Aä\néB");
        let (_, out) = run(
            &ctx,
            &mut s,
            view(),
            vec![key(Key::A, command), Event::Copy],
        );
        assert!(out
            .platform_output
            .commands
            .iter()
            .any(|o| matches!(o, egui::OutputCommand::CopyText(t) if t == "Aä\néB")));
        run(
            &ctx,
            &mut s,
            view(),
            vec![Event::Cut, Event::Paste("Intro\nOutro".into())],
        );
        let (end, _) = run(&ctx, &mut s, view(), vec![key(Key::Enter, command)]);
        assert_eq!(end, Outcome::Commit);
        assert_eq!(s.style.content, "Intro\nOutro");
        s.surrender_focus(&ctx);
        assert!(!ctx.memory(|m| m.has_focus(s.id.unwrap())));
    }

    #[test]
    fn old_edit_footprint_tracks_the_camera_and_shortening_preserves_styles() {
        let ctx = egui::Context::default();
        let authored = TextStyle {
            content: "Longest line\nshort".into(),
            face: Typeface::HackRegular,
            size: 16.,
            letter_spacing: 2.,
            line_height: 1.6,
            align: TextAlign::Right,
        };
        let mut s = PointText::new(Some(3), 1, [-5., 12.], authored.clone(), [30, 70, 120, 255]);
        run(&ctx, &mut s, view(), vec![]);
        let old = Rect::from_min_size(s.anchor.into(), authored.layout().unwrap().size.into());
        s.original_bounds = Some(old);
        run(
            &ctx,
            &mut s,
            view(),
            vec![
                key(Key::A, Modifiers::CTRL | Modifiers::COMMAND),
                Event::Text("X".into()),
            ],
        );
        let moved = View {
            board: view().board.translate(egui::vec2(30., 48.)),
            scale: 1.3,
        };
        run(&ctx, &mut s, moved, vec![]);
        assert!(s.hit(moved.screen(old.min)) && s.hit(moved.screen(old.max)));
        assert!(!s.hit(view().screen(old.min)));
        assert_eq!(
            s.style,
            TextStyle {
                content: "X".into(),
                ..authored
            }
        );
        let outside = egui::pos2(690., 410.);
        let (end, _) = run(
            &ctx,
            &mut s,
            moved,
            vec![
                Event::PointerMoved(outside),
                Event::PointerButton {
                    pos: outside,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert_eq!(end, Outcome::Commit);
    }

    #[test]
    fn escaping_a_draft_or_committing_only_whitespace_cancels() {
        let ctx = egui::Context::default();
        for (text, end_key, mods) in [
            ("unsaved", Key::Escape, Modifiers::NONE),
            ("  \n", Key::Enter, Modifiers::CTRL | Modifiers::COMMAND),
        ] {
            let style = TextStyle {
                content: text.into(),
                ..Default::default()
            };
            let mut s = PointText::new(None, 1, [0.; 2], style, [255; 4]);
            run(&ctx, &mut s, view(), vec![]);
            assert_eq!(
                run(&ctx, &mut s, view(), vec![key(end_key, mods)]).0,
                Outcome::Cancel
            );
            s.surrender_focus(&ctx);
        }
    }

    #[test]
    fn clipped_caret_requests_a_camera_move_only_after_input() {
        let ctx = egui::Context::default();
        let mut s = PointText::new(
            None,
            1,
            [342., 247.],
            TextStyle {
                content: String::new(),
                ..Default::default()
            },
            [0; 4],
        );
        run(&ctx, &mut s, view(), vec![]);
        assert!(s.take_caret_reveal().is_none());
        run(&ctx, &mut s, view(), vec![Event::Text("Edge".into())]);
        let caret = s
            .take_caret_reveal()
            .expect("typing exposed a caret past the right edge");
        assert!(caret.right() > 800.);
        run(&ctx, &mut s, view(), vec![]);
        assert!(s.take_caret_reveal().is_none());
    }
}

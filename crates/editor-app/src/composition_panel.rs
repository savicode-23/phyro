use document_core::{AnimationProject, LoopMode};
use eframe::egui::{self, Ui};

/// Playback settings belong to the selected placement, not its source clip.
pub(super) fn playback(
    ui: &mut Ui,
    project: &mut AnimationProject,
    clip: usize,
    frame: u32,
    selection: &[u64],
) -> Option<String> {
    let [id] = selection else { return None };
    if project.clips.len() < 2 {
        return None;
    }
    let drawing = project.clips[clip].drawing_at(frame);
    let slot = drawing.objects.iter().position(|o| o.id == *id)?;
    let object = &drawing.objects[slot];
    let source = project.clip_index(object.clip_instance.as_ref()?.clip_id)?;
    let editable = crate::commands::is_editable(drawing, object.layer);
    let mut message = None;
    egui::CollapsingHeader::new("Clip playback")
        .default_open(true)
        .show_unindented(ui, |ui| {
            ui.strong(&project.clips[source].name);
            if !editable {
                ui.disable();
            }
            let source_frames = project.clips[source].frames;
            let clip = &mut project.clips[clip];
            let Some(instance) = clip.drawing_at(frame).objects[slot].clip_instance.as_ref() else {
                return;
            };
            let source_frame = instance.sample(frame).map_or(0, |(_, t)| t);
            if !instance.drawing_keys.is_empty() {
                let keyed = instance.drawing_keys.contains_key(&frame);
                let mut chosen = source_frame + 1;
                ui.horizontal(|ui| {
                    ui.label("Source frame");
                    ui.add(
                        egui::DragValue::new(&mut chosen)
                            .range(1..=source_frames)
                            .suffix(format!(" / {source_frames}")),
                    )
                    .on_hover_text("Choose the child frame to hold from this parent frame");
                });
                if chosen != source_frame + 1 {
                    message = clip
                        .drawing_at_mut(frame)
                        .set_drawing_key(*id, frame, chosen - 1)
                        .err();
                }
                ui.horizontal_wrapped(|ui| {
                    if keyed {
                        if ui
                            .button("Remove key")
                            .on_hover_text("Remove this drawing key")
                            .clicked()
                        {
                            message = clip
                                .drawing_at_mut(frame)
                                .remove_drawing_key(*id, frame)
                                .err();
                        }
                    } else if ui
                        .button("Key drawing")
                        .on_hover_text("Hold this source frame at the playhead")
                        .clicked()
                    {
                        message = clip
                            .drawing_at_mut(frame)
                            .set_drawing_key(*id, frame, chosen - 1)
                            .err();
                    }
                    if ui
                        .button("Clear keys and play")
                        .on_hover_text(
                            "Clear all drawing keys for this placement and resume timed playback",
                        )
                        .clicked()
                    {
                        message = clip.drawing_at_mut(frame).clear_drawing_keys(*id).err();
                    }
                });
                return;
            }
            let original_timing = instance.timing;
            let mut timing = original_timing;
            ui.horizontal(|ui| {
                ui.label("Offset");
                ui.add(egui::DragValue::new(&mut timing.offset).suffix(" frames"))
                    .on_hover_text("Child frame at the start of the parent clip");
            });
            ui.horizontal(|ui| {
                ui.label("Speed");
                ui.add(egui::DragValue::new(&mut timing.rate_numerator).range(1..=u16::MAX));
                ui.label("/");
                ui.add(egui::DragValue::new(&mut timing.rate_denominator).range(1..=u16::MAX))
                    .on_hover_text("1 / 1 uses the child clip's original frame rate");
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut timing.mode, LoopMode::Loop, "Loop");
                ui.selectable_value(&mut timing.mode, LoopMode::Clamp, "Hold at ends");
            });
            if timing != original_timing {
                clip.drawing_at_mut(frame).objects[slot]
                    .clip_instance
                    .as_mut()
                    .unwrap()
                    .timing = timing;
            }
            ui.horizontal_wrapped(|ui| {
                ui.weak(format!("Child frame {}", source_frame + 1));
                if ui
                    .button("Hold child frame")
                    .on_hover_text("Add a drawing key at the playhead and hold this child frame")
                    .clicked()
                {
                    message = clip
                        .drawing_at_mut(frame)
                        .set_drawing_key(*id, frame, source_frame)
                        .err();
                }
            });
        });
    message
}

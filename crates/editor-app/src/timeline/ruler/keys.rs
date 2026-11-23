use super::{has_key, layout::Row, BoneTrack, Tracks};
use crate::commands::is_editable;
use document_core::{Clip, Property, VectorObject};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::Arc,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Scope {
    clip: u64,
    exposure: u32,
    rig_start: u32,
    layer: Option<u64>,
    object: Option<u64>,
    bone: Option<u64>,
}

impl Scope {
    pub(super) fn new(
        clip: &Clip,
        frame: u32,
        object: Option<u64>,
        bone: Option<(u64, u32, u64)>,
    ) -> Self {
        let drawing = clip.exposure(frame).0;
        let layer = object
            .and_then(|id| clip.cels[&drawing].objects.iter().find(|o| o.id == id))
            .map(|o| o.layer);
        let rig_start = clip.rig_exposure(frame).0;
        let exposure = layer.map_or(rig_start, |id| clip.layer_exposure(frame, id).0);
        Self {
            clip: clip.session_id(),
            exposure,
            rig_start,
            layer,
            object,
            bone: bone
                .filter(|&(id, start, _)| id == clip.session_id() && start == rig_start)
                .map(|(_, _, id)| id),
        }
    }
}

pub(super) type Marks = BTreeSet<(Row, u32)>;

#[derive(Default)]
pub(in crate::timeline) struct KeySelection {
    pub(super) scope: Option<Scope>,
    pub(super) marks: Marks,
}

impl KeySelection {
    pub(in crate::timeline) fn clear_marks(&mut self) {
        self.marks.clear();
    }
    fn only_bones(&self) -> bool {
        self.marks.first().is_some_and(|k| k.0 == Row::Bone)
            && self.marks.last().is_some_and(|k| k.0 == Row::Bone)
    }

    fn matches(&self, current: Scope) -> bool {
        self.scope.is_some_and(|previous| {
            let bones = self
                .marks
                .range((Row::Bone, 0)..=(Row::Bone, u32::MAX))
                .next()
                .is_some();
            previous.clip == current.clip
                && (self.only_bones()
                    || (previous.exposure == current.exposure
                        && previous.layer == current.layer
                        && previous.object == current.object))
                && (!bones
                    || (previous.bone == current.bone && previous.rig_start == current.rig_start))
        })
    }

    pub(super) fn sync(&mut self, current: Scope) {
        if !self.matches(current) {
            self.marks.clear();
        }
        self.scope = Some(current);
    }

    // Validate and stage one exposure, so collisions or a rejected rig pose cannot
    // partially retime a selection. Other holds continue sharing their drawings.
    pub(super) fn edit(
        &mut self,
        clip: &mut Clip,
        delta: Option<i32>,
        frame: u32,
    ) -> Result<(), String> {
        let scope = self.scope.ok_or("Select at least one key.")?;
        if self.marks.is_empty()
            || scope.clip != clip.session_id()
            || !clip.cels.contains_key(&scope.exposure)
        {
            return Err("The selected keys are no longer available.".into());
        }
        if frame >= clip.frames {
            return Err("Move keys within the same drawing exposure.".into());
        }
        let start = clip.exposure(frame).0;
        let (rig_start, rig_length) = clip.rig_exposure(frame);
        let (owner, duration) = scope.layer.map_or((rig_start, rig_length), |layer| {
            clip.layer_exposure(frame, layer)
        });
        if !self.only_bones() && owner != scope.exposure {
            return Err("Move keys within the same drawing exposure.".into());
        }
        let exposure = owner..owner + duration;
        let original = &clip.cels[&start];
        let object_index = scope
            .object
            .and_then(|id| original.objects.iter().position(|o| o.id == id));
        let object = object_index.map(|i| &original.objects[i]);
        let bone_index = scope
            .bone
            .and_then(|id| original.skeleton.bones.iter().position(|b| b.id == id));
        let bone = bone_index.map(|index| BoneTrack {
            id: original.skeleton.bones[index].id,
            index,
            start: rig_start,
            end: rig_start + rig_length,
        });
        for &(row, from) in &self.marks {
            if row == Row::Bone && scope.rig_start != rig_start {
                return Err("Move bone keys within the same rig.".into());
            }
            if row != Row::Bone
                && !object.is_some_and(|o| {
                    is_editable(original, o.layer)
                        && !matches!(row, Row::Property(c) if c.is_muted(o))
                })
            {
                return Err("Choose keys on an editable track.".into());
            }
            if !key_at(row, clip, object, bone, &exposure, from) {
                return Err("A selected key no longer exists.".into());
            }
            if let Some(delta) = delta {
                let to = from
                    .checked_add_signed(delta)
                    .filter(|t| {
                        if row == Row::Bone {
                            (rig_start..rig_start + rig_length).contains(t)
                        } else {
                            exposure.contains(t)
                        }
                    })
                    .ok_or("Move keys within the same drawing exposure.")?;
                if !self.marks.contains(&(row, to))
                    && key_at(row, clip, object, bone, &exposure, to)
                {
                    return Err("An unselected key already occupies the destination.".into());
                }
            }
        }
        let mut drawing = original.clone();
        let doc = Arc::make_mut(&mut drawing);
        let geometry = Row::Property(Property::Geometry);
        let mut moved_guided_keys = false;
        if let (Some(delta), Some(index)) = (delta, object_index) {
            let object = &mut doc.objects[index];
            if object.animated.guided_geometry.is_some() {
                let moves: BTreeMap<_, _> = self
                    .marks
                    .range((geometry, 0)..=(geometry, u32::MAX))
                    .map(|&(_, tick)| (tick, tick.checked_add_signed(delta).unwrap()))
                    .collect();
                if !moves.is_empty() {
                    object.move_guided_keys(&moves)?;
                    moved_guided_keys = true;
                }
            }
        }
        let mut edit = |&(row, tick): &(Row, u32)| -> Result<(), String> {
            if moved_guided_keys && row == geometry {
                return Ok(());
            }
            let to = delta.map(|d| tick.checked_add_signed(d).unwrap());
            match row {
                Row::Property(c) => {
                    let o = &mut doc.objects[object_index.unwrap()];
                    match to {
                        Some(to) => c.move_key(o, tick, to)?,
                        None if !c.remove(o, tick) => {
                            return Err("The neighboring poses cannot be joined. Adjust their point matches before removing this key.".into());
                        }
                        None => {}
                    }
                }
                Row::Child => {
                    let keys = &mut doc.objects[object_index.unwrap()]
                        .clip_instance
                        .as_mut()
                        .unwrap()
                        .drawing_keys;
                    move_or_remove(keys, tick, to);
                }
                Row::Bone => move_or_remove(
                    &mut doc.skeleton.bones[bone_index.unwrap()].rotation_keys,
                    tick,
                    to,
                ),
                _ => unreachable!(),
            }
            Ok(())
        };
        // Move farthest keys first so selected neighbors can occupy one another's slots.
        if delta.is_some_and(|d| d > 0) {
            self.marks.iter().rev().try_for_each(&mut edit)?;
        } else {
            self.marks.iter().try_for_each(&mut edit)?;
        }
        if self
            .marks
            .range((Row::Bone, 0)..=(Row::Bone, u32::MAX))
            .next()
            .is_some()
        {
            doc.refresh_rig()?;
        }
        clip.cels.insert(start, drawing);
        if let Some(d) = delta {
            self.marks = self
                .marks
                .iter()
                .map(|&(r, t)| (r, t.checked_add_signed(d).unwrap()))
                .collect();
        } else {
            self.marks.clear();
        }
        Ok(())
    }
}

fn move_or_remove<T>(map: &mut std::collections::BTreeMap<u32, T>, from: u32, to: Option<u32>) {
    let value = map.remove(&from).expect("validated source key");
    if let Some(to) = to {
        map.insert(to, value);
    }
}

pub(super) fn key_at(
    row: Row,
    clip: &Clip,
    object: Option<&VectorObject>,
    bone: Option<BoneTrack>,
    exposure: &Range<u32>,
    tick: u32,
) -> bool {
    if row == Row::Bone {
        return bone.is_some_and(|b| b.key_at(clip, tick));
    }
    exposure.contains(&tick)
        && match row {
            Row::Property(c) => object.is_some_and(|o| has_key(c, o, tick)),
            Row::Child => object
                .and_then(|o| o.clip_instance.as_ref())
                .is_some_and(|i| i.drawing_keys.contains_key(&tick)),
            _ => false,
        }
}

impl Tracks {
    pub(super) fn row_exposure(&self, row: Row) -> Range<u32> {
        if row == Row::Bone {
            if let Some(bone) = self.bone {
                return bone.start..bone.end;
            }
        }
        self.exposure.clone()
    }

    pub(super) fn key_at(&self, row: Row, clip: &Clip, tick: u32) -> bool {
        key_at(
            row,
            clip,
            self.object(clip),
            self.bone,
            &self.exposure,
            tick,
        )
    }
}

impl super::Timeline {
    pub(in crate::timeline) fn sync_key_selection(
        &mut self,
        clip: &Clip,
        selected: Option<u64>,
        bone: Option<(u64, u32, u64)>,
    ) {
        if self.keys.scope.is_some() {
            self.keys.sync(Scope::new(clip, self.frame, selected, bone));
        }
    }

    pub(in crate::timeline) fn key_actions(
        &self,
        clip: &Clip,
        selected: Option<u64>,
    ) -> (bool, bool) {
        let object = self.editable_track(clip, selected);
        let remove = !self.keys.marks.is_empty()
            || object.is_some_and(|o| has_key(self.channel, o, self.frame));
        (object.is_some(), remove)
    }

    fn editable_track<'a>(
        &self,
        clip: &'a Clip,
        selected: Option<u64>,
    ) -> Option<&'a VectorObject> {
        let id = selected?;
        let doc = clip.drawing_at(self.frame);
        doc.objects
            .iter()
            .find(|o| o.id == id)
            .filter(|o| is_editable(doc, o.layer) && !self.channel.is_muted(o))
    }
    pub(crate) fn remove_keys(&mut self, clip: &mut Clip, selected: Option<u64>) -> bool {
        if self.keys.marks.is_empty() {
            if !self.key_actions(clip, selected).1 {
                return false;
            }
            self.keys.scope = Some(Scope::new(clip, self.frame, selected, None));
            self.keys
                .marks
                .insert((Row::Property(self.channel), self.frame));
        }
        self.frame_focus = None;
        let result = self.keys.edit(clip, None, self.frame);
        self.result(result).is_some()
    }
    pub(crate) fn focused_keys(
        &self,
        clip: &Clip,
        object: Option<u64>,
        bone: Option<(u64, u32, u64)>,
    ) -> Option<bool> {
        self.keys.scope?;
        self.keys
            .matches(Scope::new(clip, self.frame, object, bone))
            .then_some(!self.keys.marks.is_empty())
    }

    pub(crate) fn leave_grid_selection(&mut self) {
        self.error = None;
        self.deselect_grid();
        self.keys = KeySelection::default();
    }
    pub(crate) fn leave_if_pressed(&mut self, ui: &eframe::egui::Ui) {
        if (self.frame_focus.is_some() || self.keys.scope.is_some() || self.error.is_some())
            && ui.input(|i| i.pointer.any_pressed())
            && ui.rect_contains_pointer(ui.max_rect())
        {
            self.leave_grid_selection();
        }
    }
    pub(crate) fn deselect_grid(&mut self) {
        self.keys.clear_marks();
        self.frame_focus = None;
        self.gesture = None;
    }
    pub(super) fn focus_row_label(&mut self, scope: Scope) {
        self.deselect_grid();
        self.keys.scope = Some(scope);
    }
    pub(in crate::timeline) fn select_current_key(&mut self, clip: &Clip, selected: Option<u64>) {
        if !self
            .editable_track(clip, selected)
            .is_some_and(|o| has_key(self.channel, o, self.frame))
        {
            return;
        }
        self.frame_focus = None;
        self.keys.scope = Some(Scope::new(clip, self.frame, selected, None));
        self.keys.marks.clear();
        self.keys
            .marks
            .insert((Row::Property(self.channel), self.frame));
    }
}

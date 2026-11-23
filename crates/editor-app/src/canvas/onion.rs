use std::sync::Arc;

mod detail;
mod held_ranges;
use held_ranges::HeldRanges;

use super::display_filter::{self, ARTWORK_TEXTURE};
use document_core::{Clip, Document, VectorObject};
use eframe::egui::{self, Color32, Painter, Pos2, Rect, TextureHandle};

use super::View;

#[derive(Clone)]
struct Ghost {
    drawing: Arc<Document>,
    frame: u32,
    tint: [u8; 4],
}

struct Mask {
    drawing: Arc<Document>,
    frame: u32,
    scale: f32,
    texture: TextureHandle,
}

#[derive(Default)]
pub(super) struct OnionSkins {
    ghosts: Vec<Ghost>,
    cache: Vec<Mask>,
    detail: detail::Detail,
    origin: Option<[f32; 2]>,
    holds: HeldRanges,
}

impl OnionSkins {
    pub(super) fn set(
        &mut self,
        clip: &Clip,
        frame: u32,
        sides: [(u32, [u8; 4]); 2],
        editing: bool,
        painted: Option<&Arc<Document>>,
    ) {
        self.origin = editing.then_some(clip.origin);
        self.ghosts.clear();
        if !editing || sides.iter().all(|(count, _)| *count == 0) {
            self.cache.clear();
            self.detail.clear();
            return;
        }
        let (start, duration) = clip.exposure(frame);
        let active = &clip.cels[&start];
        let animated: Vec<_> = animated_objects(active).collect();
        self.holds.begin(painted);
        let mut shared_snapshot = None;
        for (forward, (count, tint)) in [false, true].into_iter().zip(sides) {
            let mut previous = frame;
            for distance in 1..=count.min(crate::timeline::MAX_ONIONS) {
                let current_exposure = (start..start + duration).contains(&previous);
                let Some(tick) = find_adjacent(
                    clip,
                    previous,
                    forward,
                    current_exposure.then_some(&mut self.holds),
                    current_exposure.then_some(animated.as_slice()),
                ) else {
                    break;
                };
                let start = clip.exposure(tick).0;
                let mut color = tint;
                color[3] = (u32::from(tint[3]) / distance) as u8;
                let source = &clip.cels[&start];
                // Reuse the already painted snapshot when this exposure also
                // supplies the drawing being edited. Retaining the live Arc
                // here would copy the whole drawing on the next mutable access.
                let snapshot = if Arc::ptr_eq(source, active) {
                    shared_snapshot.get_or_insert_with(|| {
                        self.cache
                            .iter()
                            .find(|m| {
                                m.frame == tick
                                    && (Arc::ptr_eq(&m.drawing, source) || m.drawing == *source)
                            })
                            .map_or_else(|| Arc::clone(source), |m| Arc::clone(&m.drawing))
                    })
                } else {
                    source
                };
                self.ghosts.push(Ghost {
                    drawing: Arc::clone(snapshot),
                    frame: tick,
                    tint: color,
                });
                previous = tick;
            }
        }
    }
    pub(super) fn paint_ghosts(
        &mut self,
        ctx: &egui::Context,
        painter: &Painter,
        view: View,
        scale: f32,
    ) -> Result<(), String> {
        if self.ghosts.is_empty() {
            self.cache.clear();
            self.detail.clear();
            return Ok(());
        }
        if self.detail.paint(ctx, painter, view, &self.ghosts) {
            return Ok(());
        }
        let mut available = std::mem::take(&mut self.cache);
        for ghost in self.ghosts.iter().rev() {
            let scale = scale.min(512.0 / ghost.drawing.width.max(ghost.drawing.height) as f32);
            let matched = available.iter().position(|m| {
                m.frame == ghost.frame
                    && m.scale == scale
                    && (Arc::ptr_eq(&m.drawing, &ghost.drawing) || m.drawing == ghost.drawing)
            });
            let mask = if let Some(i) = matched {
                available.swap_remove(i)
            } else {
                let pixels = renderer::render_scaled(&ghost.drawing, ghost.frame, scale)?;
                let image = egui::ColorImage {
                    size: [pixels.width() as usize, pixels.height() as usize],
                    pixels: pixels
                        .data()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| Color32::from_rgba_premultiplied(p[3], p[3], p[3], p[3]))
                        .collect(),
                };
                Mask {
                    drawing: Arc::clone(&ghost.drawing),
                    frame: ghost.frame,
                    scale,
                    texture: ctx.load_texture("onion silhouette", image, ARTWORK_TEXTURE),
                }
            };
            let [r, g, b, a] = ghost.tint;
            painter.add(display_filter::image(
                painter,
                mask.texture.id(),
                view.board,
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
                Color32::from_rgba_unmultiplied(r, g, b, a),
            ));
            self.cache.push(mask);
        }
        Ok(())
    }

    pub(super) fn paint_origin(&self, painter: &Painter, view: View) {
        let Some(origin) = self.origin else { return };
        let p = view.screen(origin.into());
        for (width, color) in [(3.0_f32, Color32::BLACK), (1.0, Color32::WHITE)] {
            painter.circle_stroke(p, 4.0, (width, color));
            for axis in [egui::vec2(7., 0.), egui::vec2(0., 7.)] {
                painter.line_segment([p - axis, p + axis], (width, color));
            }
        }
    }
}

fn animated_objects(drawing: &Document) -> impl Iterator<Item = &VectorObject> {
    drawing
        .objects
        .iter()
        .filter(|o| potentially_animated(drawing, o))
}

fn potentially_animated(drawing: &Document, o: &VectorObject) -> bool {
    drawing.layers.iter().any(|l| l.id == o.layer && l.visible)
        && (o.animated.visible || !o.animated.visible_keys.is_empty())
        && (o.opacity > 0. || !o.opacity_keys.is_empty())
        && (o.has_animated_rig()
            || o.last_keyframe().is_some()
            || o.clip_instance
                .as_ref()
                .and_then(|i| i.shared_clip())
                .is_some_and(|clip| {
                    clip.cels.len() > 1 || animated_objects(clip.drawing_at(0)).next().is_some()
                }))
}

fn object_pose_changed(object: &VectorObject, from: u32, tick: u32) -> bool {
    if from == tick {
        return false;
    }
    let draws_at = |t| {
        object.visible_at(t)
            && object.opacity_at(t) > 0.
            && (object.is_instance() || object.has_fill_at(t) || object.has_stroke_at(t))
    };
    let visible = draws_at(tick);
    if draws_at(from) != visible {
        return true;
    }
    if !visible {
        return false;
    }
    if object.opacity_at(from) != object.opacity_at(tick)
        || object.position_at(from) != object.position_at(tick)
        || object.rotation_at(from) != object.rotation_at(tick)
        || object.scale_at(from) != object.scale_at(tick)
        || object.skew_at(from) != object.skew_at(tick)
    {
        return true;
    }
    if let Some(child) = &object.clip_instance {
        if let (Some((first, a)), Some((next, b))) = (child.sample(from), child.sample(tick)) {
            if !std::ptr::eq(first, next) {
                return true;
            }
            return a != b && animated_objects(first).any(|o| object_pose_changed(o, a, b));
        }
    }
    (!object.no_fill && object.fill_at(from) != object.fill_at(tick))
        || (!object.no_stroke
            && (object.stroke_at(from) != object.stroke_at(tick)
                || object.stroke_width_at(from) != object.stroke_width_at(tick)))
        || ((object.skin.is_some() || !object.animated.geometry_keys.is_empty())
            && object.skinned_shape_at(from) != object.skinned_shape_at(tick))
}

#[cfg(test)]
fn adjacent_pose(clip: &Clip, from: u32, forward: bool) -> Option<u32> {
    find_adjacent(clip, from, forward, None, None)
}

/// The last tick in the requested direction at which every local input is
/// constant. A motion path and a custom pivot are functions of these tracks.
fn held_through(o: &VectorObject, tick: u32, mut edge: u32) -> u32 {
    use document_core::{Easing, Property};
    fn track<T>(
        o: &VectorObject,
        property: Property,
        tick: u32,
        edge: u32,
        keys: &std::collections::BTreeMap<u32, T>,
    ) -> u32 {
        if edge == tick || keys.len() < 2 || property.is_muted(o) {
            return edge;
        }
        let previous = keys.range(..=tick).next_back().map(|(&at, _)| at);
        let next = keys.range(tick + 1..).next().map(|(&at, _)| at);
        match (previous, next) {
            (None, Some(next)) if edge > tick => edge.min(next),
            (Some(previous), None) if edge < tick => edge.max(previous),
            (None, _) | (_, None) => edge,
            (Some(previous), Some(next)) if property.easing_at(o, previous) == Easing::Hold => {
                if edge < tick {
                    edge.max(previous)
                } else {
                    edge.min(next - 1)
                }
            }
            _ => tick,
        }
    }
    edge = track(o, Property::Position, tick, edge, &o.keys);
    edge = track(o, Property::Rotation, tick, edge, &o.rotation_keys);
    edge = track(o, Property::Scale, tick, edge, &o.scale_keys);
    edge = track(o, Property::Opacity, tick, edge, &o.opacity_keys);
    let a = &o.animated;
    edge = track(o, Property::Geometry, tick, edge, &a.geometry_keys);
    edge = track(o, Property::Fill, tick, edge, &a.fill_keys);
    edge = track(o, Property::Stroke, tick, edge, &a.stroke_keys);
    edge = track(o, Property::StrokeWidth, tick, edge, &a.width_keys);
    edge = track(o, Property::Visibility, tick, edge, &a.visible_keys);
    edge = track(o, Property::Skew, tick, edge, &a.skew_keys);
    track(o, Property::MotionProgress, tick, edge, &a.progress_keys)
}

fn find_adjacent(
    clip: &Clip,
    from: u32,
    forward: bool,
    holds: Option<&mut HeldRanges>,
    cached_objects: Option<&[&VectorObject]>,
) -> Option<u32> {
    let (start, duration) = clip.exposure(from);
    let edge = if forward { start + duration - 1 } else { start };
    let advance = |t: u32, d: u32| if forward { t + d } else { t - d };
    let drawing = clip.drawing_at(from);
    let collected;
    let animated = if let Some(objects) = cached_objects {
        objects
    } else {
        collected = animated_objects(drawing).collect::<Vec<_>>();
        &collected
    };
    let changed = |tick| animated.iter().any(|o| object_pose_changed(o, from, tick));
    // Nested playback, skeletal chains and curved motion can return to the same
    // pose within a single exposure. Comparing only its endpoints would miss it.
    if animated.iter().any(|o| {
        o.clip_instance.is_some()
            || o.has_animated_rig()
            || !o.animated.is_default()
            || o.motion.is_some()
    }) {
        // A drawing holds its final values. Only a nested clip can keep playing
        // past the last local key; skip the held tail in either scrub direction.
        let nested = animated.iter().any(|o| o.clip_instance.is_some());
        let mut holds = if nested { holds } else { None };
        let last = if nested {
            clip.frames - 1
        } else {
            animated
                .iter()
                .filter_map(|o| o.last_keyframe())
                .chain(
                    drawing
                        .skeleton
                        .bones
                        .iter()
                        .filter_map(|b| b.rotation_keys.keys().next_back().copied()),
                )
                .max()
                .unwrap_or(0)
        };
        let start = from.min(last);
        let end = edge.min(last);
        let mut cursor = holds
            .as_mut()
            .map_or(start, |ranges| ranges.skip(start, end, animated));
        let local_tracks = !nested && !animated.iter().any(|o| o.has_animated_rig());
        let changed_tick = loop {
            if cursor == end {
                break None;
            }
            cursor = advance(cursor, 1);
            if changed(cursor) {
                break Some(cursor);
            }
            if local_tracks {
                // Interpolated paths can turn back within a span. Only skip
                // frames for which every contributing track provably holds.
                let mut through = end;
                for &object in animated {
                    through = held_through(object, cursor, through);
                    if through == cursor {
                        break;
                    }
                }
                cursor = through;
            }
        };
        if let Some(ranges) = holds {
            let last_equal =
                changed_tick.map_or(end, |tick| if forward { tick - 1 } else { tick + 1 });
            ranges.remember(start, last_equal);
        }
        if let Some(tick) = changed_tick {
            return Some(tick);
        }
        return if forward {
            (edge + 1 < clip.frames).then_some(edge + 1)
        } else {
            edge.checked_sub(1)
        };
    }
    let mut cursor = from;
    while !animated.is_empty() && cursor != edge {
        let first = advance(cursor, 1);
        if changed(first) {
            return Some(first);
        }
        fn boundary<V>(
            keys: &std::collections::BTreeMap<u32, V>,
            cursor: u32,
            forward: bool,
        ) -> Option<u32> {
            if forward {
                keys.range(cursor + 1..).next().map(|(&t, _)| t)
            } else {
                keys.range(..cursor).next_back().map(|(&t, _)| t)
            }
        }
        let distance = animated
            .iter()
            .flat_map(|o| {
                [
                    boundary(&o.keys, cursor, forward),
                    boundary(&o.rotation_keys, cursor, forward),
                    boundary(&o.scale_keys, cursor, forward),
                    boundary(&o.opacity_keys, cursor, forward),
                    boundary(&o.animated.geometry_keys, cursor, forward),
                    boundary(&o.animated.skew_keys, cursor, forward),
                    boundary(&o.animated.width_keys, cursor, forward),
                    boundary(&o.animated.visible_keys, cursor, forward),
                    boundary(&o.animated.fill_keys, cursor, forward),
                    boundary(&o.animated.stroke_keys, cursor, forward),
                    boundary(&o.animated.progress_keys, cursor, forward),
                ]
            })
            .flatten()
            .map(|t| t.abs_diff(cursor))
            .min()
            .unwrap_or(u32::MAX)
            .min(edge.abs_diff(cursor));
        let next = advance(cursor, distance);
        if changed(next) {
            let (mut low, mut high) = (1, distance);
            while low + 1 < high {
                let mid = low + (high - low) / 2;
                if changed(advance(cursor, mid)) {
                    high = mid
                } else {
                    low = mid
                }
            }
            return Some(advance(cursor, high));
        }
        cursor = next;
    }
    if forward {
        (edge + 1 < clip.frames).then_some(edge + 1)
    } else {
        edge.checked_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use document_core::AnimationProject;

    #[test]
    fn jumping_held_local_tracks_matches_stepping_every_pose() {
        use document_core::{Easing, Interpolation, MotionPath, PathNode, Property};

        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        clip.set_length(96).unwrap();
        let doc = clip.drawing_at_mut(0);
        doc.add_rectangle([20.; 2], [6.; 2], [255; 4]);
        doc.add_rectangle([40.; 2], [6.; 2], [255; 4]);
        let a = &mut doc.objects[0];
        a.interpolation = Interpolation::Hold;
        a.animated
            .fill_keys
            .extend([(9, [230; 4]), (24, [232; 4]), (50, [230; 4])]);
        Property::Fill.set_easing(a, 24, Easing::EaseInOut);
        a.motion = Some(
            MotionPath::new(
                vec![
                    PathNode::corner([0., 0.]),
                    PathNode::corner([10., 10.]),
                    PathNode::corner([0., 0.]),
                ],
                false,
            )
            .unwrap(),
        );
        a.animated.progress_keys.extend([(57, 0.), (70, 1.)]);
        Property::MotionProgress.set_easing(a, 57, Easing::Linear);
        a.rotation_keys.extend([(4, -30.), (48, 80.)]);
        Property::Rotation.set_muted(a, true);
        let b = &mut doc.objects[1];
        b.animated
            .visible_keys
            .extend([(8, true), (15, false), (83, true)]);
        b.keys.extend([(12, [1.; 2]), (19, [4.; 2]), (80, [5.; 2])]);
        Property::Position.set_easing(b, 19, Easing::Hold);
        let doc = clip.drawing_at(0);
        for from in 0..clip.frames {
            let changed = |tick| {
                doc.objects
                    .iter()
                    .any(|o| object_pose_changed(o, from, tick))
            };
            assert_eq!(
                adjacent_pose(clip, from, true),
                (from + 1..clip.frames).find(|&t| changed(t)),
                "forward from {from}"
            );
            assert_eq!(
                adjacent_pose(clip, from, false),
                (0..from).rev().find(|&t| changed(t)),
                "backward from {from}"
            );
        }
    }

    fn keyed_rig(clip: &mut Clip) {
        clip.set_length(8).unwrap();
        let scene = clip.drawing_at_mut(0);
        let root = scene.add_bone("Arm", None, [20., 20.], [35., 20.]).unwrap();
        let wrist = scene
            .add_bone("Hand", Some(root), [35., 20.], [42., 20.])
            .unwrap();
        let shape = scene.add_rectangle([40., 16.], [8., 4.], [255; 4]);
        scene.bind_rigid(shape, Some(wrist)).unwrap();
        for (tick, angle) in [(0, 0.), (2, 0.), (4, 70.), (6, 0.)] {
            scene.set_bone_rotation_key(root, tick, angle).unwrap();
        }
    }

    #[test]
    fn bone_ghosts_compare_evaluated_artwork_and_skip_held_angles() {
        let mut p = AnimationProject::default();
        let clip = &mut p.clips[0];
        keyed_rig(clip);
        let mut skins = OnionSkins::default();
        skins.set(clip, 1, [(1, [255; 4]), (2, [255; 4])], true, None);
        assert_eq!(
            skins.ghosts.iter().map(|g| g.frame).collect::<Vec<_>>(),
            [3, 4]
        );
        let ghost = &skins.ghosts[0];
        let current = renderer::render(clip.drawing_at(1), 1).unwrap();
        let mask = renderer::render(&ghost.drawing, ghost.frame).unwrap();
        assert_ne!(current.data(), mask.data());
        assert_eq!(adjacent_pose(clip, 7, false), Some(5));
        let scene = clip.drawing_at_mut(0);
        let root = scene.skeleton.bones[0].id;
        scene
            .set_bone_interpolation(root, document_core::Interpolation::Hold)
            .unwrap();
        assert_eq!(adjacent_pose(clip, 1, true), Some(4));
        assert_eq!(adjacent_pose(clip, 7, false), Some(5));
        clip.drawing_at_mut(0).layers[0].visible = false;
        assert_eq!(adjacent_pose(clip, 2, true), None);
        clip.drawing_at_mut(0).layers[0].visible = true;
        clip.drawing_at_mut(0).objects[0].opacity = 0.;
        assert_eq!(adjacent_pose(clip, 2, true), None);
        let scene = clip.drawing_at_mut(0);
        scene.objects[0].opacity = 1.;
        scene.bind_rigid(scene.objects[0].id, None).unwrap();
        assert_eq!(adjacent_pose(clip, 2, true), None);
    }

    #[test]
    fn skin_bend_produces_an_onion_even_when_the_object_transform_is_static() {
        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        keyed_rig(clip);
        let scene = clip.drawing_at_mut(0);
        let id = scene.objects[0].id;
        let child = scene.skeleton.bones[1].id;
        let shape = document_core::path_edit::primitive_path(&scene.objects[0].shape).unwrap();
        scene.objects[0].set_shape_at(0, shape).unwrap();
        scene.bind_skin_at(id, child, 0).unwrap();
        assert_eq!(
            scene.objects[0].position_at(0),
            scene.objects[0].position_at(4)
        );
        assert_eq!(adjacent_pose(clip, 1, true), Some(3));
    }

    #[test]
    fn held_exposure_reuses_both_ghosts_and_refreshes_their_pixels_after_an_edit() {
        let mut project = AnimationProject::from_document(Document {
            width: 128,
            height: 128,
            ..Document::default()
        });
        let clip = &mut project.clips[0];
        keyed_rig(clip);
        let mut skins = OnionSkins::default();
        let ctx = egui::Context::default();
        let view = View {
            board: Rect::from_min_max(Pos2::ZERO, egui::pos2(128., 128.)),
            scale: 1.,
        };
        let mut paint = |clip: &mut Clip, expect_upload: bool| {
            skins.set(clip, 3, [(1, [255; 4]); 2], true, None);
            // The canvas takes mutable access after the timeline prepares the ghosts.
            let drawing = clip.drawing_at_mut(3) as *const Document;
            let output = ctx.run(Default::default(), |ctx| {
                skins
                    .paint_ghosts(
                        ctx,
                        &ctx.layer_painter(egui::LayerId::background()),
                        view,
                        1.,
                    )
                    .unwrap();
            });
            assert_eq!(skins.cache.len(), 2);
            for mask in &skins.cache {
                let upload = output
                    .textures_delta
                    .set
                    .iter()
                    .find(|(id, _)| *id == mask.texture.id());
                assert_eq!(upload.is_some(), expect_upload);
                if let Some((_, delta)) = upload {
                    let egui::ImageData::Color(image) = &delta.image else {
                        panic!("expected a color mask")
                    };
                    let pixels = renderer::render(clip.drawing_at(3), mask.frame).unwrap();
                    let expected_alpha = pixels.data().as_chunks::<4>().0.iter().map(|p| p[3]);
                    assert!(image.pixels.iter().map(|p| p.a()).eq(expected_alpha));
                }
            }
            drawing
        };
        let active = paint(clip, true);
        assert_eq!(active, paint(clip, false));
        clip.drawing_at_mut(3)
            .add_rectangle([6., 12.], [16., 9.], [255; 4]);
        let edited = paint(clip, true);
        assert_eq!(edited, paint(clip, false));
    }

    #[test]
    fn nested_bone_ghosts_respect_the_child_playback_rate() {
        let mut project = AnimationProject::default();
        project.clips[0].set_length(16).unwrap();
        project.clips[0].set_fps(12).unwrap();
        let child = project.add_clip("Wrist").unwrap();
        let source = &mut project.clips[child];
        source.set_fps(6).unwrap();
        keyed_rig(source);
        let id = source.id;
        project.place_clip(id, 0, 0, 1, [0.; 2]).unwrap();
        let clip = &project.clips[0];
        assert_eq!(adjacent_pose(clip, 0, true), Some(6));
        assert_eq!(adjacent_pose(clip, 13, false), Some(11));
    }

    #[test]
    fn nested_drawing_keys_show_the_next_distinct_held_pose() {
        let mut project = AnimationProject::default();
        let child = project.add_clip("Eyes").unwrap();
        let clip = &mut project.clips[child];
        clip.drawing_at_mut(0)
            .add_rectangle([4.; 2], [8., 4.], [255; 4]);
        clip.duplicate_frame(0).unwrap();
        clip.drawing_at_mut(1).objects[0].size[1] = 1.;
        let id = clip.id;
        let placed = project.place_clip(id, 0, 0, 1, [0.; 2]).unwrap();
        let parent = &mut project.clips[0];
        let drawing = parent.drawing_at_mut(0);
        drawing.set_drawing_key(placed, 0, 1).unwrap();
        drawing.set_drawing_key(placed, 1, 1).unwrap();
        drawing.set_drawing_key(placed, 3, 0).unwrap();
        assert_eq!(adjacent_pose(parent, 0, true), Some(3));
        assert_eq!(adjacent_pose(parent, 4, false), Some(2));
    }

    #[test]
    fn nested_ghosts_skip_parent_ticks_that_render_the_same_child_pose() {
        let mut p = AnimationProject::default();
        let child = p.add_clip("Blink").unwrap();
        let c = &mut p.clips[child];
        c.set_length(3).unwrap();
        c.set_fps(6).unwrap();
        c.drawing_at_mut(0)
            .add_rectangle([1.; 2], [3.; 2], [255; 4]);
        c.drawing_at_mut(0).objects[0]
            .keys
            .extend([(0, [1.; 2]), (2, [9.; 2])]);
        let id = c.id;
        p.place_clip(id, 0, 0, 1, [0.; 2]).unwrap();
        let clip = &p.clips[0];
        assert_eq!(adjacent_pose(clip, 0, true), Some(2));
        assert_eq!(adjacent_pose(clip, 3, false), Some(1));
    }

    #[test]
    fn finished_tweens_and_stepped_keys_do_not_consume_ghost_slots() {
        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        clip.set_length(10).unwrap();
        clip.blank_frame(7).unwrap();
        let scene = clip.drawing_at_mut(0);
        scene.add_rectangle([0.; 2], [10.; 2], [255; 4]);
        scene.objects[0].opacity_keys.extend([(0, 0.), (2, 1.)]);
        let mut skins = OnionSkins::default();
        skins.set(clip, 4, [(1, [255; 4]), (1, [255; 4])], true, None);
        assert_eq!(
            skins.ghosts.iter().map(|g| g.frame).collect::<Vec<_>>(),
            [1, 8]
        );
        let o = &mut clip.drawing_at_mut(0).objects[0];
        o.interpolation = document_core::Interpolation::Hold;
        o.opacity_keys.insert(6, 0.5);
        skins.set(clip, 4, [(1, [255; 4]), (1, [255; 4])], true, None);
        assert_eq!(
            skins.ghosts.iter().map(|g| g.frame).collect::<Vec<_>>(),
            [1, 6]
        );
    }
    #[test]
    fn both_sides_can_show_eight_poses_and_recolor_the_cached_masks() {
        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        clip.set_length(17).unwrap();
        let source = clip.drawing_at(0).clone();
        for tick in 0..17 {
            let mut drawing = source.clone();
            drawing.add_rectangle([tick as f32, 1.], [2.; 2], [255; 4]);
            clip.cels.insert(tick, Arc::new(drawing));
        }
        let ctx = egui::Context::default();
        let view = View {
            board: Rect::from_min_max(Pos2::ZERO, egui::pos2(128., 128.)),
            scale: 1.,
        };
        let mut skins = OnionSkins::default();
        for (index, alpha) in [160, 80].into_iter().enumerate() {
            let sides = [(8, [120, 20, 30, alpha]), (8, [30, 50, 200, alpha])];
            skins.set(clip, 8, sides, true, None);
            assert_eq!(skins.ghosts.len(), 16);
            assert_eq!(
                (skins.ghosts[7].frame, skins.ghosts[7].tint),
                (0, [120, 20, 30, alpha / 8])
            );
            assert_eq!(
                (skins.ghosts[15].frame, skins.ghosts[15].tint),
                (16, [30, 50, 200, alpha / 8])
            );
            let output = ctx.run(Default::default(), |ctx| {
                skins
                    .paint_ghosts(
                        ctx,
                        &ctx.layer_painter(egui::LayerId::background()),
                        view,
                        1.,
                    )
                    .unwrap();
            });
            let uploads = skins
                .cache
                .iter()
                .filter(|mask| {
                    output
                        .textures_delta
                        .set
                        .iter()
                        .any(|(id, _)| *id == mask.texture.id())
                })
                .count();
            assert_eq!(uploads, if index == 0 { 16 } else { 0 });
        }
        skins.set(clip, 8, [(0, [255; 4]), (4, [255; 4])], true, None);
        assert_eq!(
            skins.ghosts.iter().map(|g| g.frame).collect::<Vec<_>>(),
            [9, 10, 11, 12]
        );
    }

    #[test]
    fn onions_skip_held_drawings_and_stay_out_of_playback() {
        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        clip.blank_frame(2).unwrap();
        let sides = [(1, [255, 0, 0, 110]), (1, [0, 255, 255, 110])];
        let mut onion = OnionSkins::default();
        onion.set(clip, 0, sides, true, None);
        assert_eq!(onion.ghosts.len(), 1);
        assert_eq!(onion.ghosts[0].frame, 3);
        onion.set(clip, 0, sides, false, None);
        assert!(onion.ghosts.is_empty());
        assert!(onion.origin.is_none());
    }
}

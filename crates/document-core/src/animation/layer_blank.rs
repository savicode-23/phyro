use super::Clip;
#[cfg(test)]
use std::sync::Arc;

impl Clip {
    /// Start an independent drawing on one layer without shifting animation time.
    /// Inside a hold the new exposure begins at the playhead. From a drawing
    /// start, advance one tick so the drawing just authored remains available.
    pub fn blank_layer_frame(&mut self, frame: u32, layer: u64) -> Result<u32, String> {
        self.new_layer_frame(frame, layer, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnimationProject, Layer, Shape, MAX_FRAMES};

    #[test]
    fn layer_blank_splits_a_long_hold_without_retiming_other_layers_or_events() {
        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        clip.set_length(1000).unwrap();
        clip.set_fps(24).unwrap();
        let d = clip.drawing_at_mut(0);
        d.layers.push(Layer {
            id: 2,
            name: "Sparks".into(),
            ..Layer::default()
        });
        let plane = d.add_rectangle([0.; 2], [12.; 2], [200; 4]);
        d.objects[0].keys.insert(0, [0.; 2]);
        d.objects[0].keys.insert(999, [500., 45.]);
        let bone = d.add_bone("Wing", None, [0., 0.], [10., 0.]).unwrap();
        d.set_bone_rotation_key(bone, 999, 13.).unwrap();
        d.bind_rigid(plane, Some(bone)).unwrap();
        d.layers[0].locked = true;
        d.guides_x.push(312.);
        let spark = d.add_shape([8.; 2], [4.; 2], [100; 4], Shape::Ellipse, 2);
        let cursor = d.next_id;
        clip.add_event(870, "flash").unwrap();
        clip.cels.insert(900, clip.cels[&0].clone());
        let before = clip.clone();
        assert_eq!(clip.blank_layer_frame(400, 2), Ok(400));
        assert_eq!(clip.exposure(399), (0, 400));
        assert_eq!(clip.exposure(899), (400, 500));
        assert_eq!(clip.exposure(900), (900, 100));
        assert_eq!(clip.cels.len(), 3);
        assert_eq!(clip.frames, 1000);
        assert_eq!(clip.fps, 24);
        assert_eq!(clip.events, before.events);
        for tick in [0, 900] {
            assert!(Arc::ptr_eq(&clip.cels[&tick], &before.cels[&tick]));
        }
        let mut expected = before.drawing_at(400).clone();
        expected.objects.retain(|o| o.id != spark);
        assert_eq!(clip.drawing_at(400), &expected);
        let added =
            clip.drawing_at_mut(400)
                .add_shape([5.; 2], [3.; 2], [255; 4], Shape::Ellipse, 2);
        assert_eq!(added, cursor);
        for tick in [0, 399, 400, 550, 899, 900, 999] {
            let objects = &clip.drawing_at(tick).objects;
            assert_eq!(
                objects[0].local_to_world([0.25, 0.5], tick),
                before.drawing_at(tick).objects[0].local_to_world([0.25, 0.5], tick)
            );
            assert_eq!(
                objects.iter().any(|o| o.id == added),
                (400..900).contains(&tick)
            );
        }
        project.validate().unwrap();
    }

    #[test]
    fn layer_blank_keeps_nested_playback_and_the_original_group_mask_on_the_active_layer() {
        let mut p = AnimationProject::default();
        let child = p.add_clip("Move").unwrap();
        p.clips[child].set_length(5).unwrap();
        let child_art = p.clips[child].drawing_at_mut(0);
        child_art.add_rectangle([0.; 2], [12.; 2], [255; 4]);
        child_art.objects[0]
            .keys
            .extend([(0, [0.; 2]), (4, [30., 4.])]);
        p.clips[0].drawing_at_mut(0).layers.push(Layer {
            id: 2,
            name: "Other".into(),
            ..Layer::default()
        });
        let source = p.clips[child].id;
        let placed = p.place_clip(source, 0, 0, 1, [0.; 2]).unwrap();
        let other = p.place_clip(source, 0, 0, 2, [64., 0.]).unwrap();
        let d = p.clips[0].drawing_at_mut(0);
        let sibling = d.add_rectangle([2.; 2], [6.; 2], [150; 4]);
        for o in &mut d.objects {
            if [placed, sibling].contains(&o.id) {
                o.group = Some(42);
            }
        }
        let cutter = d.add_rectangle([0.; 2], [25.; 2], [255; 4]);
        d.set_clip(&[placed, cutter], 0, Some([120, 255])).unwrap();
        d.set_drawing_key(placed, 1, 3).unwrap();
        let original = d.clone();
        assert_eq!(p.clips[0].blank_layer_frame(2, 1), Ok(2));
        let d = p.clips[0].drawing_at_mut(2);
        let retained: Vec<_> = original
            .objects
            .iter()
            .filter(|o| o.id != sibling)
            .cloned()
            .collect();
        assert_eq!(d.objects, retained);
        for id in [placed, other] {
            let obj = d.objects.iter().find(|o| o.id == id).unwrap();
            let old = original.objects.iter().find(|o| o.id == id).unwrap();
            for tick in 2..5 {
                assert_eq!(
                    obj.instance_at(tick).unwrap().1,
                    old.instance_at(tick).unwrap().1
                );
            }
            let shared = |o: &crate::VectorObject| {
                o.clip_instance
                    .as_ref()
                    .unwrap()
                    .shared_clip()
                    .unwrap()
                    .clone()
            };
            assert!(Arc::ptr_eq(&shared(obj), &shared(old)));
        }
        assert!(d.release_clip(&[placed], 2).unwrap().contains(&cutter));
        assert!(p.clips[0]
            .drawing_at(0)
            .objects
            .iter()
            .any(|o| o.id == sibling));
        p.validate().unwrap();
    }

    #[test]
    fn layer_blank_protects_authored_boundaries_validates_layers_and_appends_only_at_the_end() {
        let mut p = AnimationProject::default();
        let c = &mut p.clips[0];
        c.set_length(2).unwrap();
        c.drawing_at_mut(0)
            .add_rectangle([1.; 2], [3.; 2], [255; 4]);
        c.cels.insert(1, c.cels[&0].clone());
        let before = c.clone();
        for (tick, layer) in [(0, 1), (0, 98), (2, 1)] {
            assert!(c.blank_layer_frame(tick, layer).is_err());
            assert_eq!(*c, before);
        }
        c.drawing_at_mut(1).layers[0].locked = true;
        let locked = c.clone();
        assert!(c.blank_layer_frame(1, 1).is_err());
        assert_eq!(*c, locked);
        c.drawing_at_mut(1).layers[0].locked = false;
        c.drawing_at_mut(1).objects.clear();
        let ready = c.clone();
        assert_eq!(c.blank_layer_frame(0, 1), Ok(1));
        assert_eq!(*c, ready);
        assert_eq!(c.blank_layer_frame(1, 1), Ok(2));
        assert_eq!(c.frames, 3);
        assert!(Arc::ptr_eq(&c.cels[&1], &c.cels[&2]));
        c.set_length(MAX_FRAMES).unwrap();
        assert_eq!(c.blank_layer_frame(MAX_FRAMES - 1, 1), Ok(MAX_FRAMES - 1));
        let full = c.clone();
        assert!(c.blank_layer_frame(MAX_FRAMES - 1, 1).is_err());
        assert_eq!(*c, full);
        c.validate().unwrap();
    }

    #[test]
    fn appending_an_fx_frame_resizes_shared_drawings_once_and_preserves_undo_snapshots() {
        let mut project = AnimationProject::default();
        let clip = &mut project.clips[0];
        clip.set_length(4).unwrap();
        let doc = clip.drawing_at_mut(0);
        doc.layers.push(Layer {
            id: 2,
            name: "FX".into(),
            ..Layer::default()
        });
        doc.add_rectangle([1.; 2], [4.; 2], [255; 4]);
        doc.objects[0].keys.insert(3, [8., 4.]);
        for start in [1, 2, 3] {
            clip.cels.insert(start, clip.cels[&0].clone());
        }
        clip.drawing_at_mut(2).objects[0].name = "Alternate pose".into();
        clip.add_event(2, "Beat").unwrap();
        let before = clip.clone();

        assert_eq!(clip.blank_layer_frame(3, 2), Ok(4));
        assert_eq!(clip.frames, 5);
        assert_eq!(before.frames, 4);
        assert_eq!(clip.events, before.events);
        for tick in [0, 1, 3, 4] {
            let drawing = &clip.cels[&tick];
            assert!(Arc::ptr_eq(drawing, &clip.cels[&0]));
            assert!(!Arc::ptr_eq(drawing, &before.cels[&0]));
            assert_eq!(drawing.frames, 5);
            assert_eq!(drawing.objects, before.cels[&0].objects);
        }
        assert_eq!(clip.cels[&2].objects, before.cels[&2].objects);
        assert!(before.cels.values().all(|d| d.frames == 4));
        let unique = Arc::as_ptr(&clip.cels[&2]);
        clip.set_length(6).unwrap();
        assert_eq!(unique, Arc::as_ptr(&clip.cels[&2]));
        let unchanged = clip.clone();
        clip.set_length(6).unwrap();
        assert!(unchanged
            .cels
            .iter()
            .all(|(at, d)| Arc::ptr_eq(d, &clip.cels[at])));
        before.validate().unwrap();
        clip.validate().unwrap();
    }

    #[test]
    fn layer_blank_from_a_drawing_start_advances_one_tick_and_keeps_other_symbol_instances() {
        let mut p = AnimationProject::default();
        p.clips[0].set_length(4).unwrap();
        let d = p.clips[0].drawing_at_mut(0);
        d.layers.push(Layer {
            id: 2,
            name: "Backdrop".into(),
            ..Layer::default()
        });
        let art = d.add_rectangle([0.; 2], [8.; 2], [255; 4]);
        let (symbol, first) = p.convert_to_symbol(0, 0, &[art], "Spark").unwrap();
        let other = p.place_symbol(symbol, 0, 0, 2, [44., 15.]).unwrap();
        let defs = p.symbols.clone();
        let c = &mut p.clips[0];
        assert_eq!(c.blank_layer_frame(0, 1), Ok(1));
        assert_eq!(c.blank_layer_frame(1, 1), Ok(2));
        assert_eq!(c.frames, 4);
        assert_eq!(c.drawing_at(0).objects[0].id, first);
        for f in [1, 2, 3] {
            let shapes = &c.drawing_at(f).objects;
            assert_eq!(shapes.len(), 1);
            assert_eq!(shapes[0].id, other);
            assert_eq!(&shapes[0], &c.drawing_at(0).objects[1]);
        }
        assert!(Arc::ptr_eq(&c.cels[&1], &c.cels[&2]));
        assert_eq!(p.symbols, defs);
        p.validate().unwrap();
    }
}

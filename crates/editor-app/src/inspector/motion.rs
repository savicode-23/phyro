use super::*;
use document_core::MotionPath;

fn attach(doc: &mut Document, target: u64, guide: u64, frame: u32) -> Result<(), String> {
    let source = doc
        .objects
        .iter()
        .find(|o| o.id == guide)
        .ok_or("Select a guide path.")?;
    if source.group.is_some() {
        return Err("Ungroup the guide before attaching it.".into());
    }
    let index = doc
        .objects
        .iter()
        .position(|o| o.id == target)
        .ok_or("Select the object to animate.")?;
    let path = MotionPath::from_guide_for(source, frame, &doc.objects[index])?;
    let first = path.nodes()[0].point;
    let target = &mut doc.objects[index];
    let center = target.unrig_point_at(target.local_to_world([0.5; 2], frame), frame);
    if target.motion.is_some() {
        return Err("Detach the existing guide before attaching a new one.".into());
    }
    let delta = [first[0] - center[0], first[1] - center[1]];
    for p in std::iter::once(&mut target.position).chain(target.keys.values_mut()) {
        p[0] += delta[0];
        p[1] += delta[1];
    }
    target.motion = Some(path);
    if doc.frames > 1 && target.animated.progress_keys.is_empty() {
        Property::MotionProgress.set(target, 0, KeyValue::Scalar(0.));
        Property::MotionProgress.set(target, doc.frames - 1, KeyValue::Scalar(1.));
    }
    doc.objects.retain(|o| o.id != guide);
    Ok(())
}

pub(super) fn show(
    ui: &mut Ui,
    doc: &mut Document,
    selected: &mut Vec<u64>,
    frame: u32,
    style: &mut CanvasStyle,
) {
    ui.collapsing("Motion path",|ui|{
        let notice=ui.make_persistent_id("motion-notice");
        if selected.len()==2 {
            let ids:Vec<_>=doc.paint_order().into_iter().filter(|o|selected.contains(&o.id)&&commands::is_editable(doc,o.layer)).map(|o|o.id).collect();
            if let [target,guide]=ids.as_slice(){
                if ui.button("Attach to upper path").on_hover_text("Turn the upper path into a motion guide. Its first node becomes the object's center at the start of travel.").clicked() {
                    match attach(doc,*target,*guide,frame) {
                        Ok(())=>{*selected=vec![*target];style.edit_motion=true;ui.data_mut(|d|d.remove::<String>(notice));}
                        Err(e)=>ui.data_mut(|d|d.insert_temp(notice,e)),
                    }
                }
                if let Some(err)=ui.data(|d|d.get_temp::<String>(notice)){ui.colored_label(ui.visuals().warn_fg_color,err);}
            }
            return;
        }
        let Some(o)=selected.first().filter(|_|selected.len()==1).and_then(|id|doc.objects.iter_mut().find(|o|o.id==*id)) else {
            ui.small("Draw a guide with the Pen. Select the object and its guide together.");return;
        };
        if o.motion.is_none(){ui.small("Draw a path above this object, then select both to attach it.");return;}
        let mut progress=o.progress_at(frame)*100.;
        ui.horizontal(|ui|{ui.label("Progress");
          if ui.add(egui::DragValue::new(&mut progress).range(0.0..=100.).suffix("%")).changed(){
              style.key_mode.set(o,frame,Property::MotionProgress,KeyValue::Scalar(progress/100.));
          }
          if ui.small_button("+ Key").on_hover_text("Key motion progress at the current frame").clicked(){Property::MotionProgress.add(o,frame);}
        });
        let path=o.motion.as_mut().unwrap();
        ui.checkbox(&mut path.align_tangent,"Face direction of travel");
        ui.checkbox(&mut path.reverse,"Reverse direction");
        ui.checkbox(&mut style.edit_motion,"Edit guide on canvas").on_hover_text("Drag square anchors and round control handles with Select or Nodes.");
        if ui.button("Detach at current pose").clicked() {
            let offset=o.motion_offset_at(frame);
            let heading=o.motion_heading_at(frame);
            o.motion=None;
            for p in std::iter::once(&mut o.position).chain(o.keys.values_mut()){p[0]+=offset[0];p[1]+=offset[1];}
            o.rotation+=heading;
            for angle in o.rotation_keys.values_mut(){*angle+=heading;}
            for key in Property::MotionProgress.frames(o){Property::MotionProgress.remove(o,key);}
            Property::MotionProgress.set_muted(o,false);
            o.animated.progress=0.;
            style.edit_motion=false;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::PathNode;
    #[test]
    fn attaching_to_posed_art_follows_the_drawn_world_curve() {
        let mut doc = Document::default();
        let art = doc.add_rectangle([56., 56.], [16., 16.], [255; 4]);
        let arm = doc.add_bone("Arm", None, [64., 64.], [80., 64.]).unwrap();
        doc.bind_rigid(art, Some(arm)).unwrap();
        doc.set_bone_rotation(arm, 90.).unwrap();
        let guide = doc.add_shape(
            [40., 50.],
            [24., 8.],
            [255; 4],
            Shape::Path {
                nodes: vec![PathNode::corner([0.; 2]), PathNode::corner([1.; 2])],
                closed: false,
            },
            1,
        );
        attach(&mut doc, art, guide, 0).unwrap();
        let o = &doc.objects[0];
        assert_eq!(o.local_to_world([0.5; 2], 0), [40., 50.]);
        assert_eq!(o.local_to_world([0.5; 2], 4), [64., 58.]);
        assert_eq!(o.rigid_bone, Some(arm));
    }

    #[test]
    fn guide_replaces_its_artwork_and_attaches_at_the_drawn_start() {
        let mut doc = Document::default();
        let id = doc.add_rectangle([11., 15.], [8., 6.], [255; 4]);
        let path = doc.add_shape(
            [20., 30.],
            [10., 10.],
            [255; 4],
            Shape::Path {
                nodes: vec![PathNode::corner([0.; 2]), PathNode::corner([1.; 2])],
                closed: false,
            },
            1,
        );
        attach(&mut doc, id, path, 0).unwrap();
        assert_eq!(doc.objects.len(), 1);
        assert_eq!(doc.objects[0].position_at(0), [16., 27.]);
        assert_eq!(doc.objects[0].position_at(4), [26., 37.]);
        doc.validate().unwrap();
    }
}

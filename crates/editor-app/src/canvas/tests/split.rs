use super::*;
use egui::epaint::{Mesh, Primitive};

fn paint_nodes(s: &mut CanvasSession, events: Vec<egui::Event>) -> (bool, Vec<Mesh>, bool) {
    let (edited, out) = s.paint(egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
        modifiers: egui::Modifiers::COMMAND,
        events,
        ..Default::default()
    });
    let artwork = s.canvas.texture.as_ref().unwrap().handle.id();
    let parts = s.ctx.tessellate(out.shapes, out.pixels_per_point);
    let mut parts = parts
        .into_iter()
        .skip_while(|p| !matches!(&p.primitive, Primitive::Mesh(m) if m.texture_id == artwork));
    let canvas = parts.next().unwrap().clip_rect;
    let controls = parts
        .filter(|p| p.clip_rect == canvas)
        .filter_map(|p| match p.primitive {
            Primitive::Mesh(m) => Some(m),
            _ => None,
        })
        .collect();
    let uploaded = out.textures_delta.set.iter().any(|(id, _)| *id == artwork);
    (edited, controls, uploaded)
}

#[test]
fn splitting_a_curve_updates_warm_node_controls_and_snap_targets_on_press() {
    let mut s = CanvasSession::new(Tool::Node);
    s.doc.width = 64;
    s.doc.height = 64;
    let mut a = PathNode::corner([0.25, 0.5]);
    let mut b = PathNode::corner([0.75, 0.5]);
    a.outgoing = [0.375, 0.25];
    b.incoming = [0.625, 0.75];
    let shape = Shape::Compound {
        contours: vec![
            vec![a, b, PathNode::corner([0.5, 0.85])],
            vec![PathNode::corner([3., 3.]); 2048],
        ],
    };
    let id = s.doc.add_shape([0.; 2], [64.; 2], [180; 4], shape, 1);
    s.selected = vec![id];
    crate::path_editor::select(&s.ctx, id, 0, 0, 1);
    let center = egui::pos2(200., 200.);
    paint_nodes(&mut s, vec![egui::Event::PointerMoved(center)]);
    paint_nodes(&mut s, vec![]);
    let (_, warm, _) = paint_nodes(&mut s, vec![]);
    let before_split = s.doc.clone();

    let (edited, immediate, _) = paint_nodes(
        &mut s,
        vec![egui::Event::PointerButton {
            pos: center,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    assert!(edited);
    // A refusal arrives after the topology edit but before the exact pixels.
    s.canvas.refinement.as_mut().unwrap().inject_error();
    for _ in 0..3 {
        let (_, controls, uploaded) = paint_nodes(&mut s, vec![]);
        assert_eq!(immediate, controls);
        assert!(!uploaded);
        assert!(s.canvas.refinement.is_none());
        assert!(s
            .canvas
            .refinement_warning()
            .unwrap()
            .contains("Injected renderer failure"));
    }
    let cached = s.canvas.texture.as_ref().unwrap();
    assert_ne!(cached.document.as_ref(), &s.doc);
    assert_eq!(cached.authored().as_ref(), &s.doc);
    let point = s.doc.objects[0].shape.contours()[0][1]
        .point
        .map(|p| p * 64.);
    let settings = SnapSettings {
        enabled: true,
        anchors: true,
        ..Default::default()
    };
    let hit = snapping::PenTargets::after_paint(
        &mut None,
        point.into(),
        cached.authored(),
        0,
        settings,
        cached.scale,
        egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(64., 64.)),
    );
    assert_eq!(hit.anchor, Some(point.into()));
    assert_ne!(immediate, warm);
    s.canvas.nodes.clear();
    assert_eq!(immediate, paint_nodes(&mut s, vec![]).1);
    s.canvas.zoom_steps += 0.25;
    assert!(!paint_nodes(&mut s, vec![]).2);
    let cached = s.canvas.texture.as_ref().unwrap();
    assert!(s.canvas.refinement.is_some());
    assert_eq!(cached.document.as_ref(), &before_split);
    assert_eq!(cached.authored().as_ref(), &s.doc);
    s.doc = before_split;
    paint_nodes(&mut s, vec![]);
    assert!(s.canvas.refinement_warning().is_none());
    let cached = s.canvas.texture.as_ref().unwrap();
    assert!(cached.pending_split.is_none());
    assert_eq!(cached.document.as_ref(), &s.doc);
}

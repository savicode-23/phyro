use super::*;
use crate::path_editor::NodeSet;
use document_core::VectorObject;
use egui::epaint::{CircleShape, ClippedShape, Primitive, RectShape, TessellationOptions};
use std::sync::{Arc, Weak};

const MAX_MESH_BYTES: usize = 64 * 1024 * 1024;

fn allocated_bytes(mesh: &egui::Mesh) -> usize {
    std::mem::size_of::<egui::Mesh>()
        + mesh.vertices.capacity() * std::mem::size_of::<egui::epaint::Vertex>()
        + mesh.indices.capacity() * std::mem::size_of::<u32>()
}

#[derive(PartialEq)]
struct Layout {
    object: u64,
    frame: u32,
    range: NodeSet,
    board: Rect,
    scale: f32,
    clip: Rect,
    ppp: f32,
    atlas_size: [usize; 2],
    tessellation: TessellationOptions,
}

enum MeshState {
    Pending,
    Ready(Vec<egui::Shape>),
    TooLarge,
}

struct Cached {
    layout: Layout,
    source: Weak<Document>,
    state: MeshState,
}

#[derive(Default)]
pub(in crate::canvas) struct NodeOverlay {
    cached: Option<Cached>,
}

impl NodeOverlay {
    pub(in crate::canvas) fn clear(&mut self) {
        self.cached = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::canvas) fn paint(
        &mut self,
        p: &egui::Painter,
        view: View,
        doc: &Document,
        frame: u32,
        ids: &[u64],
        source: Option<&Arc<Document>>,
        preview: Option<(u64, &NodeSet)>,
    ) {
        if ids.len() != 1 {
            self.clear();
        }
        for object in visible_selection(doc, ids) {
            let contours = object.shape.contours();
            if object.text.is_some() || contours.is_empty() {
                self.clear();
                frame_outline(p, view, object, frame, false);
                continue;
            }
            let range = preview
                .filter(|(id, _)| *id == object.id)
                .map(|(_, nodes)| nodes.clone())
                .unwrap_or_else(|| crate::path_editor::selected_nodes(p.ctx(), object.id, frame));
            let tessellation = p.ctx().tessellation_options(|t| *t);
            let clip = if tessellation.debug_ignore_clip_rects {
                Rect::EVERYTHING
            } else {
                // Include the selection ring, stroke and antialias fringe.
                p.clip_rect().expand(
                    7. + tessellation.feathering_size_in_pixels.max(2.) / p.pixels_per_point(),
                )
            };
            let cacheable = source.filter(|_| {
                ids.len() == 1
                    && contours.iter().map(Vec::len).sum::<usize>() >= 512
                    && !tessellation.debug_ignore_clip_rects
                    && !tessellation.debug_paint_clip_rects
            });
            let Some(source) = cacheable else {
                self.clear();
                p.extend(marks(object, frame, view, &range, clip));
                continue;
            };
            let layout = Layout {
                object: object.id,
                frame,
                range: range.clone(),
                board: view.board,
                scale: view.scale,
                clip: p.clip_rect(),
                ppp: p.pixels_per_point(),
                atlas_size: p.ctx().fonts(|f| f.font_image_size()),
                tessellation,
            };
            if let Some(previous) = self
                .cached
                .as_mut()
                .filter(|v| v.source.as_ptr() == Arc::as_ptr(source) && v.layout == layout)
            {
                match &previous.state {
                    MeshState::Ready(meshes) => p.extend(meshes.iter().cloned()),
                    MeshState::TooLarge => p.extend(marks(object, frame, view, &range, clip)),
                    MeshState::Pending => {
                        let shapes = marks(object, frame, view, &range, clip)
                            .into_iter()
                            .map(|shape| ClippedShape {
                                clip_rect: layout.clip,
                                shape,
                            })
                            .collect();
                        let mut bytes = 0;
                        let meshes: Vec<_> = p
                            .ctx()
                            .tessellate(shapes, layout.ppp)
                            .into_iter()
                            .map(|pr| match pr.primitive {
                                Primitive::Mesh(mut mesh) => {
                                    if allocated_bytes(&mesh) > MAX_MESH_BYTES
                                        && mesh.bytes_used() <= MAX_MESH_BYTES
                                    {
                                        mesh.vertices.shrink_to_fit();
                                        mesh.indices.shrink_to_fit();
                                    }
                                    bytes += allocated_bytes(&mesh);
                                    egui::Shape::Mesh(Arc::new(mesh))
                                }
                                Primitive::Callback(_) => unreachable!(
                                    "node controls consist of lines, rectangles, and circles"
                                ),
                            })
                            .collect();
                        p.extend(meshes.iter().cloned());
                        previous.state = if bytes <= MAX_MESH_BYTES {
                            MeshState::Ready(meshes)
                        } else {
                            MeshState::TooLarge
                        };
                    }
                }
            } else {
                // While the path, selection or viewport is changing, paint directly.
                // Build the mesh only when another frame confirms the same inputs.
                self.cached = Some(Cached {
                    layout,
                    source: Arc::downgrade(source),
                    state: MeshState::Pending,
                });
                p.extend(marks(object, frame, view, &range, clip));
            }
        }
    }
}

#[cfg(test)]
mod tests;

fn marks(
    object: &VectorObject,
    frame: u32,
    view: View,
    range: &NodeSet,
    clip: Rect,
) -> Vec<egui::Shape> {
    let world = object.local_to_world_at(frame);
    let screen = |local| view.screen(world(local).into());
    let mut marks = Vec::new();
    let mut selected_anchors = Vec::new();
    for (contour, nodes) in object.skinned_shape_at(frame).contours().iter().enumerate() {
        for (index, n) in nodes.iter().enumerate() {
            let anchor = screen(n.point);
            for h in [n.incoming, n.outgoing] {
                let handle = screen(h);
                if anchor.distance(handle) > 2. {
                    // A handle line can cross the viewport with both endpoints offscreen.
                    if clip.intersects(Rect::from_two_pos(anchor, handle)) {
                        marks.push(egui::Shape::line_segment([anchor, handle], (1., ACCENT)));
                    }
                    if clip.contains(handle) {
                        marks.push(egui::Shape::Circle(CircleShape {
                            center: handle,
                            radius: 3.,
                            fill: Color32::WHITE,
                            stroke: (1., ACCENT).into(),
                        }));
                    }
                }
            }
            if clip.contains(anchor) {
                let selected = range.contains(contour, index);
                let anchor_mark = egui::Shape::Rect(RectShape::new(
                    Rect::from_center_size(anchor, Vec2::splat(7.)),
                    0.,
                    if selected { ACCENT } else { Color32::WHITE },
                    (1., ACCENT),
                    egui::StrokeKind::Inside,
                ));
                if selected {
                    selected_anchors.push(anchor_mark);
                    selected_anchors.push(egui::Shape::circle_stroke(
                        anchor,
                        5.,
                        (1.5, Color32::WHITE),
                    ));
                } else {
                    marks.push(anchor_mark);
                }
            }
        }
    }
    marks.extend(selected_anchors);
    marks
}

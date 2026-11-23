use crate::{PathNode, Shape, VectorObject, MAX_PATH_NODES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathEnd {
    Start,
    End,
}

impl VectorObject {
    fn continuable_nodes(&self) -> Option<&[PathNode]> {
        if self.text.is_some()
            || self.image.is_some()
            || self.is_instance()
            || self.skin.is_some()
            || !self.animated.geometry_keys.is_empty()
            || !self.effects.is_empty()
        {
            return None;
        }
        match &self.shape {
            Shape::Path {
                nodes,
                closed: false,
            } if nodes.len() >= 2 => Some(nodes),
            _ => None,
        }
    }

    /// The start and end of an extendable path, in artboard coordinates.
    /// Handles retain their authored orientation, including the unused tips.
    pub fn pen_endpoints_at(&self, frame: u32) -> Option<[PathNode; 2]> {
        let nodes = self.continuable_nodes()?;
        let world = self.local_to_world_at(frame);
        let ends = [nodes[0], *nodes.last()?].map(|n| PathNode {
            point: world(n.point),
            incoming: world(n.incoming),
            outgoing: world(n.outgoing),
            ..n
        });
        ends.iter()
            .flat_map(|n| [n.point, n.incoming, n.outgoing])
            .flatten()
            .all(f32::is_finite)
            .then_some(ends)
    }

    /// Extend the same open path from either end. All controls and new anchors
    /// are in artboard space in the direction the artist drew them. The two
    /// existing anchors are excluded from `added`. An incoming control at the
    /// opposite endpoint completes a closed contour without duplicating it.
    pub fn extend_open_path(
        &mut self,
        frame: u32,
        from: PathEnd,
        join_outgoing: [f32; 2],
        added: &[PathNode],
        close_incoming: Option<[f32; 2]>,
    ) -> bool {
        let Some(original) = self.continuable_nodes() else {
            return false;
        };
        let closed = close_incoming.is_some();
        if added.len() > MAX_PATH_NODES.saturating_sub(original.len())
            || (added.is_empty() && !closed)
            || (closed && original.len() + added.len() < 3)
        {
            return false;
        }
        let from_start = from == PathEnd::Start;
        let (join, opposite) = if from_start {
            (0, original.len() - 1)
        } else {
            (original.len() - 1, 0)
        };
        let mut nodes = Vec::with_capacity(original.len() + added.len());
        let world = self.local_to_world_at(frame);
        let local = self.world_to_local_at(frame);
        // Preserve existing local handle bits when a plain endpoint click leaves
        // a handle untouched. A round trip through a rotated world pose can round it.
        let set_tip = |node: &mut PathNode, incoming, point| {
            let tip = if incoming {
                &mut node.incoming
            } else {
                &mut node.outgoing
            };
            if point != world(*tip) {
                *tip = local(point);
                crate::segment_bend::retain_compatible_style(node);
            }
        };
        let local_nodes = added.iter().map(|n| PathNode {
            point: local(n.point),
            incoming: local(n.incoming),
            outgoing: local(n.outgoing),
            ..*n
        });
        let original_start = if from_start { added.len() } else { 0 };
        if from_start {
            nodes.extend(local_nodes.rev().map(|mut n| {
                std::mem::swap(&mut n.incoming, &mut n.outgoing);
                n
            }));
            nodes.extend_from_slice(original);
        } else {
            nodes.extend_from_slice(original);
            nodes.extend(local_nodes);
        }
        set_tip(&mut nodes[original_start + join], from_start, join_outgoing);
        if let Some(point) = close_incoming {
            set_tip(&mut nodes[original_start + opposite], !from_start, point);
        }
        let shape = Shape::Path { nodes, closed };
        if !shape.valid_nodes(self.size) {
            return false;
        }
        let original_range = original_start..original_start + original.len();
        let Some(profile) = self.extended_pressure_profile(frame, &shape, original_range, closed)
        else {
            return false;
        };
        self.shape = shape;
        self.stroke_profile = profile;
        true
    }
}

#[cfg(test)]
mod tests;

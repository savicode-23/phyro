use super::{can_edit_topology, KeyMode};
use document_core::{Property, Shape, VectorObject};
use eframe::egui::{self, Pos2, Rect};
use std::sync::Arc;

// Sorted, nonoverlapping runs keep a large contiguous selection cheap to paint
// and to share with the overlay cache and an in-progress drag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeSet {
    Range((usize, usize, usize)),
    Runs(Arc<[(usize, usize, usize)]>),
}

impl NodeSet {
    pub(crate) fn range(contour: usize, a: usize, b: usize) -> Self {
        Self::Range((contour, a.min(b), a.max(b)))
    }

    pub(crate) fn runs(&self) -> &[(usize, usize, usize)] {
        match self {
            Self::Range(run) => std::slice::from_ref(run),
            Self::Runs(runs) => runs,
        }
    }

    pub(crate) fn contains(&self, contour: usize, node: usize) -> bool {
        let runs = self.runs();
        let i = runs.partition_point(|&(c, _, last)| (c, last) < (contour, node));
        runs.get(i)
            .is_some_and(|&(c, first, _)| c == contour && first <= node)
    }

    pub(crate) fn with_node(&self, contour: usize, node: usize) -> Self {
        if self.contains(contour, node) {
            return self.clone();
        }
        let mut runs = self.runs().to_vec();
        let i = runs.partition_point(|&(c, first, _)| (c, first) < (contour, node));
        runs.insert(i, (contour, node, node));
        let mut merged: Vec<(usize, usize, usize)> = Vec::with_capacity(runs.len());
        for (c, first, last) in runs {
            if let Some(run) = merged.last_mut().filter(|r| r.0 == c && r.2 + 1 == first) {
                run.2 = last;
            } else {
                merged.push((c, first, last));
            }
        }
        match merged.as_slice() {
            [only] => Self::Range(*only),
            _ => Self::Runs(merged.into()),
        }
    }

    fn delete_from(&self, shape: &mut Shape) {
        for group in self.runs().chunk_by(|a, b| a.0 == b.0) {
            let mut runs = group.iter().peekable();
            let mut i = 0;
            shape.contours_mut()[group[0].0].retain(|_| {
                let remove = runs.peek().is_some_and(|&&(_, a, _)| a <= i);
                if runs.peek().is_some_and(|&&(_, _, b)| i == b) {
                    runs.next();
                }
                i += 1;
                !remove
            });
        }
    }

    pub(super) fn reverse_contour(&mut self, contour: usize, len: usize) {
        let reverse = |(c, a, b)| {
            if c == contour {
                (c, len - 1 - b, len - 1 - a)
            } else {
                (c, a, b)
            }
        };
        match self {
            Self::Range(run) => *run = reverse(*run),
            Self::Runs(runs) => {
                let runs = Arc::make_mut(runs);
                for run in runs.iter_mut() {
                    *run = reverse(*run);
                }
                runs.sort_unstable();
            }
        }
    }

    pub(super) fn len(&self) -> usize {
        self.runs().iter().map(|&(_, a, b)| b - a + 1).sum()
    }

    pub(super) fn valid(&self, shape: &Shape) -> bool {
        !self.runs().is_empty()
            && self
                .runs()
                .iter()
                .all(|&(c, _, b)| shape.contours().get(c).is_some_and(|nodes| b < nodes.len()))
    }

    pub(crate) fn in_box(
        object: &VectorObject,
        frame: u32,
        rect: Rect,
        previous: Option<&Self>,
    ) -> Self {
        let mut runs = Vec::new();
        let world = object.local_to_world_at(frame);
        for (contour, nodes) in object.skinned_shape_at(frame).contours().iter().enumerate() {
            let mut start = None;
            for (i, node) in nodes.iter().enumerate() {
                if rect.contains(Pos2::from(world(node.point)))
                    || previous.is_some_and(|p| p.contains(contour, i))
                {
                    start.get_or_insert(i);
                } else if let Some(a) = start.take() {
                    runs.push((contour, a, i - 1));
                }
            }
            if let Some(a) = start {
                runs.push((contour, a, nodes.len() - 1));
            }
        }
        match runs.as_slice() {
            [only] => Self::Range(*only),
            _ => Self::Runs(runs.into()),
        }
    }
}

#[derive(Clone)]
pub(super) struct NodeSelection {
    pub(super) object: u64,
    pub(super) frame: u32,
    pub(super) contour: usize,
    pub(super) anchor: usize,
    pub(super) end: usize,
    pub(super) nodes: NodeSet,
}

impl NodeSelection {
    pub(super) fn load(ctx: &egui::Context, object: u64, frame: u32) -> Self {
        ctx.data(|d| d.get_temp::<Self>(selection_key(d)))
            .filter(|s| s.object == object && s.frame == frame)
            .unwrap_or(Self {
                object,
                frame,
                contour: 0,
                anchor: 0,
                end: 0,
                nodes: NodeSet::range(0, 0, 0),
            })
    }

    pub(super) fn set_range(&mut self, contour: usize, anchor: usize, end: usize) {
        self.contour = contour;
        self.anchor = anchor;
        self.end = end;
        self.nodes = NodeSet::range(contour, anchor, end);
    }

    pub(super) fn can_delete(&self, object: &VectorObject, mode: KeyMode) -> bool {
        if object.text.is_some()
            || object.is_instance()
            || !object.visible_at(self.frame)
            || !can_edit_topology(object, mode)
            || Property::Geometry.is_muted(object)
            || !self.nodes.valid(&object.shape)
        {
            return false;
        }
        let minimum = if matches!(object.shape, Shape::Path { closed: false, .. }) {
            2
        } else {
            3
        };
        self.nodes.runs().chunk_by(|a, b| a.0 == b.0).all(|runs| {
            let removed: usize = runs.iter().map(|(_, a, b)| b - a + 1).sum();
            object.shape.contours()[runs[0].0].len() >= minimum + removed
        })
    }

    pub(super) fn delete_from(&mut self, shape: &mut Shape) {
        let (contour, first, _) = self.nodes.runs()[0];
        self.nodes.delete_from(shape);
        let next = first.min(shape.contours()[contour].len() - 1);
        self.set_range(contour, next, next);
    }

    pub(super) fn save(self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_temp(selection_key(d), self));
    }
}

// Drawings in a symbol editor can reuse the main scene's object and frame numbers.
fn selection_key(data: &egui::util::IdTypeMap) -> egui::Id {
    egui::Id::new((
        "selected-path-nodes",
        data.get_temp::<u64>(egui::Id::new("node-canvas")),
    ))
}

pub(crate) fn set_nodes(ctx: &egui::Context, object: u64, frame: u32, nodes: NodeSet) {
    let mut s = NodeSelection::load(ctx, object, frame);
    if let Some(&(contour, first, last)) = nodes.runs().first() {
        s.set_range(contour, first, last);
    }
    s.nodes = nodes;
    s.save(ctx);
}

pub(crate) fn selected_nodes(ctx: &egui::Context, object: u64, frame: u32) -> NodeSet {
    NodeSelection::load(ctx, object, frame).nodes
}

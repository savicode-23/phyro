use super::{CanvasStyle, Tool};
use document_core::{Gradient, LineCap, LineJoin};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DrawingPaint {
    fill: [u8; 4],
    no_fill: bool,
    gradient: Option<Arc<Gradient>>,
    stroke: [u8; 4],
    no_stroke: bool,
    stroke_gradient: Option<Arc<Gradient>>,
    width: f32,
    cap: LineCap,
    join: LineJoin,
}

impl Default for DrawingPaint {
    fn default() -> Self {
        Self {
            fill: [88, 209, 167, 255],
            no_fill: false,
            gradient: None,
            stroke: [35, 40, 48, 255],
            no_stroke: false,
            stroke_gradient: None,
            width: 1.5,
            cap: LineCap::default(),
            join: LineJoin::default(),
        }
    }
}

impl DrawingPaint {
    pub(crate) fn read(style: &CanvasStyle) -> Self {
        Self {
            fill: style.fill,
            no_fill: style.no_fill,
            gradient: style.gradient.clone(),
            stroke: style.stroke,
            no_stroke: style.no_stroke,
            stroke_gradient: style.stroke_gradient.clone(),
            width: style.stroke_width,
            cap: style.cap,
            join: style.join,
        }
    }

    fn apply(&self, style: &mut CanvasStyle) {
        style.fill = self.fill;
        style.no_fill = self.no_fill;
        style.gradient.clone_from(&self.gradient);
        style.stroke = self.stroke;
        style.no_stroke = self.no_stroke;
        style.stroke_gradient.clone_from(&self.stroke_gradient);
        style.stroke_width = self.width;
        style.cap = self.cap;
        style.join = self.join;
    }

    fn sanitize(&mut self) {
        if !self.width.is_finite() || self.width < 0. {
            self.width = Self::default().width;
        }
        self.gradient.take_if(|g| !g.valid());
        self.stroke_gradient.take_if(|g| !g.valid());
    }

    // A deliberate change to one channel on a selected object should not replace
    // the rest of the drawing tool's style. Keep no-paint together with its paint.
    fn merge(&mut self, old: &Self, new: &Self) {
        if (old.fill, old.no_fill, &old.gradient) != (new.fill, new.no_fill, &new.gradient) {
            self.fill = new.fill;
            self.no_fill = new.no_fill;
            self.gradient.clone_from(&new.gradient);
        }
        if (old.stroke, old.no_stroke, &old.stroke_gradient)
            != (new.stroke, new.no_stroke, &new.stroke_gradient)
        {
            self.stroke = new.stroke;
            self.no_stroke = new.no_stroke;
            self.stroke_gradient.clone_from(&new.stroke_gradient);
        }
        if old.width != new.width {
            self.width = new.width;
        }
        if old.cap != new.cap {
            self.cap = new.cap;
        }
        if old.join != new.join {
            self.join = new.join;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PathPaintMemory {
    standard: DrawingPaint,
    pencil: Option<DrawingPaint>,
    #[serde(skip)]
    last_was_pencil: bool,
}

impl PathPaintMemory {
    pub(crate) fn is_drawing_tool(tool: Tool) -> bool {
        matches!(
            tool,
            Tool::Pen | Tool::Pencil | Tool::Rectangle | Tool::Ellipse | Tool::Text
        )
    }

    fn first_pencil(standard: &DrawingPaint) -> DrawingPaint {
        let mut pencil = standard.clone();
        pencil.no_fill = true;
        pencil.no_stroke = false;
        if pencil.stroke[3] == 0 && pencil.stroke_gradient.is_none() {
            pencil.stroke = DrawingPaint::default().stroke;
        }
        if pencil.width == 0. {
            pencil.width = DrawingPaint::default().width;
        }
        pencil
    }

    fn current_mut(&mut self) -> &mut DrawingPaint {
        if self.last_was_pencil {
            self.pencil
                .get_or_insert_with(|| Self::first_pencil(&self.standard))
        } else {
            &mut self.standard
        }
    }

    pub(crate) fn remember(&mut self, tool: Tool, style: &CanvasStyle) {
        if Self::is_drawing_tool(tool) {
            self.last_was_pencil = tool == Tool::Pencil;
            *self.current_mut() = DrawingPaint::read(style);
        }
    }

    pub(crate) fn restore(&mut self, tool: Tool, style: &mut CanvasStyle) {
        if Self::is_drawing_tool(tool) {
            self.last_was_pencil = tool == Tool::Pencil;
        }
        self.current_mut().apply(style);
    }

    pub(crate) fn switch(&mut self, from: Tool, to: Tool, style: &mut CanvasStyle) {
        if from == to {
            return;
        }
        self.remember(from, style);
        if Self::is_drawing_tool(to) {
            self.restore(to, style);
        }
    }

    pub(crate) fn record_edit(&mut self, before: &DrawingPaint, after: &DrawingPaint) {
        self.current_mut().merge(before, after);
    }

    pub(crate) fn adopt_style(&mut self, style: &CanvasStyle) {
        *self.current_mut() = DrawingPaint::read(style);
    }

    pub(super) fn sampled(&mut self, sample: super::eyedropper::PaintSample, style: &CanvasStyle) {
        let mut drawing = style.clone();
        self.current_mut().apply(&mut drawing);
        sample.set_default(&mut drawing);
        *self.current_mut() = DrawingPaint::read(&drawing);
    }

    pub(crate) fn sanitize(&mut self) {
        self.standard.sanitize();
        if let Some(pencil) = &mut self.pencil {
            pencil.sanitize();
        }
    }

    pub(crate) fn merge(&mut self, old: &Self, new: &Self) {
        self.standard.merge(&old.standard, &new.standard);
        if let Some(new_pencil) = &new.pencil {
            if let Some(previous) = &mut self.pencil {
                let initial = old
                    .pencil
                    .clone()
                    .unwrap_or_else(|| Self::first_pencil(&old.standard));
                previous.merge(&initial, new_pencil);
            } else {
                self.pencil = Some(new_pencil.clone());
            }
        }
    }
}

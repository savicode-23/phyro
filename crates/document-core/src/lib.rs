//! Authored artwork and frame evaluation, in top-left-origin artboard coordinates.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

pub mod animation;
pub mod arrange;
pub mod boolean;
pub use boolean::BooleanOp;
mod blend;
pub use blend::BlendMode;
mod composition;
pub use composition::{ClipInstance, ClipTiming, LoopMode};
mod symbols;
pub use symbols::{EmbeddedArtwork, SymbolDefinition, SymbolInstance};
mod appearance;
mod clipping;
pub use clipping::{ClipBinding, ClipSource};
mod bezier;
mod geometry;
mod grid;
pub use grid::{GridGeometry, GridKind, GridSettings, GridVisibility};
mod guided_geometry;
pub mod morph;
pub use guided_geometry::GuidedGeometry;
mod raster;
pub use raster::RasterImage;
mod pressure;
pub mod typography;
pub use appearance::{Gradient, GradientKind, GradientStop, Stencil};
pub mod path_edit;
pub mod path_effects;
mod segment_bend;
pub use animation::{
    AnimationProject, AnimationSet, Clip, ExportPreset, PackedOptions, DEFAULT_ANIMATION_SET_ID,
};
pub use path_effects::PathEffect;
mod rig;
pub use rig::{Bone, BoneSegment, FootPin, PlantedSole, Skeleton, SkinBinding, MAX_BONES};
mod motion;
pub use motion::MotionPath;
mod easing;
mod properties;
mod tracks;
pub use easing::Easing;
pub use properties::{AnimatedProperties, KeyValue, Property};
pub use tracks::Interpolation;

pub const FORMAT_VERSION: u32 = 2;
pub const MAX_DIMENSION: u32 = 4096;
/// Maximum pixel count of a rendered PNG or sprite sheet.
pub const MAX_RASTER_PIXELS: u64 = 16_777_216;
pub const MAX_FRAMES: u32 = 4096;
pub const MAX_FPS: u32 = 240;
pub const MAX_PATH_NODES: usize = 65_536;
/// Maximum number of normalized distance/width samples on a Pencil stroke.
pub const MAX_STROKE_SAMPLES: usize = 256;
const MAX_OBJECT_NAME_CHARS: usize = 80;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub format_version: u32,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub fps: u32,
    #[serde(default, skip_serializing_if = "Skeleton::is_default")]
    pub skeleton: Skeleton,
    /// Back-to-front paint order. IDs survive changes to order.
    pub objects: Vec<VectorObject>,
    /// Back-to-front layer order. Object order is stable within each layer.
    #[serde(default = "default_layers")]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub guides_x: Vec<f32>,
    #[serde(default)]
    pub guides_y: Vec<f32>,
    /// Persist the allocation cursor so deletion and reopening preserve identity.
    pub next_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            width: 128,
            height: 128,
            frames: 5,
            fps: 12,
            skeleton: Skeleton::default(),
            objects: Vec::new(),
            layers: default_layers(),
            guides_x: Vec::new(),
            guides_y: Vec::new(),
            next_id: 1,
        }
    }
}

impl Document {
    fn editable_object_index(&self, object_id: u64) -> Result<usize, String> {
        let index = self
            .objects
            .iter()
            .position(|o| o.id == object_id)
            .ok_or("Select one artwork object.")?;
        if !arrange::is_object_editable(self, &self.objects[index]) {
            return Err("Select artwork on a visible, unlocked layer.".into());
        }
        Ok(index)
    }

    /// Name a group in this drawing. Empty input restores the generated label.
    pub fn rename_group(&mut self, layer: u64, group: u64, name: &str) -> Result<(), String> {
        if !self
            .layers
            .iter()
            .any(|l| l.id == layer && l.visible && !l.locked)
        {
            return Err("Choose a group on a visible, unlocked layer.".into());
        }
        let name = name.trim();
        if name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err("Group names can contain up to 80 printable characters.".into());
        }
        let label: Option<Arc<str>> = (!name.is_empty()).then(|| Arc::from(name));
        let mut found = false;
        for object in self
            .objects
            .iter_mut()
            .filter(|o| o.layer == layer && o.group == Some(group))
        {
            object.group_name.clone_from(&label);
            found = true;
        }
        if found {
            Ok(())
        } else {
            Err("This group no longer exists.".into())
        }
    }

    pub fn add_rectangle(&mut self, position: [f32; 2], size: [f32; 2], fill: [u8; 4]) -> u64 {
        let layer = self.layers.first().map_or(1, |layer| layer.id);
        self.add_shape(position, size, fill, Shape::Rectangle, layer)
    }

    pub fn add_shape(
        &mut self,
        position: [f32; 2],
        size: [f32; 2],
        fill: [u8; 4],
        shape: Shape,
        layer: u64,
    ) -> u64 {
        let id = self.next_id;
        self.next_id = id.wrapping_add(1).max(1);
        self.objects.push(VectorObject {
            id,
            name: String::new(),
            rigid_bone: None,
            skin: None,
            rig_offset: rig::RigidPose::default(),
            rig_pose: rig::PoseTrack::default(),
            symbol: None,
            embedded_artwork: None,
            clip_instance: None,
            position,
            size,
            pivot: None,
            fill,
            fill_blend_mode: BlendMode::Normal,
            stroke_blend_mode: BlendMode::Normal,
            blend_mode: BlendMode::Normal,
            no_fill: false,
            no_stroke: false,
            fill_rule: FillRule::default(),
            keys: BTreeMap::new(),
            rotation_keys: BTreeMap::new(),
            scale_keys: BTreeMap::new(),
            opacity_keys: BTreeMap::new(),
            scale: unit_scale(),
            opacity: opaque(),
            interpolation: Interpolation::Linear,
            animated: AnimatedProperties::default(),
            effects: Vec::new(),
            gradient: None,
            stroke_gradient: None,
            stencil: None,
            clip_source: None,
            shape,
            motion: None,
            text: None,
            image: None,
            reference_only: false,
            image_resource: None,
            rotation: 0.0,
            layer,
            locked: false,
            group: None,
            group_name: None,
            stroke: [0, 0, 0, 255],
            stroke_width: 0.0,
            stroke_profile: Vec::new(),
            cap: LineCap::default(),
            join: LineJoin::default(),
        });
        // Normally IDs increase monotonically. A loaded document can also
        // reach the end of the ID space; continue at the next unused ID.
        while self.objects.iter().any(|rect| rect.id == self.next_id) {
            self.next_id = self.next_id.wrapping_add(1).max(1);
        }
        id
    }

    /// Visible objects ordered from back to front, including locked artwork.
    pub fn paint_order(&self) -> Vec<&VectorObject> {
        let mut by_layer: BTreeMap<u64, Vec<&VectorObject>> = BTreeMap::new();
        for object in &self.objects {
            by_layer.entry(object.layer).or_default().push(object);
        }
        let mut objects = Vec::with_capacity(self.objects.len());
        for layer in self.layers.iter().filter(|l| l.visible) {
            if let Some(mut contents) = by_layer.remove(&layer.id) {
                objects.append(&mut contents);
            }
        }
        objects
    }

    /// The frontmost editable object painted at the artboard point.
    pub fn hit_test(&self, point: [f32; 2], frame: u32) -> Option<u64> {
        self.hit_test_in(point, frame, arrange::SelectionScope::Groups)
    }

    pub fn hit_test_in(
        &self,
        point: [f32; 2],
        frame: u32,
        scope: arrange::SelectionScope,
    ) -> Option<u64> {
        self.hit_test_with(|obj| scope.contains(obj) && obj.contains(point, frame))
            .map(|obj| obj.id)
    }

    /// Pick lettering anywhere inside its layout box, including a space between glyphs.
    /// Other painted objects in front still intercept the click.
    pub fn text_at(
        &self,
        point: [f32; 2],
        frame: u32,
    ) -> Option<(&VectorObject, &typography::TextStyle)> {
        self.text_at_in(point, frame, arrange::SelectionScope::Groups)
    }

    pub fn text_at_in(
        &self,
        point: [f32; 2],
        frame: u32,
        scope: arrange::SelectionScope,
    ) -> Option<(&VectorObject, &typography::TextStyle)> {
        self.hit_test_with(|obj| {
            scope.contains(obj)
                && (obj.text.is_some()
                    && obj.visible_at(frame)
                    && obj.opacity_at(frame) > 0.
                    && obj
                        .world_to_local(point, frame)
                        .iter()
                        .all(|v| (0.0..=1.0).contains(v))
                    && obj.stencil_contains(point, frame)
                    && (obj.fill_visible_at(point, frame) || obj.stroke_visible_at(point, frame))
                    || obj.contains(point, frame))
        })
        .and_then(|obj| Some((obj, obj.text.as_ref()?)))
    }

    fn hit_test_with(&self, hit: impl Fn(&VectorObject) -> bool) -> Option<&VectorObject> {
        self.layers
            .iter()
            .rev()
            .filter(|l| l.visible && !l.locked)
            .find_map(|layer| {
                self.objects
                    .iter()
                    .rev()
                    .find(|obj| obj.layer == layer.id && !obj.locked && hit(obj))
            })
    }

    /// Check native project data before opening, saving or allocating render buffers.
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != FORMAT_VERSION {
            let version = self.format_version;
            return Err(format!("Unsupported document version {version}."));
        }
        let valid_sizes = 1..=MAX_DIMENSION;
        if !valid_sizes.contains(&self.width) || !valid_sizes.contains(&self.height) {
            return Err(format!(
                "Artboard dimensions must be 1 to {MAX_DIMENSION} pixels."
            ));
        }
        if !(1..=MAX_FRAMES).contains(&self.frames) || !(1..=MAX_FPS).contains(&self.fps) {
            return Err(format!(
                "Use 1 to {MAX_FRAMES} frames and 1 to {MAX_FPS} fps."
            ));
        }
        if self.next_id == 0 {
            return Err("Invalid next object ID.".into());
        }
        let layer_ids: HashSet<_> = self.layers.iter().map(|l| l.id).collect();
        if self.layers.is_empty() || layer_ids.contains(&0) || layer_ids.len() != self.layers.len()
        {
            return Err("A project needs at least one layer and unique, nonzero layer IDs.".into());
        }
        if self
            .guides_x
            .iter()
            .chain(&self.guides_y)
            .any(|value| !value.is_finite())
        {
            return Err("Guide positions must be finite.".into());
        }
        if self.uses_rig() && !self.rig_is_current()? {
            return Err("Refresh the artwork attachments after changing the skeleton.".into());
        }
        let mut ids = HashSet::new();
        let mut clip_sources = HashSet::new();
        let mut group_names = HashMap::new();
        for object in &self.objects {
            if object.name.chars().count() > MAX_OBJECT_NAME_CHARS
                || object.name.chars().any(char::is_control)
            {
                return Err("Object names can contain up to 80 printable characters.".into());
            }
            if object
                .symbol
                .as_ref()
                .is_some_and(|s| s.artwork().is_none())
            {
                return Err("A symbol instance is missing its artwork.".into());
            }
            if object
                .clip_instance
                .as_ref()
                .is_some_and(|i| i.shared_clip().is_none())
                || object.conflicting_placements()
            {
                return Err("Invalid nested clip placement.".into());
            }
            if let Some(instance) = &object.clip_instance {
                instance.validate_timing(self.frames)?;
            }
            if object.id == 0 || object.id == self.next_id || !ids.insert(object.id) {
                return Err(format!("Invalid or duplicate object ID {}.", object.id));
            }
            if !layer_ids.contains(&object.layer) || object.group == Some(0) {
                return Err(format!(
                    "Object {} has an invalid layer or group.",
                    object.id
                ));
            }
            if let Some(name) = object.group_name.as_deref() {
                let Some(group) = object.group else {
                    return Err(format!("Object {} has an invalid group name.", object.id));
                };
                if name.is_empty()
                    || name != name.trim()
                    || name.chars().count() > 80
                    || name.chars().any(char::is_control)
                {
                    return Err(format!("Object {} has an invalid group name.", object.id));
                }
                if group_names
                    .insert((object.layer, group), name)
                    .is_some_and(|previous| previous != name)
                {
                    return Err("Objects in the same group must share its name.".into());
                }
            }
            if let Some(binding) = &object.clip_source {
                binding.validate(object.stencil.is_some(), &mut clip_sources)?;
            }
            object.validate_art_data(self.frames)?;
        }
        // Documents with unnamed groups need no group-name allocations or
        // hash lookups. When labels exist, check for an unlabeled member too.
        if !group_names.is_empty()
            && self.objects.iter().any(|object| {
                object.group_name.is_none()
                    && object
                        .group
                        .is_some_and(|group| group_names.contains_key(&(object.layer, group)))
            })
        {
            return Err("Objects in the same group must share its name.".into());
        }
        Ok(())
    }
}

impl VectorObject {
    fn validate_art_data(&self, frames: u32) -> Result<(), String> {
        if self.reference_only && self.image.is_none() {
            return Err(format!(
                "Object {} has no embedded reference image.",
                self.id
            ));
        }
        if self.image_resource.is_some()
            || (self.image.is_some()
                && (self.is_instance()
                    || self.text.is_some()
                    || !matches!(self.shape, Shape::Rectangle)
                    || self.stroke_width != 0.
                    || self.gradient.is_some()
                    || self.stroke_gradient.is_some()
                    || !self.effects.is_empty()
                    || self.skin.is_some()))
        {
            return Err(format!("Object {} has an invalid embedded image.", self.id));
        }
        let positive_size = self.size.iter().all(|&v| v.is_finite() && v > 0.0);
        if !positive_size
            || !self.valid_bounds(self.position)
            || self.pivot.is_some_and(|p| {
                p.iter()
                    .zip(self.size)
                    .any(|(coordinate, size)| !(coordinate * size).is_finite())
            })
            || !self.rotation.is_finite()
            || !self.stroke_width.is_finite()
            || self.stroke_width < 0.0
        {
            return Err(format!("Object {} has invalid geometry.", self.id));
        }
        if !self.shape.valid_nodes(self.size) {
            return Err(format!("Object {} has invalid path nodes.", self.id));
        }
        if !pressure::valid_profile(&self.stroke_profile) {
            return Err(format!(
                "Object {} has an invalid stroke pressure profile.",
                self.id
            ));
        }
        for (&frame, &position) in &self.keys {
            if frame >= frames || !self.valid_bounds(position) {
                let id = self.id;
                return Err(format!("Object {id} has an invalid key at frame {frame}."));
            }
        }
        if let Some(style) = &self.text {
            style.validate()?;
            if !matches!(self.shape, Shape::Compound { .. }) {
                return Err("Text requires glyph outlines.".into());
            }
        }
        self.validate_tracks(frames)?;
        if let Some(motion) = &self.motion {
            motion.validate()?;
        }
        self.validate_effects()?;
        self.validate_appearance()?;
        Ok(())
    }
}

fn base_layer_id() -> u64 {
    1
}

fn default_layers() -> Vec<Layer> {
    vec![Layer::default()]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
    pub blend_mode: BlendMode,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            id: base_layer_id(),
            name: "Layer 1".into(),
            visible: true,
            locked: false,
            blend_mode: BlendMode::Normal,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Shape {
    #[default]
    Rectangle,
    Ellipse,
    Path {
        nodes: Vec<PathNode>,
        closed: bool,
    },
    /// Closed cubic contours with nonzero winding. Opposite windings cut holes.
    Compound {
        contours: Vec<Vec<PathNode>>,
    },
}

impl Shape {
    /// The editable contours, in paint order. Primitives have no authored nodes.
    pub fn contours(&self) -> &[Vec<PathNode>] {
        match self {
            Self::Path { nodes, .. } => std::slice::from_ref(nodes),
            Self::Compound { contours } => contours,
            _ => &[],
        }
    }

    pub fn contours_mut(&mut self) -> &mut [Vec<PathNode>] {
        match self {
            Self::Path { nodes, .. } => std::slice::from_mut(nodes),
            Self::Compound { contours } => contours,
            _ => &mut [],
        }
    }

    fn valid_nodes(&self, size: [f32; 2]) -> bool {
        let contours = self.contours();
        if matches!(self, Self::Compound { .. }) && contours.is_empty() {
            return false;
        }
        contours.iter().map(Vec::len).sum::<usize>() <= MAX_PATH_NODES
            && contours.iter().all(|nodes| {
                nodes.len() >= 2
                    && nodes.iter().all(|n| {
                        [n.point, n.incoming, n.outgoing].iter().all(|p| {
                            p.iter()
                                .zip(size)
                                .all(|(coord, scale)| (coord * scale).is_finite())
                        })
                    })
            })
    }
}

/// Anchor and absolute cubic control points in the object's normalized local coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathNode {
    pub point: [f32; 2],
    pub incoming: [f32; 2],
    pub outgoing: [f32; 2],
    #[serde(default, skip_serializing_if = "path_edit::HandleStyle::is_corner")]
    pub handle_style: path_edit::HandleStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

impl FillRule {
    pub fn is_nonzero(&self) -> bool {
        *self == Self::NonZero
    }
}

impl PathNode {
    pub fn corner(point: [f32; 2]) -> Self {
        Self {
            point,
            incoming: point,
            outgoing: point,
            handle_style: path_edit::HandleStyle::Corner,
        }
    }
}

fn black() -> [u8; 4] {
    [0, 0, 0, 255]
}

fn unit_scale() -> [f32; 2] {
    [1.; 2]
}
fn opaque() -> f32 {
    1.
}
fn is_unit_scale(value: &[f32; 2]) -> bool {
    *value == unit_scale()
}
fn is_opaque(value: &f32) -> bool {
    *value == opaque()
}
fn is_linear(value: &Interpolation) -> bool {
    *value == Interpolation::Linear
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineCap {
    Butt,
    #[default]
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

// Native drawings that omitted the join used Round before Miter became the drawing default.
fn legacy_line_join() -> LineJoin {
    LineJoin::Round
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorObject {
    pub id: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<SymbolInstance>,
    /// Independent static artwork owned by this placement. Copies share unchanged storage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedded_artwork: Option<EmbeddedArtwork>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_instance: Option<ClipInstance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigid_bone: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<Box<SkinBinding>>,
    #[serde(default, skip_serializing_if = "rig::RigidPose::is_identity")]
    pub(crate) rig_offset: rig::RigidPose,
    #[serde(skip)]
    pub(crate) rig_pose: rig::PoseTrack,
    /// Authored top-left corner of the untransformed base size box.
    pub position: [f32; 2],
    pub size: [f32; 2],
    /// Rotation, scale and skew pivot in normalized local coordinates. Defaults to the box center.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<[f32; 2]>,
    /// Unpremultiplied red, green, blue, alpha.
    pub fill: [u8; 4],
    #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
    pub fill_blend_mode: BlendMode,
    #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
    pub stroke_blend_mode: BlendMode,
    /// Disable vector fill paint, retaining authored colors and animation keys.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_fill: bool,
    /// Disable stroke paint, retaining its width, colors and animation keys.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_stroke: bool,
    #[serde(default, skip_serializing_if = "FillRule::is_nonzero")]
    pub fill_rule: FillRule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<Gradient>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_gradient: Option<Gradient>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stencil: Option<Stencil>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_source: Option<ClipBinding>,
    /// Authored absolute positions by zero-based frame index.
    pub keys: BTreeMap<u32, [f32; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rotation_keys: BTreeMap<u32, f32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scale_keys: BTreeMap<u32, [f32; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub opacity_keys: BTreeMap<u32, f32>,
    #[serde(default = "unit_scale", skip_serializing_if = "is_unit_scale")]
    pub scale: [f32; 2],
    #[serde(default = "opaque", skip_serializing_if = "is_opaque")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
    pub blend_mode: BlendMode,
    #[serde(default, skip_serializing_if = "is_linear")]
    pub interpolation: Interpolation,
    #[serde(default, skip_serializing_if = "AnimatedProperties::is_default")]
    pub animated: AnimatedProperties,
    #[serde(default)]
    pub shape: Shape,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<typography::TextStyle>,
    /// Embedded image pixels are shared across placements and undo snapshots.
    #[serde(skip)]
    pub image: Option<Arc<RasterImage>>,
    /// Display an embedded image while editing, but omit it from exported artwork.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reference_only: bool,
    /// Native archive reference, resolved before the drawing enters the editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_resource: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<MotionPath>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<PathEffect>,
    #[serde(default = "black")]
    pub stroke: [u8; 4],
    /// Width in artboard pixels. Zero disables the stroke.
    #[serde(default)]
    pub stroke_width: f32,
    /// Normalized arc length and width relative to the evaluated stroke width.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stroke_profile: Vec<[f32; 2]>,
    #[serde(default)]
    pub cap: LineCap,
    #[serde(default = "legacy_line_join")]
    pub join: LineJoin,
    /// Clockwise degrees around the object's transform pivot.
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "base_layer_id")]
    pub layer: u64,
    #[serde(default)]
    pub group: Option<u64>,
    /// Prevent editing this artwork through canvas and selection controls.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// Shared by members of the same group on this layer, independent of each object's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_name: Option<Arc<str>>,
}

pub type Rectangle = VectorObject;

impl VectorObject {
    pub fn uses_blending(&self) -> bool {
        !self.blend_mode.is_normal()
            || !self.fill_blend_mode.is_normal()
            || !self.stroke_blend_mode.is_normal()
    }

    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() {
            format!("{} {}", self.kind_name(), self.id)
        } else {
            self.name.clone()
        }
    }

    pub fn kind_name(&self) -> &'static str {
        if self.image.is_some() {
            return "Image";
        }
        if self.clip_instance.is_some() {
            return "Nested clip";
        }
        if self.symbol.is_some() {
            return "Symbol";
        }
        if self.embedded_artwork.is_some() {
            return "Independent artwork";
        }
        if self.text.is_some() {
            return "Text";
        }
        match self.shape {
            Shape::Rectangle => "Rectangle",
            Shape::Ellipse => "Ellipse",
            Shape::Path { .. } => "Path",
            Shape::Compound { .. } => "Compound path",
        }
    }

    fn valid_bounds(&self, position: [f32; 2]) -> bool {
        Self::valid_box(position, self.size)
    }
    fn valid_box(position: [f32; 2], size: [f32; 2]) -> bool {
        position
            .iter()
            .zip(size)
            .all(|(&v, extent)| v.is_finite() && (v + extent).is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_frames_interpolate_without_changing_authored_values() {
        let mut doc = Document::default();
        doc.add_rectangle([11., 22.], [4., 4.], [255; 4]);
        let rect = &mut doc.objects[0];
        assert_eq!(rect.position_at(4), [11., 22.]);
        rect.keys.insert(0, [2., 10.]);
        rect.keys.insert(4, [10., 2.]);
        let original = rect.clone();
        let poses: Vec<_> = (0..5).map(|frame| rect.position_at(frame)).collect();
        assert_eq!(poses, [[2., 10.], [4., 8.], [6., 6.], [8., 4.], [10., 2.]]);
        assert_eq!(*rect, original);
    }

    #[test]
    fn keys_hold_on_both_sides_of_track() {
        let mut doc = Document::default();
        doc.add_rectangle([0., 0.], [5., 5.], [255; 4]);
        let rect = &mut doc.objects[0];
        rect.keys.insert(2, [7., 7.]);
        assert_eq!(rect.position_at(0), [7., 7.]);
        rect.keys.insert(3, [9., 9.]);
        assert_eq!(rect.position_at(4), [9., 9.]);
    }

    #[test]
    fn selection_uses_evaluated_bounds_and_paint_order() {
        let mut doc = Document::default();
        let back = doc.add_rectangle([0., 0.], [10., 10.], [255; 4]);
        let front = doc.add_rectangle([1., 1.], [5., 5.], [255; 4]);
        doc.objects[1].keys.extend([(0, [1., 1.]), (4, [9., 1.])]);
        assert_eq!(doc.hit_test([2., 2.], 0), Some(front));
        assert_eq!(doc.hit_test([2., 2.], 2), Some(back));
        assert_eq!(doc.hit_test([5., 2.], 2), Some(front));
        assert_eq!(doc.hit_test([10., 2.], 2), None);
    }

    #[test]
    fn validation_rejects_invalid_keys_and_identifiers() {
        let mut doc = Document::default();
        let id = doc.add_rectangle([0., 0.], [4., 4.], [255; 4]);
        let rect = &mut doc.objects[0];
        rect.keys.insert(5, [0., 0.]);
        assert!(doc.validate().is_err());
        doc.objects[0].keys.clear();
        assert!(doc.validate().is_ok());
        doc.objects.push(doc.objects[0].clone());
        assert!(doc.validate().is_err());
        doc.objects.clear();
        assert_ne!(doc.add_rectangle([3., 4.], [8., 8.], [255; 4]), id);
        doc.objects[0].position[0] = f32::NAN;
        assert!(doc.validate().is_err());
    }
}

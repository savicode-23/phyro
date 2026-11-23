//! Correspondence belongs to the interval between two authored geometry keys.
use crate::morph::{MorphGuide, MorphPlan, MorphPoint};
use crate::properties::morphable;
use crate::{Property, Shape, VectorObject, MAX_FRAMES};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidedGeometry {
    spans: BTreeMap<u32, Span>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Span {
    end: u32,
    guides: Vec<MorphGuide>,
    /// Preserve the original cubic subdivision after a timeline splice. Artists edit only guides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    knots: Vec<MorphGuide>,
    #[serde(skip)]
    cache: Arc<Mutex<Option<CachedPlan>>>,
}

#[derive(Debug)]
struct CachedPlan {
    from: Shape,
    to: Shape,
    plan: Arc<MorphPlan>,
}

impl PartialEq for Span {
    fn eq(&self, other: &Self) -> bool {
        self.end == other.end && self.guides == other.guides && self.knots == other.knots
    }
}

impl Span {
    fn new(end: u32, guides: Vec<MorphGuide>) -> Self {
        Self {
            end,
            guides,
            knots: Vec::new(),
            cache: Default::default(),
        }
    }

    fn plan(&self, from: &Shape, to: &Shape) -> Result<Arc<MorphPlan>, String> {
        // Geometry keys are also accessible to transforms. Compare the actual poses so
        // shared drawing snapshots can never return a plan for a previously edited curve.
        let mut cached = self
            .cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(entry) = &*cached {
            if entry.from == *from && entry.to == *to {
                return Ok(entry.plan.clone());
            }
        }
        let constraints = if self.knots.is_empty() {
            &self.guides
        } else {
            &self.knots
        };
        if !self.knots.is_empty() {
            self.guide_indices()?;
        }
        let plan = Arc::new(MorphPlan::build(from, to, constraints)?);
        *cached = Some(CachedPlan {
            from: from.clone(),
            to: to.clone(),
            plan: plan.clone(),
        });
        Ok(plan)
    }

    fn guide_indices(&self) -> Result<Vec<usize>, String> {
        if self.knots.is_empty() {
            return Ok((0..self.guides.len()).collect());
        }
        let signature = |g: &MorphGuide| {
            (
                g.from.segment,
                g.from.t.to_bits(),
                g.to.segment,
                g.to.t.to_bits(),
            )
        };
        let indices: BTreeMap<_, _> = self
            .knots
            .iter()
            .enumerate()
            .map(|(i, g)| (signature(g), i))
            .collect();
        let mut seen = BTreeSet::new();
        self.guides
            .iter()
            .map(|g| {
                let index = *indices
                    .get(&signature(g))
                    .ok_or("A matched point is missing from its morph subdivision.")?;
                if seen.insert(index) {
                    Ok(index)
                } else {
                    Err("Each morph landmark can have only one partner.".into())
                }
            })
            .collect()
    }

    /// Keep the same correspondence across a splice without baking frames. An interior
    /// pose uses the common subdivision and both authored endpoints keep their own nodes.
    fn portion(
        &self,
        plan: &MorphPlan,
        closed: bool,
        left: Pose,
        right: Pose,
        end: u32,
    ) -> Result<Self, String> {
        if left == Pose::From && right == Pose::To {
            return Ok(Self {
                end,
                ..self.clone()
            });
        }
        let count = plan.knots().len();
        let common = |i: usize| {
            if !closed && i == count - 1 {
                MorphPoint {
                    segment: i - 1,
                    t: 1.,
                }
            } else {
                MorphPoint { segment: i, t: 0. }
            }
        };
        let make = |i, from, to| {
            let point = |pose| match pose {
                Pose::From => from,
                Pose::Middle => common(i),
                Pose::To => to,
            };
            MorphGuide {
                from: point(left),
                to: point(right),
            }
        };
        let mut result = Self::new(end, Vec::new());
        result.knots = plan
            .knots()
            .iter()
            .enumerate()
            .map(|(i, &(a, b))| make(i, a, b))
            .collect();
        let anchors = plan.sample_guides();
        for (guide, index) in self.guides.iter().zip(self.guide_indices()?) {
            let p = anchors[index];
            let i = p.segment + usize::from(p.t == 1.);
            let pair = make(i, guide.from, guide.to);
            result.knots[i] = pair;
            result.guides.push(pair);
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pose {
    From,
    Middle,
    To,
}

fn same_outline(a: &Shape, b: &Shape) -> bool {
    matches!((a, b), (Shape::Path { closed: x, .. }, Shape::Path { closed: y, .. }) if x == y)
}

impl GuidedGeometry {
    fn span(&self, start: u32, end: u32) -> Option<&Span> {
        self.spans.get(&start).filter(|span| span.end == end)
    }

    fn repair(&mut self, keys: &BTreeMap<u32, Shape>) {
        self.spans.retain(|start, s| {
            keys.contains_key(start)
                && keys
                    .range(start + 1..)
                    .next()
                    .is_some_and(|(&t, _)| t == s.end)
        });
        for ((&a, x), (&b, y)) in keys.iter().zip(keys.iter().skip(1)) {
            if !morphable(x, y) {
                self.spans
                    .entry(a)
                    .or_insert_with(|| Span::new(b, Vec::new()));
            }
        }
    }
}

impl VectorObject {
    /// Write a Geometry pose, splitting a guided interval if the pose is a new in-between.
    pub(crate) fn write_geometry_key(&mut self, tick: u32, shape: Shape) -> Result<(), String> {
        if tick >= MAX_FRAMES {
            return Err("Choose a frame inside the animation.".into());
        }
        let Some(track) = &self.animated.guided_geometry else {
            self.animated.geometry_keys.insert(tick, shape);
            return Ok(());
        };
        let previous_tick = self
            .animated
            .geometry_keys
            .range(..tick)
            .next_back()
            .map(|(&a, _)| a);
        let previous_easing = previous_tick.map(|a| Property::Geometry.easing_at(self, a));
        let mut split_easing = None;
        let keys = &mut self.animated.geometry_keys;
        if keys.get(&tick) == Some(&shape) {
            return Ok(());
        }
        let mut next = track.clone();
        if let Some(previous) = keys.get(&tick) {
            if !morphable(previous, &shape) {
                for (_, span) in next
                    .spans
                    .iter_mut()
                    .filter(|(a, s)| **a == tick || s.end == tick)
                {
                    if !span.guides.is_empty() {
                        return Err("Clear the matched points on this pose before changing its node layout.".into());
                    }
                    *span = Span::new(span.end, Vec::new());
                }
            }
        } else if let Some((&a, span)) = track
            .spans
            .range(..tick)
            .next_back()
            .filter(|(_, s)| tick < s.end)
        {
            let b = span.end;
            let old_plan = span.plan(&keys[&a], &keys[&b])?;
            let closed = matches!(shape, Shape::Path { closed: true, .. });
            let easing = previous_easing.unwrap_or_default();
            let progress = f64::from(tick - a) / f64::from(b - a);
            split_easing = Some((
                a,
                easing.portion(0., progress),
                easing.portion(progress, 1.),
            ));
            let held = easing == crate::Easing::Hold;
            let middle = if held { Pose::From } else { Pose::Middle };
            if (held && morphable(&keys[&a], &shape))
                || (!held && shape.contours()[0].len() == old_plan.knots().len())
            {
                next.spans.insert(
                    a,
                    span.portion(&old_plan, closed, Pose::From, middle, tick)?,
                );
                next.spans
                    .insert(tick, span.portion(&old_plan, closed, middle, Pose::To, b)?);
            } else {
                if !span.guides.is_empty() {
                    return Err(
                        "Insert a Geometry key before changing its matched node layout.".into(),
                    );
                }
                next.spans.insert(a, Span::new(tick, Vec::new()));
                next.spans.insert(tick, Span::new(b, Vec::new()));
            }
        }
        let old = keys.insert(tick, shape);
        next.repair(keys);
        let checked = next
            .spans
            .iter()
            .filter(|(a, span)| **a == tick || span.end == tick)
            .try_for_each(|(a, span)| span.plan(&keys[a], &keys[&span.end]).map(drop));
        if let Err(error) = checked {
            if let Some(old) = old {
                keys.insert(tick, old);
            } else {
                keys.remove(&tick);
            }
            return Err(error);
        }
        self.animated.guided_geometry = Some(next);
        if let Some((a, left, right)) = split_easing {
            Property::Geometry.set_easing(self, a, left);
            Property::Geometry.set_easing(self, tick, right);
        }
        Ok(())
    }

    pub(crate) fn remove_geometry_key(&mut self, tick: u32) -> bool {
        let keys = &mut self.animated.geometry_keys;
        let Some(removed) = keys.remove(&tick) else {
            return false;
        };
        let Some(track) = &self.animated.guided_geometry else {
            return true;
        };
        let mut next = track.clone();
        // When deleting an in-between, join pairs that meet on exactly the same landmark.
        if let (Some((&a, _)), Some((&b, _))) =
            (keys.range(..tick).next_back(), keys.range(tick..).next())
        {
            let left = track.span(a, tick);
            let right = track.span(tick, b);
            if left.is_some() || right.is_some() {
                let mut guides = Vec::new();
                if let (Some(left), Some(right)) = (left, right) {
                    let by_point: BTreeMap<_, _> = right
                        .guides
                        .iter()
                        .map(|r| ((r.from.segment, r.from.t.to_bits()), r.to))
                        .collect();
                    for l in &left.guides {
                        if let Some(&to) = by_point.get(&(l.to.segment, l.to.t.to_bits())) {
                            guides.push(MorphGuide { from: l.from, to });
                        }
                    }
                }
                let merged = Span::new(b, guides);
                if merged.plan(&keys[&a], &keys[&b]).is_err() {
                    keys.insert(tick, removed);
                    return false;
                }
                next.spans.insert(a, merged);
            }
        }
        next.repair(keys);
        self.animated.guided_geometry = (!next.spans.is_empty()
            || keys.values().any(|s| !morphable(&self.shape, s)))
        .then_some(next);
        true
    }

    /// Retime a selection atomically so endpoints can move past each other's old positions.
    pub fn move_guided_keys(&mut self, moved: &BTreeMap<u32, u32>) -> Result<(), String> {
        let track = self
            .animated
            .guided_geometry
            .as_ref()
            .ok_or("Choose a guided Geometry track.")?;
        let keys = &self.animated.geometry_keys;
        let mut destinations = BTreeSet::new();
        for (&a, &b) in moved {
            if b >= MAX_FRAMES
                || !keys.contains_key(&a)
                || !destinations.insert(b)
                || (keys.contains_key(&b) && !moved.contains_key(&b))
            {
                return Err("Geometry keys must move to distinct available frames.".into());
            }
        }
        let dest = |t: u32| moved.get(&t).copied().unwrap_or(t);
        let new_keys = keys
            .iter()
            .map(|(&t, s)| (dest(t), s.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut next = GuidedGeometry::default();
        for (&a, span) in &track.spans {
            let (mut lo, mut hi) = (dest(a), dest(span.end));
            let reversed = hi < lo;
            if reversed {
                std::mem::swap(&mut lo, &mut hi);
            }
            if new_keys.range(lo + 1..hi).next().is_none() {
                let relocated = if reversed {
                    let reverse = |g: &MorphGuide| MorphGuide {
                        from: g.to,
                        to: g.from,
                    };
                    let mut result = Span::new(hi, span.guides.iter().map(reverse).collect());
                    result.knots = span.knots.iter().map(reverse).collect();
                    result
                } else {
                    Span {
                        end: hi,
                        ..span.clone()
                    }
                };
                next.spans.insert(lo, relocated);
            }
        }
        next.repair(&new_keys);
        for (&a, span) in &next.spans {
            span.plan(&new_keys[&a], &new_keys[&span.end])?;
        }
        self.animated.geometry_keys = new_keys;
        self.animated.guided_geometry = Some(next);
        if let Some(easing) = self.animated.easing.get_mut(&Property::Geometry) {
            *easing = easing.iter().map(|(&t, &e)| (dest(t), e)).collect();
        }
        Ok(())
    }

    pub(crate) fn retime_guided_geometry(
        &mut self,
        source: &Self,
        moved: &BTreeMap<u32, u32>,
    ) -> Result<(), String> {
        let original = source
            .animated
            .guided_geometry
            .as_ref()
            .expect("retiming a guided track");
        let keys = moved
            .iter()
            .map(|(&dest, &src)| (dest, source.shape_at(src).into_owned()))
            .collect();
        let mut next = GuidedGeometry::default();
        for ((&dest, &src), (&next_dest, &next_src)) in moved.iter().zip(moved.iter().skip(1)) {
            if next_src <= src {
                continue;
            }
            let Some((&a, span)) = original.spans.range(..=src).next_back() else {
                continue;
            };
            if next_src > span.end {
                continue;
            }
            let x = &source.animated.geometry_keys[&a];
            let y = &source.animated.geometry_keys[&span.end];
            let plan = span.plan(x, y)?;
            let held = Property::Geometry.easing_at(source, a) == crate::Easing::Hold;
            let pose = |t| {
                if t == span.end {
                    Pose::To
                } else if t == a || held {
                    Pose::From
                } else {
                    Pose::Middle
                }
            };
            let partial = span.portion(
                &plan,
                matches!(x, Shape::Path { closed: true, .. }),
                pose(src),
                pose(next_src),
                next_dest,
            )?;
            next.spans.insert(dest, partial);
        }
        next.repair(&keys);
        self.animated.geometry_keys = keys;
        self.animated.guided_geometry = (!next.spans.is_empty()
            || self
                .animated
                .geometry_keys
                .values()
                .any(|s| !morphable(&self.shape, s)))
        .then_some(next);
        self.animated.easing.remove(&Property::Geometry);
        self.validate_guided_geometry()
    }

    pub fn can_guided_morph(&self) -> bool {
        matches!(self.shape, Shape::Path { .. })
            && self.skin.is_none()
            && self.stroke_profile.is_empty()
            && self.text.is_none()
            && self.image.is_none()
            && !self.is_instance()
    }

    pub(crate) fn accepts_guided_pose(&self, shape: &Shape) -> bool {
        self.animated.guided_geometry.is_some()
            && self.can_guided_morph()
            && same_outline(&self.shape, shape)
    }

    /// A single path in object-local coordinates, at each end of a sparse Geometry interval.
    pub fn set_guided_morph(
        &mut self,
        start: u32,
        end: u32,
        source: Shape,
        target: Shape,
        guides: Vec<MorphGuide>,
    ) -> Result<(), String> {
        if !self.can_guided_morph() {
            return Err(
                "Choose a single editable path with a uniform-width stroke and no skin binding."
                    .into(),
            );
        }
        if start >= end || end >= MAX_FRAMES {
            return Err("Choose two different frames in increasing order.".into());
        }
        if !same_outline(&self.shape, &source)
            || !same_outline(&source, &target)
            || !source.valid_nodes(self.size)
            || !target.valid_nodes(self.size)
        {
            return Err(
                "Both poses must be valid single outlines, both open or both closed.".into(),
            );
        }
        if self
            .animated
            .geometry_keys
            .range(start + 1..end)
            .next()
            .is_some()
        {
            return Err(
                "Choose adjacent Geometry keys. There is another key between these poses.".into(),
            );
        }
        let span = Span::new(end, guides);
        span.plan(&source, &target)?;
        let mut staged = self.clone();
        if let Some(track) = &mut staged.animated.guided_geometry {
            if track.span(start, end).is_some() {
                track.spans.remove(&start);
            }
            for (t, pose) in [(start, source), (end, target)] {
                staged.write_geometry_key(t, pose)?;
            }
        } else {
            staged
                .animated
                .geometry_keys
                .extend([(start, source), (end, target)]);
            let mut track = GuidedGeometry::default();
            track.repair(&staged.animated.geometry_keys);
            staged.animated.guided_geometry = Some(track);
        }
        staged
            .animated
            .guided_geometry
            .get_or_insert_with(Default::default)
            .spans
            .insert(start, span);
        staged.validate_guided_geometry()?;
        self.animated = staged.animated;
        Ok(())
    }

    pub fn set_morph_guides(
        &mut self,
        start: u32,
        end: u32,
        guides: Vec<MorphGuide>,
    ) -> Result<(), String> {
        let track = self
            .animated
            .guided_geometry
            .as_mut()
            .ok_or("Choose a guided Geometry span.")?;
        match track.span(start, end) {
            None => return Err("Choose two endpoints of the same guided Geometry span.".into()),
            Some(span) if span.guides == guides => return Ok(()),
            _ => {}
        }
        let keys = &self.animated.geometry_keys;
        let span = Span::new(end, guides);
        span.plan(
            keys.get(&start).ok_or("Missing starting pose.")?,
            keys.get(&end).ok_or("Missing ending pose.")?,
        )?;
        track.spans.insert(start, span);
        Ok(())
    }

    pub fn morph_guides(&self, start: u32, end: u32) -> Option<&[MorphGuide]> {
        Some(
            &self
                .animated
                .guided_geometry
                .as_ref()?
                .span(start, end)?
                .guides,
        )
    }

    /// The same compiled correspondence used by playback, including spliced breakdown poses.
    pub fn guided_morph_plan(&self, start: u32, end: u32) -> Option<Arc<MorphPlan>> {
        let keys = &self.animated.geometry_keys;
        self.guided_plan(start, end, keys.get(&start)?, keys.get(&end)?)
    }

    pub fn guided_morph_span(&self, tick: u32) -> Option<(u32, u32)> {
        let spans = &self.animated.guided_geometry.as_ref()?.spans;
        if let Some((&start, span)) = spans.range(..tick).next_back() {
            if tick <= span.end {
                return Some((start, span.end));
            }
        }
        spans.get(&tick).map(|span| (tick, span.end))
    }

    pub(crate) fn guided_plan(
        &self,
        a: u32,
        b: u32,
        from: &Shape,
        to: &Shape,
    ) -> Option<Arc<MorphPlan>> {
        self.animated
            .guided_geometry
            .as_ref()?
            .span(a, b)?
            .plan(from, to)
            .ok()
    }

    pub(crate) fn validate_guided_geometry(&self) -> Result<(), String> {
        let Some(track) = &self.animated.guided_geometry else {
            return Ok(());
        };
        let keys = &self.animated.geometry_keys;
        if !self.can_guided_morph() || keys.is_empty() {
            return Err(
                "A guided morph needs Geometry keys on a single editable path with uniform width."
                    .into(),
            );
        }
        for (&a, span) in &track.spans {
            let Some(x) = keys.get(&a) else {
                return Err("Missing starting pose for a guided morph.".into());
            };
            let Some((&b, y)) = keys
                .range((std::ops::Bound::Excluded(a), std::ops::Bound::Unbounded))
                .next()
            else {
                return Err("Missing ending pose for a guided morph.".into());
            };
            if b != span.end {
                return Err("Guided morph poses must be adjacent Geometry keys.".into());
            }
            span.plan(x, y)?;
        }
        for ((&a, x), (&b, y)) in keys.iter().zip(keys.iter().skip(1)) {
            if !morphable(x, y) && track.span(a, b).is_none() {
                return Err("Different node counts require a guided Geometry span.".into());
            }
        }
        Ok(())
    }
}

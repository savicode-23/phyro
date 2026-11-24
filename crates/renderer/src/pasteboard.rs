//! Cached, signed pixel regions for the pasteboard and zoomed artboard. Object positions and
//! paint servers retain their authored coordinate system throughout rendering.
use super::{
    bounds::{pixel_bounds, symbol_bounds, union, SymbolBounds},
    clip_groups, Document, IntRect, Pixmap, Transform, VectorObject,
};
use std::collections::HashMap;

pub const TILE: u32 = 256;
/// A pixel of neighboring paint on each side supports linear texture sampling.
pub const TEXTURE: u32 = TILE + 2;
pub type Tile = [i64; 2];

fn tile_origin(key: Tile) -> [i64; 2] {
    key.map(|v| v.saturating_mul(i64::from(TILE)).saturating_sub(1))
}

fn full_tile() -> IntRect {
    IntRect::from_xywh(0, 0, TEXTURE, TEXTURE).unwrap()
}

fn on_tile(rect: IntRect, key: Tile) -> Option<IntRect> {
    let [x, y] = tile_origin(key);
    let local = |n: i32, origin| {
        i64::from(n)
            .saturating_sub(origin)
            .clamp(0, i64::from(TEXTURE)) as i32
    };
    IntRect::from_ltrb(
        local(rect.left(), x),
        local(rect.top(), y),
        local(rect.right(), x),
        local(rect.bottom(), y),
    )
}

#[derive(Default)]
pub struct Damage {
    all: bool,
    regions: Vec<IntRect>,
    viewport: Option<[i64; 4]>,
}
impl Damage {
    pub fn all() -> Self {
        Self {
            all: true,
            ..Self::default()
        }
    }
    fn for_view([first, last]: [Tile; 2]) -> Self {
        let [l, t] = tile_origin(first);
        let [x, y] = tile_origin(last);
        Self {
            viewport: Some([
                l,
                t,
                x.saturating_add(i64::from(TEXTURE)),
                y.saturating_add(i64::from(TEXTURE)),
            ]),
            ..Self::default()
        }
    }
    fn add(&mut self, bounds: Option<IntRect>) {
        if self.all {
            return;
        }
        if let (Some(b), Some([l, t, r, end])) = (bounds, self.viewport) {
            if i64::from(b.right()) <= l
                || i64::from(b.bottom()) <= t
                || i64::from(b.left()) >= r
                || i64::from(b.top()) >= end
            {
                return;
            }
        }
        if let Some(r) = bounds.filter(|_| self.regions.len() < 64) {
            self.regions.push(r)
        } else {
            self.all = true;
            self.regions.clear();
        }
    }
    pub fn affects(&self, key: Tile) -> bool {
        self.area(key).is_some()
    }
    fn area(&self, key: Tile) -> Option<IntRect> {
        if self.all {
            Some(full_tile())
        } else {
            self.regions
                .iter()
                .filter_map(|&r| on_tile(r, key))
                .reduce(union)
        }
    }
}

/// A paint-ordered list of off-page candidates. Slots refer to the currently
/// displayed document. The index owns no reference to a drag preview.
pub struct Scene {
    frame: u32,
    scale: [f32; 2],
    page: IntRect,
    on_page: bool,
    layer: HashMap<u64, usize>,
    objects: Vec<Vec<(usize, Option<IntRect>)>>,
    clip_runs: HashMap<usize, usize>,
}

impl Scene {
    pub fn new(doc: &Document, frame: u32, page_pixels: [u32; 2]) -> Option<Self> {
        Self::for_region(doc, frame, page_pixels, false)
    }

    /// Rasterize only visible tiles of a high-density artboard. The page size
    /// defines the coordinate grid and never determines a surface allocation.
    pub fn for_artboard(doc: &Document, frame: u32, page_pixels: [u32; 2]) -> Option<Self> {
        Self::for_region(doc, frame, page_pixels, true)
    }

    fn for_region(
        doc: &Document,
        frame: u32,
        page_pixels: [u32; 2],
        on_page: bool,
    ) -> Option<Self> {
        let page = IntRect::from_xywh(0, 0, page_pixels[0], page_pixels[1])?;
        (doc.width > 0 && doc.height > 0 && frame < doc.frames)
            .then(|| Self::build(doc, frame, page, on_page))
    }

    fn build(doc: &Document, frame: u32, page: IntRect, on_page: bool) -> Self {
        let layer: HashMap<_, _> = doc
            .layers
            .iter()
            .filter(|l| l.visible)
            .enumerate()
            .map(|(i, l)| (l.id, i))
            .collect();
        let mut scene = Self {
            frame,
            scale: [
                page.width() as f32 / doc.width as f32,
                page.height() as f32 / doc.height as f32,
            ],
            page,
            on_page,
            objects: vec![vec![]; layer.len()],
            clip_runs: clip_groups::shared_slots(doc, frame),
            layer,
        };
        let mut symbols = SymbolBounds::new();
        for (slot, o) in doc.objects.iter().enumerate() {
            if let Some((l, b)) = scene.candidate(o, &mut symbols) {
                scene.objects[l].push((slot, b));
            }
        }
        scene
    }
    pub fn scale(&self) -> [f32; 2] {
        self.scale
    }
    pub fn page_pixels(&self) -> [u32; 2] {
        [self.page.width(), self.page.height()]
    }
    pub fn frame(&self) -> u32 {
        self.frame
    }
    pub fn is_empty(&self) -> bool {
        self.objects.iter().all(Vec::is_empty)
    }
    /// Sample off-page ink from the same evaluated slots and pixel grid used by the visible tiles.
    pub fn sample(&self, doc: &Document, point: [f32; 2]) -> Result<[u8; 4], String> {
        let [x, y] = [0, 1].map(|i| (point[i] * self.scale[i]).floor());
        let objects = self
            .objects
            .iter()
            .flatten()
            .filter(|(_, bounds)| {
                bounds.is_none_or(|b| {
                    b.left() as f32 <= x
                        && x < b.right() as f32
                        && b.top() as f32 <= y
                        && y < b.bottom() as f32
                })
            })
            .map(|(slot, _)| {
                (
                    *self.clip_runs.get(slot).unwrap_or(slot),
                    &doc.objects[*slot],
                )
            });
        super::sample_objects(objects, &doc.layers, self.frame, point, self.scale)
    }
    fn candidate(
        &self,
        o: &VectorObject,
        symbols: &mut SymbolBounds,
    ) -> Option<(usize, Option<IntRect>)> {
        let &l = self.layer.get(&o.layer)?;
        if !o.visible_at(self.frame) || o.opacity_at(self.frame) <= 0. {
            return None;
        }
        let transform = Transform::from_scale(self.scale[0], self.scale[1]);
        // At extreme coordinates f32 transform rounding can exceed the padding.
        // Let the translated tile renderer decide those cases in its own frame.
        let bounds = if o.static_artwork().is_some() {
            symbol_bounds(symbols, o, self.frame, transform)
        } else {
            pixel_bounds(o, self.frame, transform)
        };
        let bound = bounds
            .filter(|b| b.left().min(b.top()) > -(1 << 22) && b.right().max(b.bottom()) < 1 << 22);
        let eligible = bound.is_none_or(|b| {
            if self.on_page {
                self.page.intersect(&b).is_some()
            } else {
                !self.page.contains(&b)
            }
        });
        eligible.then_some((l, bound))
    }
    /// Reuse every unchanged object's bounds. A layer or order change takes the
    /// fresh paint-order path. Appending artwork and cancelling it preserve slots.
    pub fn update(&mut self, old: &Document, new: &Document, visible: [Tile; 2]) -> Damage {
        if old.layers != new.layers
            || old.skeleton != new.skeleton
            || old.width != new.width
            || old.height != new.height
        {
            *self = Self::build(new, self.frame, self.page, self.on_page);
            return Damage::all();
        }
        let mut damage = Damage::for_view(visible);
        let mut membership: Vec<Vec<(usize, Option<Option<IntRect>>)>> =
            vec![Vec::new(); self.objects.len()];
        let mut symbols = SymbolBounds::new();
        let mut check_runs = old.objects.len() != new.objects.len() && !self.clip_runs.is_empty();
        for slot in 0..old.objects.len().max(new.objects.len()) {
            let (a, b) = (old.objects.get(slot), new.objects.get(slot));
            if a == b {
                continue;
            }
            if a.zip(b).is_some_and(|(a, b)| a.id != b.id) {
                *self = Self::build(new, self.frame, self.page, self.on_page);
                return Damage::all();
            }
            check_runs |= a
                .into_iter()
                .chain(b)
                .any(|o| o.group.is_some() && o.clip_source.is_some())
                || !self.clip_runs.is_empty() && a.zip(b).is_some_and(|(a, b)| a.layer != b.layer);
            let prior = a.and_then(|o| self.layer.get(&o.layer)).and_then(|&l| {
                self.objects[l]
                    .binary_search_by_key(&slot, |s| s.0)
                    .ok()
                    .map(|i| (l, i))
            });
            let next = b.and_then(|o| self.candidate(o, &mut symbols));
            if let Some((l, i)) = prior {
                damage.add(self.objects[l][i].1);
            }
            if let Some((_, extent)) = next {
                damage.add(extent);
            }
            match (prior, next) {
                (Some((l, i)), Some((n, extent))) if l == n => self.objects[l][i].1 = extent,
                (prior, next) => {
                    if let Some((l, _)) = prior {
                        membership[l].push((slot, None));
                    }
                    if let Some((l, extent)) = next {
                        membership[l].push((slot, Some(extent)));
                    }
                }
            }
        }
        for (layer, updates) in self.objects.iter_mut().zip(membership) {
            if updates.is_empty() {
                continue;
            }
            // Apply all insertions/removals in one sorted merge. Even a group
            // of thousands crossing the page edge moves each entry at most once.
            let mut merged = Vec::with_capacity(layer.len() + updates.len());
            let mut prior = layer.drain(..).peekable();
            for (slot, extent) in updates {
                while prior.peek().is_some_and(|entry| entry.0 < slot) {
                    merged.push(prior.next().unwrap());
                }
                if prior.peek().is_some_and(|entry| entry.0 == slot) {
                    prior.next();
                }
                if let Some(extent) = extent {
                    merged.push((slot, extent));
                }
            }
            merged.extend(prior);
            *layer = merged;
        }
        if check_runs {
            let runs = clip_groups::shared_slots(new, self.frame);
            if self.clip_runs != runs {
                self.clip_runs = runs;
                return Damage::all();
            }
        }
        damage
    }
    pub fn plan(&self, key: Tile, damage: &Damage) -> Option<TilePlan> {
        let area = damage.area(key)?;
        let slots = self
            .objects
            .iter()
            .flatten()
            .filter(|(_, bound)| match bound {
                None => true,
                Some(b) => on_tile(*b, key).is_some_and(|q| q.intersect(&area).is_some()),
            })
            .map(|(slot, _)| (*slot, *self.clip_runs.get(slot).unwrap_or(slot)))
            .collect();
        Some(TilePlan {
            paint: TilePaint {
                key,
                area,
                page: self.page,
                on_page: self.on_page,
                frame: self.frame,
                scale: self.scale,
            },
            slots,
        })
    }
}

pub struct TilePlan {
    paint: TilePaint,
    slots: Vec<(usize, usize)>,
}
impl TilePlan {
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    /// Small, local paints normally finish within the current pointer frame.
    pub fn is_small(&self, doc: &Document) -> bool {
        self.slots.len() <= 8
            && self.slots.iter().all(|&(slot, _)| {
                let o = &doc.objects[slot];
                !o.is_instance()
                    && o.stencil.is_none()
                    && o.shape.contours().iter().map(Vec::len).sum::<usize>() <= 128
            })
    }
    pub fn render(&self, doc: &Document) -> Result<Patch, String> {
        self.paint.render(
            self.slots.iter().map(|&(i, run)| (run, &doc.objects[i])),
            &doc.layers,
        )
    }
    pub fn render_until(
        &self,
        doc: &Document,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Patch, String> {
        self.paint.render(
            self.slots
                .iter()
                .take_while(|_| !cancel.load(std::sync::atomic::Ordering::Relaxed))
                .map(|&(i, run)| (run, &doc.objects[i])),
            &doc.layers,
        )
    }
    /// Cold paints own just these candidates. A worker cannot hold the editor's
    /// authored snapshot or prevent the next transform from reusing its allocation.
    pub fn detach(self, doc: &Document) -> TileJob {
        TileJob {
            paint: self.paint,
            layers: doc
                .layers
                .iter()
                .filter(|l| !l.blend_mode.is_normal())
                .cloned()
                .collect(),
            objects: self
                .slots
                .into_iter()
                .map(|(i, run)| (run, doc.objects[i].clone()))
                .collect(),
        }
    }
}

pub struct TileJob {
    paint: TilePaint,
    layers: Vec<document_core::Layer>,
    objects: Vec<(usize, VectorObject)>,
}
impl TileJob {
    pub fn render_until(&self, cancel: &std::sync::atomic::AtomicBool) -> Result<Patch, String> {
        self.paint.render(
            self.objects
                .iter()
                .take_while(|_| !cancel.load(std::sync::atomic::Ordering::Relaxed))
                .map(|(run, o)| (*run, o)),
            &self.layers,
        )
    }
}

struct TilePaint {
    key: Tile,
    area: IntRect,
    page: IntRect,
    on_page: bool,
    frame: u32,
    scale: [f32; 2],
}
pub struct Patch {
    pub origin: [usize; 2],
    pub size: [usize; 2],
    pub pixels: Option<Pixmap>,
}
impl TilePaint {
    fn render<'a>(
        &self,
        objects: impl Iterator<Item = (usize, &'a VectorObject)>,
        layers: &[document_core::Layer],
    ) -> Result<Patch, String> {
        let rect = self.area;
        let mut patch = Patch {
            origin: [rect.left() as usize, rect.top() as usize],
            size: [rect.width() as usize, rect.height() as usize],
            pixels: None,
        };
        let mut objects = objects.peekable();
        if objects.peek().is_none() {
            return Ok(patch);
        }
        // A curve clipped at the raster boundary can tessellate differently to
        // one that continues across it. Paint beyond the texture before cropping,
        // using the same surface for a full tile and its subsequent edit patches.
        const BORDER: u32 = TILE / 2;
        const SURFACE: u32 = TEXTURE + 2 * BORDER;
        let [x, y] = tile_origin(self.key).map(|v| v.saturating_sub(i64::from(BORDER)));
        let transform = Transform::from_scale(self.scale[0], self.scale[1])
            .post_translate(-(x as f32), -(y as f32));
        // Render in this tile's original coordinates even for a small edit. This
        // keeps masks, gradients and antialiasing identical to a freshly painted tile.
        let mut image =
            Pixmap::new(SURFACE, SURFACE).ok_or("Could not allocate a pasteboard tile.")?;
        clip_groups::paint_with_layers(
            &mut image,
            objects,
            layers,
            (self.frame, transform, [transform.tx, transform.ty]),
            None,
            crate::RenderPurpose::Editor,
        )?;
        // Keep the edge texels beside the page. Linear sampling may blend
        // them into the visible margin when the screen and raster grids differ.
        let page = (!self.on_page)
            .then(|| self.page.inset(1, 1).and_then(|p| on_tile(p, self.key)))
            .flatten();
        let visible = (rect.top()..rect.bottom()).any(|row| {
            let start = (row as usize + BORDER as usize) * SURFACE as usize + BORDER as usize;
            let ink = |l: i32, r: i32| {
                l < r
                    && image.pixels()[start + l as usize..start + r as usize]
                        .iter()
                        .any(|p| p.alpha() != 0)
            };
            if let Some(p) = page.filter(|p| p.top() <= row && row < p.bottom()) {
                ink(rect.left(), rect.right().min(p.left()))
                    || ink(rect.left().max(p.right()), rect.right())
            } else {
                ink(rect.left(), rect.right())
            }
        });
        if visible {
            let region = rect.translate(BORDER as i32, BORDER as i32).unwrap();
            patch.pixels = Some(
                image
                    .clone_rect(region)
                    .ok_or("Could not copy pasteboard pixels.")?,
            );
        }
        Ok(patch)
    }
}

#[cfg(test)]
mod tests;

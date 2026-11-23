use document_core::GradientStop;
use tiny_skia::{
    ColorU8, FilterQuality, Pattern, Pixmap, PremultipliedColorU8, Rect, Shader, SpreadMode,
    Transform,
};

/// Shade a box gradient at destination pixel centers. Its nearest-neighbor pattern is
/// used only in this paint; the regular vector path supplies antialiasing and fill rules.
#[allow(clippy::too_many_arguments)]
pub(super) fn shader<'a>(
    stops: &[GradientStop],
    center: [f32; 2],
    radius: f32,
    opacity: f32,
    space: Transform,
    fill_bounds: Rect,
    transform: Transform,
    size: [u32; 2],
    tile: &'a mut Option<Pixmap>,
) -> Option<Shader<'a>> {
    let pixels = space.post_concat(transform);
    let inverse = pixels.invert()?;
    let bounds = fill_bounds
        .transform(transform)?
        .intersect(&Rect::from_xywh(0., 0., size[0] as f32, size[1] as f32)?)?;
    let x = bounds.left().floor();
    let y = bounds.top().floor();
    let width = (bounds.right().ceil() - x) as u32;
    let height = (bounds.bottom().ceil() - y) as u32;
    let tile_to_world = Transform::from_translate(x, y).post_concat(transform.invert()?);
    if tile
        .as_ref()
        .is_none_or(|p| (p.width(), p.height()) != (width, height))
    {
        // Release differently sized fill scratch before shading a stroke on the same object.
        *tile = None;
        *tile = Pixmap::new(width, height);
    }
    let tile = tile.as_mut()?;
    let ramp = Ramp {
        stops,
        first: stops.first()?,
        last: stops.last()?,
        opacity,
    };
    // For an axis-aligned paint, every pixel takes its color from either its column
    // or its row. Sample each axis once even for multi-stop gradients.
    let columns = if inverse.kx == 0. && inverse.ky == 0. {
        (0..width)
            .map(|column| {
                let dx = (inverse.sx * (x + column as f32 + 0.5) + (inverse.tx - center[0])).abs();
                (dx, ramp.pixel(dx / radius))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let uniform = if columns.is_empty() {
        ramp.uniform_intervals(tile.pixels().len())
    } else {
        Vec::new()
    };
    for (row, values) in tile
        .pixels_mut()
        .chunks_exact_mut(width as usize)
        .enumerate()
    {
        let py = y + row as f32 + 0.5;
        let row_x = inverse.kx * py + inverse.tx - center[0];
        let row_y = inverse.sy * py + inverse.ty - center[1];
        if !columns.is_empty() {
            let dy = row_y.abs();
            let color = ramp.pixel(dy / radius);
            for (pixel, &(dx, col_color)) in values.iter_mut().zip(&columns) {
                *pixel = if dx > dy { col_color } else { color };
            }
            continue;
        }
        for (column, pixel) in values.iter_mut().enumerate() {
            let px = x + column as f32 + 0.5;
            let dx = (inverse.sx * px + row_x).abs();
            let dy = (inverse.ky * px + row_y).abs();
            let t = dx.max(dy) / radius;
            *pixel = uniform
                .get((t * uniform.len() as f32) as usize)
                .copied()
                .flatten()
                .unwrap_or_else(|| ramp.pixel(t));
        }
    }
    Some(Pattern::new(
        tile.as_ref(),
        SpreadMode::Pad,
        FilterQuality::Nearest,
        1.,
        tile_to_world,
    ))
}

struct Ramp<'a> {
    stops: &'a [GradientStop],
    first: &'a GradientStop,
    last: &'a GradientStop,
    opacity: f32,
}

impl Ramp<'_> {
    fn rgba(&self, t: f32) -> [u8; 4] {
        if t < self.first.offset {
            return self.first.color;
        }
        if !t.is_finite() || t >= self.last.offset {
            return self.last.color;
        }
        let right = if self.stops.len() == 2 {
            1
        } else {
            self.stops.partition_point(|stop| stop.offset <= t)
        };
        let (a, b) = (&self.stops[right - 1], &self.stops[right]);
        let ratio = (t - a.offset) / (b.offset - a.offset);
        std::array::from_fn(|i| {
            let start = f32::from(a.color[i]);
            (start + ratio * (f32::from(b.color[i]) - start) + 0.5) as u8
        })
    }

    fn premultiply(&self, [r, g, b, a]: [u8; 4]) -> PremultipliedColorU8 {
        ColorU8::from_rgba(r, g, b, (f32::from(a) * self.opacity + 0.5) as u8).premultiply()
    }

    fn pixel(&self, t: f32) -> PremultipliedColorU8 {
        self.premultiply(self.rgba(t))
    }

    /// Between adjacent stops each straight RGBA component is monotonic. If the
    /// rounded endpoints agree, the whole interval produces precisely that pixel.
    /// A power-of-two interval count keeps its boundaries exactly representable.
    fn uniform_intervals(&self, pixel_count: usize) -> Vec<Option<PremultipliedColorU8>> {
        let count = (pixel_count / 32).min(8192).next_power_of_two();
        let mut start = self.rgba(0.);
        let mut next_stop = 0;
        (0..count)
            .map(|i| {
                let end_t = (i + 1) as f32 / count as f32;
                let end = self.rgba(end_t);
                let previous_stop = next_stop;
                while next_stop < self.stops.len() && self.stops[next_stop].offset <= end_t {
                    next_stop += 1;
                }
                let uniform =
                    (start == end && previous_stop == next_stop).then(|| self.premultiply(start));
                start = end;
                uniform
            })
            .collect()
    }
}

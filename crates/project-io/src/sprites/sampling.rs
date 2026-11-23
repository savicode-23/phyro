use super::{render_tick, Clip, Pixmap};
use std::{io, ops::Range};

#[derive(Default)]
pub(super) struct ExposureClock {
    exposure: Range<u32>,
    static_drawing: bool,
}

impl ExposureClock {
    /// A single immutable clip owns the cache for the duration of each export.
    pub(super) fn sample_tick(&mut self, clip: &Clip, tick: u32) -> u32 {
        if clip.cels.len() == clip.frames as usize {
            return tick;
        }
        if !self.exposure.contains(&tick) {
            let (start, duration) = clip.exposure(tick);
            self.exposure = start..start + duration;
            self.static_drawing = duration > 1 && clip.drawing_at(start).frame_independent();
        }
        if self.static_drawing {
            self.exposure.start
        } else {
            tick
        }
    }
}

pub(super) struct Frame {
    tick: u32,
    pub(super) image: Pixmap,
    png: Option<Vec<u8>>,
}

impl Frame {
    pub(super) fn png(&mut self) -> io::Result<&[u8]> {
        if self.png.is_none() {
            self.png = Some(self.image.encode_png().map_err(io::Error::other)?);
        }
        Ok(self.png.as_deref().unwrap())
    }
}

#[derive(Default)]
pub(super) struct FrameCache(Option<Frame>);

impl FrameCache {
    pub(super) fn sample(&mut self, clip: &Clip, tick: u32, scale: f64) -> io::Result<&mut Frame> {
        if self.0.as_ref().is_none_or(|frame| frame.tick != tick) {
            // Release potentially large pixels and PNG bytes before allocating
            // their replacements. Only one source frame lives between ticks.
            self.0 = None;
            self.0 = Some(Frame {
                tick,
                image: render_tick(clip, tick, scale)?,
                png: None,
            });
        }
        Ok(self.0.as_mut().unwrap())
    }
}

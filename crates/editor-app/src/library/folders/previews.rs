use super::backend::{Backend, ClipInfo, Entry, PreviewKey};
use crate::library::{AssetDrop, AssetSource};
use eframe::egui;
use std::collections::HashMap;
use std::path::Path;

// Each 128×128 RGBA texture is at most 64 KiB. Keep room for the visible
// grid, the selected strip and a handful of recent playback frames.
const MAX_RESIDENT: usize = 64;
pub(super) const MAX_VISIBLE: usize = 56;

#[derive(Clone, Copy)]
pub(super) struct Image {
    pub id: egui::TextureId,
    pub size: egui::Vec2,
}

struct Cached {
    used: u64,
    texture: Result<egui::TextureHandle, String>,
    clips: Vec<ClipInfo>,
}

#[derive(Default)]
pub(super) struct Previews {
    items: HashMap<PreviewKey, Cached>,
    missing: Vec<PreviewKey>,
    priority_count: usize,
    clock: u64,
}

impl Previews {
    pub(super) fn start_drag(
        &self,
        response: &egui::Response,
        entry: &Entry,
        clip_id: Option<u64>,
    ) {
        if !response.drag_started() {
            return;
        }
        let file = PreviewKey::thumbnail(entry);
        let clips = self.clips(&file);
        let clip = clip_id.and_then(|id| clips.iter().find(|clip| clip.id == id));
        let label = match clip {
            Some(c) => format!("{} · {}", entry.name, c.name),
            None => entry.name.clone(),
        };
        let cached = |key: &PreviewKey| self.items.get(key)?.texture.as_ref().ok();
        // Retain an already uploaded texture for this gesture. A cold asset can
        // be placed immediately and a different clip never shows the file poster.
        let thumbnail = clip_id
            .and_then(|id| cached(&PreviewKey::frame(entry, id, 0)))
            .or_else(|| {
                if clip_id.is_none() || clips.first().map(|c| c.id) == clip_id {
                    cached(&file)
                } else {
                    None
                }
            })
            .cloned();
        response.dnd_set_drag_payload(AssetDrop {
            source: AssetSource::File {
                path: entry.path.clone(),
                clip_id,
            },
            label,
            thumbnail,
        });
    }

    pub(super) fn clear(&mut self) {
        self.items.clear();
        self.missing.clear();
    }

    pub(super) fn invalidate(&mut self, path: &Path) {
        self.items.retain(|key, _| key.path() != path);
    }

    pub(super) fn begin(&mut self) {
        self.clock = self.clock.wrapping_add(1);
        self.missing.clear();
        self.priority_count = 0;
    }

    pub(super) fn requests(&self) -> &[PreviewKey] {
        &self.missing
    }

    pub(super) fn receive(&mut self, ctx: &egui::Context, worker: &mut Backend) {
        // Draining a burst of completed files in one pass would put every upload
        // in the same editor frame. Leave the rest for the next idle repaint.
        for _ in 0..2 {
            let Some((key, result)) = worker.take_preview() else {
                return;
            };
            if self.items.len() >= MAX_RESIDENT {
                if let Some(oldest) = self
                    .items
                    .iter()
                    .min_by_key(|(_, item)| item.used)
                    .map(|(key, _)| key.clone())
                {
                    self.items.remove(&oldest);
                }
            }
            let (texture, clips) = match result {
                Ok(preview) => (
                    Ok(ctx.load_texture(
                        "asset preview",
                        preview.image,
                        egui::TextureOptions::LINEAR,
                    )),
                    preview.clips,
                ),
                Err(error) => (Err(error), Vec::new()),
            };
            self.items.insert(
                key,
                Cached {
                    used: self.clock,
                    texture,
                    clips,
                },
            );
        }
        ctx.request_repaint();
    }

    pub(super) fn texture(&mut self, key: PreviewKey, priority: bool) -> Option<Image> {
        if let Some(item) = self.items.get_mut(&key) {
            item.used = self.clock;
            return item.texture.as_ref().ok().map(|handle| Image {
                id: handle.id(),
                size: handle.size_vec2(),
            });
        }
        if priority {
            if !self.missing[..self.priority_count].contains(&key) {
                if let Some(i) = self.missing[self.priority_count..]
                    .iter()
                    .position(|k| k == &key)
                {
                    self.missing.remove(self.priority_count + i);
                }
                self.missing.insert(self.priority_count, key);
                self.priority_count += 1;
            }
        } else {
            // Each non-priority request comes from a distinct visible tile.
            self.missing.push(key);
        }
        None
    }

    pub(super) fn clips(&self, key: &PreviewKey) -> &[ClipInfo] {
        self.items
            .get(key)
            .map_or(&[], |item| item.clips.as_slice())
    }

    pub(super) fn error(&self, key: &PreviewKey) -> Option<&str> {
        self.items
            .get(key)?
            .texture
            .as_ref()
            .err()
            .map(String::as_str)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }
}

use super::{ClipInfo, Kind, Preview, PreviewKey, Stamp};
use document_core::{AnimationProject, Document};
use eframe::egui::ColorImage;
use std::{
    collections::hash_map::DefaultHasher,
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, PixmapRef, Transform};

const SIDE: usize = 128;
const MAX_METADATA: usize = 64 * 1024;
const HEADER: usize = 16;
const MAX_CACHE_FILE: usize = HEADER + MAX_METADATA + SIDE * SIDE * 4;

pub(super) fn cache_dir() -> Option<PathBuf> {
    let env = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    #[cfg(windows)]
    let dir = env("LOCALAPPDATA")?.join("Phyro/Cache");
    #[cfg(target_os = "macos")]
    let dir = env("HOME")?.join("Library/Caches/Phyro");
    #[cfg(not(any(windows, target_os = "macos")))]
    let dir = env("XDG_CACHE_HOME")
        .or_else(|| env("HOME").map(|p| p.join(".cache")))?
        .join("phyro");
    Some(dir.join("thumbnails-v1"))
}

pub(super) struct Renderer {
    cache: Option<PathBuf>,
    // Reuse the selected source for frame previews. Browsing still images and
    // projects retains only their small thumbnails, never a folder of projects.
    selected: Option<(PathBuf, Stamp, AnimationProject)>,
}

impl Renderer {
    pub(super) fn new(cache: Option<PathBuf>) -> Self {
        Self {
            cache,
            selected: None,
        }
    }
    pub(super) fn release(&mut self) {
        self.selected = None;
    }
    pub(super) fn selected_path(&self) -> Option<&Path> {
        self.selected.as_ref().map(|(path, _, _)| path.as_path())
    }

    pub(super) fn render(&mut self, key: &PreviewKey, version: Stamp) -> Result<Preview, String> {
        let disk = self
            .cache
            .as_ref()
            .filter(|_| key.frame.is_none() && version.modified.is_some())
            .map(|dir| cache_location(dir, &key.path, version));
        if let Some(preview) = disk
            .as_ref()
            .and_then(|(path, hash)| read_cache(path, *hash))
        {
            return Ok(preview);
        }
        let mut clips = Vec::new();
        let pixmap = match key.kind {
            Kind::Project => {
                let (clip_id, frame) = key.frame.unzip();
                let project;
                let source = if clip_id.is_some() {
                    if !self
                        .selected
                        .as_ref()
                        .is_some_and(|(p, s, _)| p == &key.path && *s == version)
                    {
                        self.release();
                        let project =
                            project_io::load_animation(&key.path).map_err(|e| e.to_string())?;
                        self.selected = Some((key.path.clone(), version, project));
                    }
                    &self.selected.as_ref().unwrap().2
                } else {
                    project = project_io::load_animation(&key.path).map_err(|e| e.to_string())?;
                    clips = project
                        .clips
                        .iter()
                        .map(|c| ClipInfo {
                            id: c.id,
                            name: project.qualified_clip_name(c).into_owned(),
                            frames: c.frames,
                            fps: c.fps,
                        })
                        .collect();
                    &project
                };
                let clip = match clip_id {
                    Some(id) => source.clips.iter().find(|c| c.id == id),
                    None => source.clips.first(),
                }
                .ok_or("This clip is unavailable.")?;
                let frame = frame.unwrap_or(0);
                if frame >= clip.frames {
                    return Err("The preview frame is outside the clip.".into());
                }
                render_drawing(clip.drawing_at(frame), frame)?
            }
            Kind::Svg => {
                let doc = project_io::import_svg(&key.path).map_err(|e| e.to_string())?;
                render_drawing(&doc, 0)?
            }
            Kind::Image => {
                if version.bytes > document_core::RasterImage::MAX_FILE_BYTES as u64 {
                    return Err("This image exceeds the 12 MiB import limit.".into());
                }
                let image =
                    project_io::import_reference_image(&key.path).map_err(|e| e.to_string())?;
                let source = PixmapRef::from_bytes(image.rgba(), image.width(), image.height())
                    .ok_or("Invalid image dimensions.")?;
                let scale = thumbnail_scale(source.width(), source.height()).min(1.);
                let width = (source.width() as f32 * scale).round().max(1.) as u32;
                let height = (source.height() as f32 * scale).round().max(1.) as u32;
                let mut scaled = Pixmap::new(width, height).ok_or("Cannot allocate thumbnail.")?;
                scaled.draw_pixmap(
                    0,
                    0,
                    source,
                    &PixmapPaint {
                        quality: FilterQuality::Bilinear,
                        ..Default::default()
                    },
                    Transform::from_scale(
                        width as f32 / source.width() as f32,
                        height as f32 / source.height() as f32,
                    ),
                    None,
                );
                scaled
            }
            Kind::Folder => return Err("Folders have no artwork preview.".into()),
        };
        if let Some((path, hash)) = disk {
            let _ = write_cache(&path, hash, &pixmap, &clips);
        }
        Ok(Preview {
            image: ColorImage::from_rgba_premultiplied(
                [pixmap.width() as usize, pixmap.height() as usize],
                pixmap.data(),
            ),
            clips,
        })
    }
}

fn render_drawing(doc: &Document, frame: u32) -> Result<Pixmap, String> {
    renderer::render_for_export(doc, frame, thumbnail_scale(doc.width, doc.height))
}

fn thumbnail_scale(w: u32, h: u32) -> f32 {
    SIDE as f32 / w.max(h).max(1) as f32
}

fn cache_location(dir: &Path, source: &Path, version: Stamp) -> (PathBuf, u64) {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    // Fixed slots cap disk usage at 64 MiB regardless of how many folders or
    // versions are browsed. The complete stamp distinguishes slot collisions.
    let slot = hasher.finish() % 512;
    version.hash(&mut hasher);
    (dir.join(format!("{slot:03x}.thumb")), hasher.finish())
}

fn read_cache(path: &Path, hash: u64) -> Option<Preview> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take((MAX_CACHE_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() < HEADER || bytes.len() > MAX_CACHE_FILE {
        return None;
    }
    if u64::from_le_bytes(bytes[0..8].try_into().ok()?) != hash {
        return None;
    }
    let w = u16::from_le_bytes(bytes[8..10].try_into().ok()?) as usize;
    let h = u16::from_le_bytes(bytes[10..12].try_into().ok()?) as usize;
    let n = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    if !(1..=SIDE).contains(&w)
        || !(1..=SIDE).contains(&h)
        || n > MAX_METADATA
        || HEADER + n + w * h * 4 != bytes.len()
    {
        return None;
    }
    let clips: Vec<ClipInfo> = serde_json::from_slice(&bytes[HEADER..HEADER + n]).ok()?;
    if clips.len() > 256
        || clips.iter().any(|c| {
            c.frames == 0
                || c.frames > document_core::MAX_FRAMES
                || c.fps == 0
                || c.fps > document_core::MAX_FPS
        })
    {
        return None;
    }
    let image = ColorImage::from_rgba_premultiplied([w, h], &bytes[HEADER + n..]);
    Some(Preview { image, clips })
}

fn write_cache(path: &Path, hash: u64, pixmap: &Pixmap, clips: &[ClipInfo]) -> std::io::Result<()> {
    let metadata = serde_json::to_vec(clips)?;
    if metadata.len() > MAX_METADATA {
        return Ok(());
    }
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(&hash.to_le_bytes())?;
    temp.write_all(&(pixmap.width() as u16).to_le_bytes())?;
    temp.write_all(&(pixmap.height() as u16).to_le_bytes())?;
    temp.write_all(&(metadata.len() as u32).to_le_bytes())?;
    temp.write_all(&metadata)?;
    temp.write_all(pixmap.data())?;
    temp.persist(path)?;
    Ok(())
}

//! Sample tiny-skia's premultiplied sRGBA bytes before blending onto the canvas.
//! Glow's usual sRGB image sampler decodes RGB before filtering but leaves alpha
//! linear. Re-encoding afterward can make an antialiased edge brighter than
//! both the artwork and the background.
use eframe::{
    egui::{self, Color32, Context, Id, Painter, Pos2, Rect, Shape, TextureId},
    egui_glow::{self, ShaderVersion},
    glow::{self, HasContext},
};
use std::sync::Arc;

// The native display filter supplies its own bilinear sampler. The ordinary
// egui mesh on older GL versions must keep RGB and alpha from the same texel.
pub(super) const ARTWORK_TEXTURE: egui::TextureOptions = egui::TextureOptions::NEAREST;
const TEXTURE_SRGB_DECODE_EXT: u32 = 0x8A48;
const SKIP_DECODE_EXT: i32 = 0x8A4A;

pub(super) struct Image {
    texture: TextureId,
    corners: [Pos2; 4],
    uv: Rect,
    tint: Color32,
}

impl Image {
    pub(super) fn affine(texture: TextureId, corners: [Pos2; 4]) -> Self {
        Self {
            texture,
            corners,
            uv: Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
            tint: Color32::WHITE,
        }
    }

    pub(super) fn rectangular(texture: TextureId, rect: Rect, uv: Rect, tint: Color32) -> Self {
        Self {
            texture,
            corners: corners(rect),
            uv,
            tint,
        }
    }

    fn mesh(self) -> Shape {
        let mut mesh = egui::Mesh::with_texture(self.texture);
        mesh.vertices = self
            .corners
            .into_iter()
            .zip(corners(self.uv))
            .map(|(pos, uv)| egui::epaint::Vertex {
                pos,
                uv,
                color: self.tint,
            })
            .collect();
        mesh.indices = vec![0, 1, 2, 2, 1, 3];
        Shape::mesh(mesh)
    }
}

fn corners(r: Rect) -> [Pos2; 4] {
    [
        r.left_top(),
        r.right_top(),
        r.left_bottom(),
        r.right_bottom(),
    ]
}

pub(super) fn image(p: &Painter, texture: TextureId, rect: Rect, uv: Rect, tint: Color32) -> Shape {
    batch(p, vec![Image::rectangular(texture, rect, uv, tint)])
}

/// Screen corners in texture order: top-left, top-right, bottom-left, bottom-right.
pub(super) fn affine_image(p: &Painter, texture: TextureId, corners: [Pos2; 4]) -> Shape {
    batch(p, vec![Image::affine(texture, corners)])
}

const GRAYSCALE_ID: &str = "canvas grayscale preview";

pub(crate) fn grayscale(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp(Id::new(GRAYSCALE_ID)).unwrap_or(false))
}

pub(crate) fn set_grayscale(ctx: &Context, enabled: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new(GRAYSCALE_ID), enabled));
}

pub(super) fn batch(p: &Painter, mut images: Vec<Image>) -> Shape {
    if images.is_empty() {
        return Shape::Noop;
    }
    let (filter, grayscale) = p.ctx().data(|data| {
        (
            data.get_temp::<Arc<Filter>>(Id::new("canvas display filter")),
            data.get_temp(Id::new(GRAYSCALE_ID)).unwrap_or(false),
        )
    });
    let Some(filter) = filter else {
        // Headless UI and older GL contexts use the same antialiased CPU
        // artwork. Nearest sampling there keeps each texel's RGB and alpha paired.
        return if images.len() == 1 {
            images.pop().unwrap().mesh()
        } else {
            Shape::Vec(images.into_iter().map(Image::mesh).collect())
        };
    };
    let rect = p
        .clip_rect()
        .intersect(images.iter().fold(Rect::NOTHING, |r, image| {
            r.union(Rect::from_points(&image.corners))
        }));
    Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
            filter.draw(&info, painter, &images, grayscale);
        })),
    })
}

pub(crate) struct Filter {
    program: glow::Program,
    vao: glow::VertexArray,
    sampler: Option<glow::Sampler>,
    destination: Option<glow::UniformLocation>,
    uv: Option<glow::UniformLocation>,
    tint: Option<glow::UniformLocation>,
    grayscale: Option<glow::UniformLocation>,
}

impl Filter {
    pub(crate) fn install(ctx: &Context, gl: &glow::Context) -> Option<Arc<Self>> {
        let version = ShaderVersion::get(gl);
        if !version.is_new_shader_interface() {
            return None;
        }
        let extensions = gl.supported_extensions();
        // Matches how egui_glow chooses the format of managed ColorImages.
        let srgb = version == ShaderVersion::Es300 || extensions.iter().any(|s| s.contains("sRGB"));
        let skip_decode = extensions
            .iter()
            .any(|s| s.ends_with("EXT_texture_sRGB_decode"));
        let context = gl.version();
        let sampler_supported = context.is_embedded
            || (context.major, context.minor) >= (3, 3)
            || extensions
                .iter()
                .any(|s| s.ends_with("ARB_sampler_objects"));
        // SAFETY: eframe calls the app creator with its GL context current.
        unsafe {
            let vao = gl.create_vertex_array().ok()?;
            let sampler = (sampler_supported && (!srgb || skip_decode))
                .then(|| gl.create_sampler().ok())
                .flatten();
            if let Some(s) = sampler {
                for param in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                    gl.sampler_parameter_i32(s, param, glow::LINEAR as i32);
                }
                for param in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                    gl.sampler_parameter_i32(s, param, glow::CLAMP_TO_EDGE as i32);
                }
                if srgb {
                    // GL_EXT_texture_sRGB_decode: filter the stored bytes directly.
                    gl.sampler_parameter_i32(s, TEXTURE_SRGB_DECODE_EXT, SKIP_DECODE_EXT);
                }
            }
            let prefix = format!(
                "{}{}\n#define HARDWARE_FILTER {}\n#define SRGB_SOURCE {}\n",
                version.version_declaration(),
                if version.is_embedded() {
                    "precision highp float;\nprecision highp int;\nprecision highp sampler2D;"
                } else {
                    ""
                },
                sampler.is_some() as u8,
                srgb as u8,
            );
            let program = match compile(gl, &prefix) {
                Ok(program) => program,
                Err(error) => {
                    gl.delete_vertex_array(vao);
                    if let Some(s) = sampler {
                        gl.delete_sampler(s);
                    }
                    eprintln!("Canvas image filter unavailable: {error}");
                    return None;
                }
            };
            let filter = Arc::new(Self {
                program,
                vao,
                sampler,
                destination: gl.get_uniform_location(program, "dst"),
                uv: gl.get_uniform_location(program, "uv_rect"),
                tint: gl.get_uniform_location(program, "tint"),
                grayscale: gl.get_uniform_location(program, "grayscale"),
            });
            ctx.data_mut(|data| data.insert_temp(Id::new("canvas display filter"), filter.clone()));
            Some(filter)
        }
    }

    fn draw(
        &self,
        info: &egui::PaintCallbackInfo,
        painter: &egui_glow::Painter,
        images: &[Image],
        grayscale: bool,
    ) {
        let [w, h] = info.screen_size_px;
        let sx = 2. * info.pixels_per_point / w as f32;
        let sy = -2. * info.pixels_per_point / h as f32;
        let gl = painter.gl();
        // SAFETY: the Glow painter owns the current context and textures. Its
        // scissor and premultiplied blending remain active throughout this batch.
        unsafe {
            gl.viewport(0, 0, w as i32, h as i32);
            gl.bind_vertex_array(Some(self.vao));
            gl.use_program(Some(self.program));
            gl.uniform_1_i32(self.grayscale.as_ref(), i32::from(grayscale));
            if let Some(s) = self.sampler {
                gl.bind_sampler(0, Some(s));
            }
            for image in images {
                let Some(texture) = painter.texture(image.texture) else {
                    continue;
                };
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                let [origin, right, bottom, _] = image.corners;
                let x = right - origin;
                let y = bottom - origin;
                gl.uniform_matrix_3x2_f32_slice(
                    self.destination.as_ref(),
                    false,
                    &[
                        x.x * sx,
                        x.y * sy,
                        y.x * sx,
                        y.y * sy,
                        origin.x * sx - 1.,
                        origin.y * sy + 1.,
                    ],
                );
                let uv = image.uv;
                gl.uniform_4_f32(
                    self.uv.as_ref(),
                    uv.min.x,
                    uv.min.y,
                    uv.width(),
                    uv.height(),
                );
                let [r, g, b, a] = image.tint.to_array().map(|c| f32::from(c) / 255.);
                gl.uniform_4_f32(self.tint.as_ref(), r, g, b, a);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }
            if self.sampler.is_some() {
                gl.bind_sampler(0, None);
            }
        }
    }

    pub(crate) fn destroy(&self, gl: &glow::Context) {
        // SAFETY: called once by eframe on exit, with the creating context current.
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            if let Some(s) = self.sampler {
                gl.delete_sampler(s);
            }
        }
    }
}

unsafe fn compile(gl: &glow::Context, prefix: &str) -> Result<glow::Program, String> {
    // SAFETY: caller supplies the current context. Flag shaders for deletion as
    // soon as they're attached so deleting the program cleans them up on error.
    unsafe {
        let program = gl.create_program()?;
        for (kind, body) in [
            (glow::VERTEX_SHADER, include_str!("display.vert")),
            (glow::FRAGMENT_SHADER, include_str!("display.frag")),
        ] {
            let shader = match gl.create_shader(kind) {
                Ok(shader) => shader,
                Err(error) => {
                    gl.delete_program(program);
                    return Err(error);
                }
            };
            gl.shader_source(shader, &format!("{prefix}{body}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let error = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                gl.delete_program(program);
                return Err(error);
            }
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        gl.link_program(program);
        if gl.get_program_link_status(program) {
            Ok(program)
        } else {
            let error = gl.get_program_info_log(program);
            gl.delete_program(program);
            Err(error)
        }
    }
}

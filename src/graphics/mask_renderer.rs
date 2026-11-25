//! Camera masks: pieces of the pre-rendered background redrawn in front of
//! the 3D models at their scene depth, so characters walk behind scenery.
//!
//! Migrated from Go `render/cameramask.go`, `Camera.NormalizeMaskDepth`,
//! `geometry.NewCameraMaskQuad` and `Image16Bit.ApplyMask`. Go writes
//! `gl_FragDepth = 1` for transparent mask pixels; GLES 2.0 has no
//! `gl_FragDepth`, so Rust discards those fragments, which has the same
//! effect (no color and no depth written).

use std::{ffi::CString, mem};

use super::texture::Texture;
use crate::fileio::{adt::AdtImage, rdt::MaskRect, rdt::RdtCamera, tim::TimImage};
use crate::math::{transform_point, Mat4};

const BACKGROUND_WIDTH: f32 = 320.0;
const BACKGROUND_HEIGHT: f32 = 240.0;
/// Go `ExtractRoomBackground`: the mask TIM starts after 320×256 words.
const MASK_TIM_OFFSET: usize = 320 * 256 * 2;

const VERTEX_SHADER_SOURCE: &str = r#"
attribute vec3 a_position;
attribute vec2 a_uv;
varying vec2 v_uv;

void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_position, 1.0);
}
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
precision mediump float;
uniform sampler2D u_texture;
varying vec2 v_uv;

void main() {
    vec4 color = texture2D(u_texture, v_uv);
    if (color.a < 0.5) {
        discard;
    }
    gl_FragColor = vec4(color.rgb, 1.0);
}
"#;

/// x, y, z in clip space (w = 1) followed by u, v.
type MaskVertex = [f32; 5];

pub struct MaskRenderer {
    program: gl::types::GLuint,
    vertex_buffer: gl::types::GLuint,
    position_attribute: gl::types::GLuint,
    uv_attribute: gl::types::GLuint,
    texture_uniform: gl::types::GLint,
    texture: Option<Texture>,
    vertex_count: gl::types::GLsizei,
}

impl MaskRenderer {
    pub fn new() -> Result<Self, String> {
        let program = link_program()?;
        let attribute = |name: &str| -> Result<gl::types::GLuint, String> {
            let name = CString::new(name).map_err(|error| error.to_string())?;
            let location = unsafe { gl::GetAttribLocation(program, name.as_ptr()) };
            gl::types::GLuint::try_from(location)
                .map_err(|_| "mask shader is missing an attribute".to_owned())
        };
        let texture_name = CString::new("u_texture").map_err(|error| error.to_string())?;
        let texture_uniform = unsafe { gl::GetUniformLocation(program, texture_name.as_ptr()) };
        let (position_attribute, uv_attribute) = match (attribute("a_position"), attribute("a_uv"))
        {
            (Ok(position), Ok(uv)) if texture_uniform >= 0 => (position, uv),
            _ => {
                unsafe { gl::DeleteProgram(program) };
                return Err("mask shader did not expose its attributes and uniforms".to_owned());
            }
        };
        let mut vertex_buffer = 0;
        unsafe { gl::GenBuffers(1, &mut vertex_buffer) };
        if vertex_buffer == 0 {
            unsafe { gl::DeleteProgram(program) };
            return Err("glGenBuffers returned buffer 0 for camera masks".to_owned());
        }
        Ok(Self {
            program,
            vertex_buffer,
            position_attribute,
            uv_attribute,
            texture_uniform,
            texture: None,
            vertex_count: 0,
        })
    }

    /// Rebuilds the mask texture and quads for a camera. A camera without a
    /// mask TIM or PRI tiles simply draws nothing.
    pub fn update(
        &mut self,
        background: &AdtImage,
        masks: &[MaskRect],
        camera: &RdtCamera,
        view_projection: &Mat4,
        fit: [f32; 2],
    ) -> Result<(), String> {
        self.vertex_count = 0;
        self.texture = None;
        let Some(mask_tim) = mask_tim(background)? else {
            return Ok(());
        };
        if masks.is_empty() {
            return Ok(());
        }
        let rgba = compose_mask_rgba(
            &background.to_rgba(),
            background.width() as usize,
            background.height() as usize,
            &mask_tim,
            masks,
        );
        let vertices = mask_vertices(masks, camera, view_projection, fit);
        let texture =
            Texture::from_rgba(background.width(), background.height(), rgba, "camera mask")?;
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, texture.id());
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vertex_buffer);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * mem::size_of::<MaskVertex>()) as gl::types::GLsizeiptr,
                vertices.as_ptr().cast(),
                gl::STATIC_DRAW,
            );
        }
        self.vertex_count = gl::types::GLsizei::try_from(vertices.len())
            .map_err(|_| "too many camera mask vertices".to_owned())?;
        self.texture = Some(texture);
        Ok(())
    }

    /// Draws the masks into color and depth. Call after clearing the depth
    /// buffer and before drawing models.
    pub fn render(&self) {
        let Some(texture) = self.texture.as_ref() else {
            return;
        };
        if self.vertex_count == 0 {
            return;
        }
        let stride = mem::size_of::<MaskVertex>() as gl::types::GLsizei;
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::DepthFunc(gl::LESS);
            gl::DepthMask(gl::TRUE);
            gl::UseProgram(self.program);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, texture.id());
            gl::Uniform1i(self.texture_uniform, 0);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vertex_buffer);
            gl::EnableVertexAttribArray(self.position_attribute);
            gl::VertexAttribPointer(
                self.position_attribute,
                3,
                gl::FLOAT,
                gl::FALSE,
                stride,
                std::ptr::null(),
            );
            gl::EnableVertexAttribArray(self.uv_attribute);
            gl::VertexAttribPointer(
                self.uv_attribute,
                2,
                gl::FLOAT,
                gl::FALSE,
                stride,
                (3 * mem::size_of::<f32>()) as *const _,
            );
            gl::DrawArrays(gl::TRIANGLES, 0, self.vertex_count);
            gl::DisableVertexAttribArray(self.position_attribute);
            gl::DisableVertexAttribArray(self.uv_attribute);
            gl::Disable(gl::DEPTH_TEST);
        }
    }
}

impl Drop for MaskRenderer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.vertex_buffer);
            gl::DeleteProgram(self.program);
        }
    }
}

/// The TIM stored after the background in a `roomcut.bin` entry, if any.
pub fn mask_tim(background: &AdtImage) -> Result<Option<TimImage>, String> {
    let raw = background.raw_data();
    if raw.len() <= MASK_TIM_OFFSET {
        return Ok(None);
    }
    crate::fileio::tim::parse_tim(&raw[MASK_TIM_OFFSET..])
        .map(Some)
        .map_err(|error| format!("camera mask TIM: {error}"))
}

/// Go `BuildCameraMask`: background colors with the alpha of the mask TIM
/// pixel (transparent where the mask word is zero) inside each tile.
pub fn compose_mask_rgba(
    background_rgba: &[u8],
    width: usize,
    height: usize,
    mask: &TimImage,
    masks: &[MaskRect],
) -> Vec<u8> {
    let mut rgba = background_rgba.to_vec();
    for rect in masks {
        for offset_y in 0..usize::from(rect.height) {
            for offset_x in 0..usize::from(rect.width) {
                let source_x = usize::from(rect.src_x) + offset_x;
                let source_y = usize::from(rect.src_y) + offset_y;
                let dest_x = rect.dest_x as usize + offset_x;
                let dest_y = rect.dest_y as usize + offset_y;
                if source_x >= mask.width()
                    || source_y >= mask.height()
                    || dest_x >= width
                    || dest_y >= height
                {
                    continue;
                }
                let source = mask.pixels()[source_y * mask.width() + source_x];
                rgba[(dest_y * width + dest_x) * 4 + 3] = if source == 0 { 0 } else { 255 };
            }
        }
    }
    rgba
}

/// Go `NormalizeMaskDepth`: the point `depth × 32` units in front of the
/// camera, projected to normalized device depth.
pub fn mask_depth(camera: &RdtCamera, view_projection: &Mat4, depth: u16) -> f32 {
    let from = camera.from.map(|value| value as f32);
    let to = camera.to.map(|value| value as f32);
    let direction = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = (direction[0].powi(2) + direction[1].powi(2) + direction[2].powi(2)).sqrt();
    if length <= f32::EPSILON {
        return 1.0;
    }
    let distance = f32::from(depth) * 32.0 / length;
    let point = [
        from[0] + direction[0] * distance,
        from[1] + direction[1] * distance,
        from[2] + direction[2] * distance,
    ];
    let clip = transform_point(view_projection, point);
    if clip[3].abs() <= f32::EPSILON {
        return 1.0;
    }
    (clip[2] / clip[3]).clamp(-1.0, 1.0)
}

/// Two triangles per tile in the same fitted screen space as the background.
fn mask_vertices(
    masks: &[MaskRect],
    camera: &RdtCamera,
    view_projection: &Mat4,
    fit: [f32; 2],
) -> Vec<MaskVertex> {
    let mut vertices = Vec::with_capacity(masks.len() * 6);
    for rect in masks {
        let z = mask_depth(camera, view_projection, rect.depth);
        let left = rect.dest_x as f32;
        let top = rect.dest_y as f32;
        let right = left + f32::from(rect.width);
        let bottom = top + f32::from(rect.height);
        let corner = |x: f32, y: f32| -> MaskVertex {
            [
                (2.0 * x / BACKGROUND_WIDTH - 1.0) * fit[0],
                (1.0 - 2.0 * y / BACKGROUND_HEIGHT) * fit[1],
                z,
                x / BACKGROUND_WIDTH,
                y / BACKGROUND_HEIGHT,
            ]
        };
        let corners = [
            corner(left, top),
            corner(right, top),
            corner(right, bottom),
            corner(left, bottom),
        ];
        for index in [0, 1, 2, 0, 2, 3] {
            vertices.push(corners[index]);
        }
    }
    vertices
}

fn link_program() -> Result<gl::types::GLuint, String> {
    let compile = |kind, source: &str| -> Result<gl::types::GLuint, String> {
        let shader = unsafe { gl::CreateShader(kind) };
        let source = CString::new(source).map_err(|error| error.to_string())?;
        unsafe {
            gl::ShaderSource(shader, 1, &source.as_ptr(), std::ptr::null());
            gl::CompileShader(shader);
        }
        let mut status = 0;
        unsafe { gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut status) };
        if status != gl::TRUE as i32 {
            unsafe { gl::DeleteShader(shader) };
            return Err("camera mask shader failed to compile".to_owned());
        }
        Ok(shader)
    };
    let vertex = compile(gl::VERTEX_SHADER, VERTEX_SHADER_SOURCE)?;
    let fragment = match compile(gl::FRAGMENT_SHADER, FRAGMENT_SHADER_SOURCE) {
        Ok(shader) => shader,
        Err(error) => {
            unsafe { gl::DeleteShader(vertex) };
            return Err(error);
        }
    };
    let program = unsafe { gl::CreateProgram() };
    let position = CString::new("a_position").map_err(|error| error.to_string())?;
    let uv = CString::new("a_uv").map_err(|error| error.to_string())?;
    unsafe {
        gl::AttachShader(program, vertex);
        gl::AttachShader(program, fragment);
        gl::BindAttribLocation(program, 0, position.as_ptr());
        gl::BindAttribLocation(program, 1, uv.as_ptr());
        gl::LinkProgram(program);
        gl::DeleteShader(vertex);
        gl::DeleteShader(fragment);
    }
    let mut status = 0;
    unsafe { gl::GetProgramiv(program, gl::LINK_STATUS, &mut status) };
    if status != gl::TRUE as i32 {
        unsafe { gl::DeleteProgram(program) };
        return Err("camera mask shader failed to link".to_owned());
    }
    Ok(program)
}

#[cfg(test)]
mod tests {
    use super::{compose_mask_rgba, mask_depth, mask_vertices};
    use crate::fileio::rdt::{MaskRect, RdtCamera};
    use crate::fileio::tim::{parse_tim, tests::tim_4bpp};
    use crate::graphics::model_renderer::camera_mvp;
    use crate::math::identity;

    fn camera() -> RdtCamera {
        RdtCamera {
            flag: 0,
            distance_to_screen: 26_684,
            from: [11268, -2664, -3600],
            to: [22356, -2466, 522],
            mask_offset: None,
            mask_count: None,
            fov_degrees: 59.963276,
        }
    }

    #[test]
    fn mask_alpha_follows_nonzero_mask_pixels() {
        // 4×1 mask: palette color index 0 is word 0 (transparent).
        let tim = parse_tim(&tim_4bpp(1, 1, 1, &[0x10, 0x01])).expect("valid TIM");
        let background = vec![200_u8; 4 * 4 * 2];
        let rect = MaskRect {
            src_x: 0,
            src_y: 0,
            dest_x: 0,
            dest_y: 1,
            depth: 10,
            width: 4,
            height: 1,
        };
        let rgba = compose_mask_rgba(&background, 4, 2, &tim, &[rect]);
        let alphas: Vec<u8> = (0..4).map(|x| rgba[(4 + x) * 4 + 3]).collect();
        assert_eq!(alphas, [0, 255, 255, 0]);
        // Row 0 is untouched.
        assert_eq!(rgba[3], 200);
    }

    #[test]
    fn farther_masks_get_larger_depth() {
        let vp = camera_mvp(&camera(), 4.0 / 3.0, [0.9, 0.9], identity()).expect("camera");
        let near = mask_depth(&camera(), &vp, 50);
        let far = mask_depth(&camera(), &vp, 400);
        assert!(near < far, "near={near} far={far}");
        assert!((-1.0..=1.0).contains(&near) && (-1.0..=1.0).contains(&far));
    }

    #[test]
    fn quads_match_the_fitted_background_space() {
        let vp = camera_mvp(&camera(), 4.0 / 3.0, [0.5, 0.5], identity()).expect("camera");
        let rect = MaskRect {
            src_x: 0,
            src_y: 0,
            dest_x: 0,
            dest_y: 0,
            depth: 100,
            width: 320,
            height: 240,
        };
        let vertices = mask_vertices(&[rect], &camera(), &vp, [0.5, 0.5]);
        assert_eq!(vertices.len(), 6);
        assert_eq!(&vertices[0][..2], &[-0.5, 0.5]);
        assert_eq!(&vertices[2][..2], &[0.5, -0.5]);
        assert_eq!(&vertices[2][3..], &[1.0, 1.0]);
    }
}

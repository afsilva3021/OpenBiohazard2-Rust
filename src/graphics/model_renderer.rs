use std::{ffi::CString, mem};

use super::texture::Texture;
use crate::fileio::{
    pld::{ModelVertex, PldModel},
    rdt::RdtCamera,
};
use crate::math::{look_at, multiply, perspective, rotation_y, scale, translation, Mat4};

// GLSL ES 1.00 port of the Go entity shader: model × boneOffset × position,
// textured with the PLD TIM. Go multiplies by its RDT environment light; that
// light is not loaded yet, so the texture is shown at full brightness.
const VERTEX_SHADER_SOURCE: &str = r#"
attribute vec3 a_position;
attribute vec2 a_uv;
uniform mat4 u_mvp;
uniform mat4 u_bone;
varying vec2 v_uv;

void main() {
    v_uv = a_uv;
    gl_Position = u_mvp * u_bone * vec4(a_position, 1.0);
}
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
precision mediump float;
uniform sampler2D u_texture;
varying vec2 v_uv;

void main() {
    gl_FragColor = vec4(texture2D(u_texture, v_uv).rgb, 1.0);
}
"#;

const CAMERA_NEAR: f32 = 16.0;
const CAMERA_FAR: f32 = 45_000.0;

/// World placement of the model (Go `Player.GetModelMatrix`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub position: [f32; 3],
    pub yaw_degrees: f32,
}

impl Placement {
    pub fn model_matrix(&self) -> Mat4 {
        multiply(
            translation(self.position),
            rotation_y(self.yaw_degrees.to_radians()),
        )
    }
}

pub struct ModelRenderer {
    program: gl::types::GLuint,
    vertex_buffer: gl::types::GLuint,
    texture: Texture,
    position_attribute: gl::types::GLuint,
    uv_attribute: gl::types::GLuint,
    mvp_uniform: gl::types::GLint,
    bone_uniform: gl::types::GLint,
    texture_uniform: gl::types::GLint,
    components: Vec<(gl::types::GLint, gl::types::GLsizei)>,
    /// Projection × view (with the background fit) for the active camera.
    view_projection: Mat4,
}

impl ModelRenderer {
    pub fn new(model: &PldModel, depth_bits: u8) -> Result<Self, String> {
        if depth_bits < 16 {
            return Err(format!(
                "3D model rendering needs a depth buffer of at least 16 bits; context reports {depth_bits}"
            ));
        }
        let buffer_bytes = model
            .vertices
            .len()
            .checked_mul(mem::size_of::<ModelVertex>())
            .filter(|&size| size <= isize::MAX as usize)
            .ok_or_else(|| "PLD vertex buffer size exceeds OpenGL ES limits".to_owned())?;
        let components = model
            .components
            .iter()
            .map(|range| {
                Ok((
                    gl::types::GLint::try_from(range.first)
                        .map_err(|_| "PLD vertex offset exceeds OpenGL ES limits".to_owned())?,
                    gl::types::GLsizei::try_from(range.count)
                        .map_err(|_| "PLD vertex count exceeds OpenGL ES limits".to_owned())?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;

        let tim = &model.texture;
        let texture = Texture::from_rgba(
            tim.width() as u32,
            tim.height() as u32,
            tim.to_rgba(),
            "PLD TIM texture",
        )?;
        // Go samples PLD textures with GL_NEAREST.
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, texture.id());
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_MIN_FILTER,
                gl::NEAREST as gl::types::GLint,
            );
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_MAG_FILTER,
                gl::NEAREST as gl::types::GLint,
            );
        }

        let bindings = create_model_program()?;
        let mut vertex_buffer = 0;
        unsafe { gl::GenBuffers(1, &mut vertex_buffer) };
        if vertex_buffer == 0 {
            unsafe { gl::DeleteProgram(bindings.program) };
            return Err("glGenBuffers returned buffer 0 for PLD model".to_owned());
        }
        unsafe {
            gl::BindBuffer(gl::ARRAY_BUFFER, vertex_buffer);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                buffer_bytes as gl::types::GLsizeiptr,
                model.vertices.as_ptr().cast(),
                gl::STATIC_DRAW,
            );
        }
        let upload_error = unsafe { gl::GetError() };
        if upload_error != gl::NO_ERROR {
            unsafe {
                gl::DeleteBuffers(1, &vertex_buffer);
                gl::DeleteProgram(bindings.program);
            }
            return Err(format!(
                "OpenGL ES failed to upload PLD vertex buffer (error 0x{upload_error:04x})"
            ));
        }
        println!("SDL/OpenGL depth buffer: {depth_bits} bits");

        Ok(Self {
            program: bindings.program,
            vertex_buffer,
            texture,
            position_attribute: bindings.position_attribute as gl::types::GLuint,
            uv_attribute: bindings.uv_attribute as gl::types::GLuint,
            mvp_uniform: bindings.mvp_uniform,
            bone_uniform: bindings.bone_uniform,
            texture_uniform: bindings.texture_uniform,
            components,
            view_projection: crate::math::identity(),
        })
    }

    /// Validates and returns the camera matrix without changing state, so a
    /// failed camera switch can keep the previous view.
    pub fn prepare_camera(
        &self,
        camera: &RdtCamera,
        projection_aspect: f32,
        screen_fit: [f32; 2],
    ) -> Result<Mat4, String> {
        camera_mvp(
            camera,
            projection_aspect,
            screen_fit,
            crate::math::identity(),
        )
    }

    pub fn set_view_projection(&mut self, view_projection: Mat4) {
        self.view_projection = view_projection;
    }

    /// Draws every MD1 component with its bone transform. The caller is
    /// responsible for the depth buffer contents (background masks).
    pub fn render(&self, placement: &Placement, bones: &[Mat4]) {
        let mvp = multiply(self.view_projection, placement.model_matrix());
        let stride = mem::size_of::<ModelVertex>() as gl::types::GLsizei;
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::DepthFunc(gl::LESS);
            gl::DepthMask(gl::TRUE);
            gl::UseProgram(self.program);
            gl::UniformMatrix4fv(self.mvp_uniform, 1, gl::FALSE, mvp.as_ptr());
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, self.texture.id());
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
            let neutral = crate::math::identity();
            for (index, &(first, count)) in self.components.iter().enumerate() {
                let bone = bones.get(index).unwrap_or(&neutral);
                gl::UniformMatrix4fv(self.bone_uniform, 1, gl::FALSE, bone.as_ptr());
                gl::DrawArrays(gl::TRIANGLES, first, count);
            }
            gl::DisableVertexAttribArray(self.position_attribute);
            gl::DisableVertexAttribArray(self.uv_attribute);
            gl::Disable(gl::DEPTH_TEST);
        }
    }
}

impl Drop for ModelRenderer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.vertex_buffer);
            gl::DeleteProgram(self.program);
        }
    }
}

pub fn camera_mvp(
    camera: &RdtCamera,
    projection_aspect: f32,
    screen_fit: [f32; 2],
    model_matrix: Mat4,
) -> Result<Mat4, String> {
    if !projection_aspect.is_finite() || projection_aspect <= 0.0 {
        return Err(format!(
            "invalid 3D projection aspect ratio {projection_aspect}"
        ));
    }
    if screen_fit
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err("invalid background fit scale for 3D overlay".to_owned());
    }
    let fov = camera.fov_degrees.to_radians();
    if !fov.is_finite() || fov <= 0.0 || fov >= std::f32::consts::PI {
        return Err(format!(
            "invalid RDT camera field of view {}°",
            camera.fov_degrees
        ));
    }

    let eye = camera.from.map(|value| value as f32);
    let target = camera.to.map(|value| value as f32);
    let view = look_at(eye, target, [0.0, -1.0, 0.0])?;
    let projection = perspective(fov, projection_aspect, CAMERA_NEAR, CAMERA_FAR);
    let fit = scale(screen_fit[0], screen_fit[1], 1.0);
    Ok(multiply(
        fit,
        multiply(projection, multiply(view, model_matrix)),
    ))
}

fn create_model_program() -> Result<ProgramBindings, String> {
    let vertex_shader = compile_shader(gl::VERTEX_SHADER, "model vertex", VERTEX_SHADER_SOURCE)?;
    let fragment_shader = match compile_shader(
        gl::FRAGMENT_SHADER,
        "model fragment",
        FRAGMENT_SHADER_SOURCE,
    ) {
        Ok(shader) => shader,
        Err(error) => {
            unsafe { gl::DeleteShader(vertex_shader) };
            return Err(error);
        }
    };
    let program = unsafe { gl::CreateProgram() };
    if program == 0 {
        unsafe {
            gl::DeleteShader(vertex_shader);
            gl::DeleteShader(fragment_shader);
        }
        return Err("glCreateProgram returned program 0 for PLD model".to_owned());
    }
    let position_name = CString::new("a_position").map_err(|error| error.to_string())?;
    let uv_name = CString::new("a_uv").map_err(|error| error.to_string())?;
    unsafe {
        gl::AttachShader(program, vertex_shader);
        gl::AttachShader(program, fragment_shader);
        gl::BindAttribLocation(program, 0, position_name.as_ptr());
        gl::BindAttribLocation(program, 1, uv_name.as_ptr());
        gl::LinkProgram(program);
    }
    let mut linked = gl::FALSE as gl::types::GLint;
    unsafe { gl::GetProgramiv(program, gl::LINK_STATUS, &mut linked) };
    unsafe {
        gl::DeleteShader(vertex_shader);
        gl::DeleteShader(fragment_shader);
    }
    if linked != gl::TRUE as gl::types::GLint {
        let log = program_info_log(program);
        eprintln!("[model shader] program link failed:\n{log}");
        unsafe { gl::DeleteProgram(program) };
        return Err(format!("PLD model shader link failed: {log}"));
    }

    let uniform = |name: &str| -> Result<gl::types::GLint, String> {
        let name = CString::new(name).map_err(|error| error.to_string())?;
        Ok(unsafe { gl::GetUniformLocation(program, name.as_ptr()) })
    };
    let bindings = ProgramBindings {
        program,
        position_attribute: unsafe { gl::GetAttribLocation(program, position_name.as_ptr()) },
        uv_attribute: unsafe { gl::GetAttribLocation(program, uv_name.as_ptr()) },
        mvp_uniform: uniform("u_mvp")?,
        bone_uniform: uniform("u_bone")?,
        texture_uniform: uniform("u_texture")?,
    };
    if bindings.position_attribute < 0
        || bindings.uv_attribute < 0
        || bindings.mvp_uniform < 0
        || bindings.bone_uniform < 0
        || bindings.texture_uniform < 0
    {
        unsafe { gl::DeleteProgram(program) };
        return Err(
            "PLD model shader did not expose its required attributes and uniforms".to_owned(),
        );
    }
    Ok(bindings)
}

fn compile_shader(kind: gl::types::GLenum, label: &str, source: &str) -> Result<u32, String> {
    let shader = unsafe { gl::CreateShader(kind) };
    if shader == 0 {
        return Err(format!("glCreateShader returned 0 for {label}"));
    }
    let source = CString::new(source).map_err(|error| error.to_string())?;
    unsafe {
        gl::ShaderSource(shader, 1, &source.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);
    }
    let mut compiled = gl::FALSE as gl::types::GLint;
    unsafe { gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut compiled) };
    if compiled != gl::TRUE as gl::types::GLint {
        let log = shader_info_log(shader);
        eprintln!("[model shader] {label} compilation failed:\n{log}");
        unsafe { gl::DeleteShader(shader) };
        return Err(format!(
            "PLD model {label} shader compilation failed: {log}"
        ));
    }
    Ok(shader)
}

fn shader_info_log(shader: gl::types::GLuint) -> String {
    let mut length = 0;
    unsafe { gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut length) };
    if length <= 1 {
        return "(empty shader info log)".to_owned();
    }
    let mut bytes = vec![0_u8; length as usize];
    let mut written = 0;
    unsafe {
        gl::GetShaderInfoLog(shader, length, &mut written, bytes.as_mut_ptr().cast());
    }
    bytes.truncate(written.max(0) as usize);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn program_info_log(program: gl::types::GLuint) -> String {
    let mut length = 0;
    unsafe { gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut length) };
    if length <= 1 {
        return "(empty program info log)".to_owned();
    }
    let mut bytes = vec![0_u8; length as usize];
    let mut written = 0;
    unsafe {
        gl::GetProgramInfoLog(program, length, &mut written, bytes.as_mut_ptr().cast());
    }
    bytes.truncate(written.max(0) as usize);
    String::from_utf8_lossy(&bytes).into_owned()
}

struct ProgramBindings {
    program: gl::types::GLuint,
    position_attribute: gl::types::GLint,
    uv_attribute: gl::types::GLint,
    mvp_uniform: gl::types::GLint,
    bone_uniform: gl::types::GLint,
    texture_uniform: gl::types::GLint,
}

#[cfg(test)]
mod tests {
    use super::camera_mvp;
    use crate::fileio::rdt::RdtCamera;
    use crate::math::{identity, look_at, multiply, perspective, rotation_y, translation};

    fn camera(from: [i32; 3], to: [i32; 3], fov_degrees: f32) -> RdtCamera {
        RdtCamera {
            flag: 0,
            distance_to_screen: 1,
            from,
            to,
            mask_offset: None,
            mask_count: None,
            fov_degrees,
        }
    }

    #[test]
    fn go_camera_target_maps_to_negative_view_z() {
        let view = look_at([0.0, 0.0, 0.0], [0.0, 0.0, 10.0], [0.0, -1.0, 0.0])
            .expect("valid camera basis");
        let target = [
            view[8] * 10.0 + view[12],
            view[9] * 10.0 + view[13],
            view[10] * 10.0 + view[14],
        ];
        assert!((target[2] + 10.0).abs() < 1e-5);
    }

    #[test]
    fn projection_and_model_transform_are_finite_for_room1000_camera() {
        let room_camera = camera([11268, -2664, -3600], [22356, -2466, 522], 59.963276);
        let model = multiply(
            translation([18800.0, 0.0, -3160.0]),
            rotation_y(180_f32.to_radians()),
        );
        let mvp = camera_mvp(&room_camera, 4.0 / 3.0, [0.9, 0.9], model)
            .expect("verified ROOM1000 camera and Go diagnostic placement");
        assert!(mvp.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn rejects_degenerate_or_invalid_camera_projection_data() {
        assert!(look_at([1.0; 3], [1.0; 3], [0.0, -1.0, 0.0]).is_err());
        let invalid = camera([0, 0, 0], [0, 0, 0], 0.0);
        assert!(camera_mvp(&invalid, 4.0 / 3.0, [0.9, 0.9], identity()).is_err());
        let valid = perspective(1.0, 4.0 / 3.0, 16.0, 45_000.0);
        assert!(valid.iter().all(|value| value.is_finite()));
    }
}

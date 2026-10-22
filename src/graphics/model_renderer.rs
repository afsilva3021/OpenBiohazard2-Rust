use std::{ffi::CString, mem};

use crate::fileio::{pld::StaticPldModel, rdt::RdtCamera};

const VERTEX_SHADER_SOURCE: &str = r#"
attribute vec3 a_position;
uniform mat4 u_mvp;

void main() {
    gl_Position = u_mvp * vec4(a_position, 1.0);
}
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
precision mediump float;
uniform vec4 u_color;

void main() {
    gl_FragColor = u_color;
}
"#;

const MODEL_COLOR: [f32; 4] = [1.0, 0.12, 0.78, 1.0];
const CAMERA_NEAR: f32 = 16.0;
const CAMERA_FAR: f32 = 45_000.0;

pub struct ModelOverlay {
    pub model: StaticPldModel,
    pub position: [f32; 3],
    pub yaw_degrees: f32,
}

pub struct ModelRenderer {
    program: gl::types::GLuint,
    vertex_buffer: gl::types::GLuint,
    position_attribute: gl::types::GLuint,
    mvp_uniform: gl::types::GLint,
    color_uniform: gl::types::GLint,
    vertex_count: gl::types::GLsizei,
    model_matrix: Mat4,
    mvp: Mat4,
}

impl ModelRenderer {
    pub fn new(
        overlay: ModelOverlay,
        camera: &RdtCamera,
        projection_aspect: f32,
        screen_fit: [f32; 2],
        depth_bits: u8,
    ) -> Result<Self, String> {
        if overlay.model.positions.is_empty() {
            return Err("PLD model has no renderable triangle vertices".to_owned());
        }
        let vertex_count = gl::types::GLsizei::try_from(overlay.model.positions.len())
            .map_err(|_| "PLD vertex count exceeds OpenGL ES limits".to_owned())?;
        let buffer_bytes = overlay
            .model
            .positions
            .len()
            .checked_mul(mem::size_of::<[f32; 3]>())
            .filter(|&size| size <= isize::MAX as usize)
            .ok_or_else(|| "PLD vertex buffer size exceeds OpenGL ES limits".to_owned())?;
        let model_matrix = multiply(
            translation(overlay.position),
            rotation_y(overlay.yaw_degrees.to_radians()),
        );
        let mvp = camera_mvp(camera, projection_aspect, screen_fit, model_matrix)?;

        if depth_bits < 16 {
            return Err(format!(
                "3D model rendering needs a depth buffer of at least 16 bits; context reports {depth_bits}"
            ));
        }

        let ProgramBindings {
            program,
            position_attribute,
            mvp_uniform,
            color_uniform,
        } = create_model_program()?;
        let mut vertex_buffer = 0;
        unsafe { gl::GenBuffers(1, &mut vertex_buffer) };
        if vertex_buffer == 0 {
            unsafe { gl::DeleteProgram(program) };
            return Err("glGenBuffers returned buffer 0 for PLD model".to_owned());
        }

        unsafe {
            gl::BindBuffer(gl::ARRAY_BUFFER, vertex_buffer);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                buffer_bytes as gl::types::GLsizeiptr,
                overlay.model.positions.as_ptr().cast(),
                gl::STATIC_DRAW,
            );
        }
        let upload_error = unsafe { gl::GetError() };
        if upload_error != gl::NO_ERROR {
            unsafe {
                gl::DeleteBuffers(1, &vertex_buffer);
                gl::DeleteProgram(program);
            }
            return Err(format!(
                "OpenGL ES failed to upload PLD vertex buffer (error 0x{upload_error:04x})"
            ));
        }

        println!("SDL/OpenGL depth buffer: {depth_bits} bits");

        Ok(Self {
            program,
            vertex_buffer,
            position_attribute: position_attribute as gl::types::GLuint,
            mvp_uniform,
            color_uniform,
            vertex_count,
            model_matrix,
            mvp,
        })
    }

    pub fn prepare_camera(
        &self,
        camera: &RdtCamera,
        projection_aspect: f32,
        screen_fit: [f32; 2],
    ) -> Result<[f32; 16], String> {
        camera_mvp(camera, projection_aspect, screen_fit, self.model_matrix)
    }

    pub fn update_camera(
        &mut self,
        camera: &RdtCamera,
        projection_aspect: f32,
        screen_fit: [f32; 2],
    ) -> Result<(), String> {
        self.mvp = self.prepare_camera(camera, projection_aspect, screen_fit)?;
        Ok(())
    }

    pub fn set_mvp(&mut self, mvp: [f32; 16]) {
        self.mvp = mvp;
    }

    pub fn render(&self) {
        unsafe {
            gl::Clear(gl::DEPTH_BUFFER_BIT);
            gl::Enable(gl::DEPTH_TEST);
            gl::DepthFunc(gl::LESS);
            gl::DepthMask(gl::TRUE);
            gl::UseProgram(self.program);
            gl::UniformMatrix4fv(self.mvp_uniform, 1, gl::FALSE, self.mvp.as_ptr());
            gl::Uniform4f(
                self.color_uniform,
                MODEL_COLOR[0],
                MODEL_COLOR[1],
                MODEL_COLOR[2],
                MODEL_COLOR[3],
            );
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vertex_buffer);
            gl::EnableVertexAttribArray(self.position_attribute);
            gl::VertexAttribPointer(
                self.position_attribute,
                3,
                gl::FLOAT,
                gl::FALSE,
                (3 * mem::size_of::<f32>()) as gl::types::GLsizei,
                std::ptr::null(),
            );
            gl::DrawArrays(gl::TRIANGLES, 0, self.vertex_count);
            gl::DisableVertexAttribArray(self.position_attribute);
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

fn camera_mvp(
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

type Mat4 = [f32; 16];

fn identity() -> Mat4 {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn multiply(left: Mat4, right: Mat4) -> Mat4 {
    let mut result = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            for k in 0..4 {
                result[column * 4 + row] += left[k * 4 + row] * right[column * 4 + k];
            }
        }
    }
    result
}

fn translation(position: [f32; 3]) -> Mat4 {
    let mut result = identity();
    result[12] = position[0];
    result[13] = position[1];
    result[14] = position[2];
    result
}

fn rotation_y(angle: f32) -> Mat4 {
    let (sin, cos) = angle.sin_cos();
    [
        cos, 0.0, -sin, 0.0, 0.0, 1.0, 0.0, 0.0, sin, 0.0, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn scale(x: f32, y: f32, z: f32) -> Mat4 {
    [
        x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Result<Mat4, String> {
    let forward = normalize(subtract(target, eye), "RDT camera direction")?;
    let side = normalize(cross(forward, up), "RDT camera up direction")?;
    let corrected_up = cross(side, forward);
    Ok([
        side[0],
        corrected_up[0],
        -forward[0],
        0.0,
        side[1],
        corrected_up[1],
        -forward[1],
        0.0,
        side[2],
        corrected_up[2],
        -forward[2],
        0.0,
        -dot(side, eye),
        -dot(corrected_up, eye),
        dot(forward, eye),
        1.0,
    ])
}

fn perspective(fov_radians: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let focal = 1.0 / (fov_radians * 0.5).tan();
    let mut result = [0.0; 16];
    result[0] = focal / aspect;
    result[5] = focal;
    result[10] = (far + near) / (near - far);
    result[11] = -1.0;
    result[14] = (2.0 * far * near) / (near - far);
    result
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize(vector: [f32; 3], label: &str) -> Result<[f32; 3], String> {
    let length = dot(vector, vector).sqrt();
    if !length.is_finite() || length <= f32::EPSILON {
        return Err(format!("cannot build view matrix: {label} is degenerate"));
    }
    Ok([vector[0] / length, vector[1] / length, vector[2] / length])
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
    unsafe {
        gl::AttachShader(program, vertex_shader);
        gl::AttachShader(program, fragment_shader);
        gl::BindAttribLocation(program, 0, position_name.as_ptr());
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

    let mvp_name = CString::new("u_mvp").map_err(|error| error.to_string())?;
    let color_name = CString::new("u_color").map_err(|error| error.to_string())?;
    let bindings = ProgramBindings {
        program,
        position_attribute: unsafe { gl::GetAttribLocation(program, position_name.as_ptr()) },
        mvp_uniform: unsafe { gl::GetUniformLocation(program, mvp_name.as_ptr()) },
        color_uniform: unsafe { gl::GetUniformLocation(program, color_name.as_ptr()) },
    };
    if bindings.position_attribute < 0 || bindings.mvp_uniform < 0 || bindings.color_uniform < 0 {
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
    mvp_uniform: gl::types::GLint,
    color_uniform: gl::types::GLint,
}

#[cfg(test)]
mod tests {
    use super::{camera_mvp, identity, look_at, multiply, perspective, rotation_y, translation};
    use crate::fileio::rdt::RdtCamera;

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

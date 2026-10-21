use std::{ffi::CString, mem, ptr};

use super::texture::Texture;

const VERTEX_SHADER_SOURCE: &str = r#"
attribute vec2 a_position;
attribute vec2 a_uv;
uniform vec2 u_scale;
varying vec2 v_uv;

void main() {
    gl_Position = vec4(a_position * u_scale, 0.0, 1.0);
    v_uv = a_uv;
}
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
precision mediump float;
uniform sampler2D u_texture;
varying vec2 v_uv;

void main() {
    gl_FragColor = texture2D(u_texture, v_uv);
}
"#;

pub struct Renderer {
    program: gl::types::GLuint,
    vertex_buffer: gl::types::GLuint,
    position_attribute: gl::types::GLuint,
    uv_attribute: gl::types::GLuint,
    scale_uniform: gl::types::GLint,
    sampler_uniform: gl::types::GLint,
    texture: Texture,
    image_aspect: f32,
    scale: [f32; 2],
}

impl Renderer {
    pub fn new(
        viewport_width: u32,
        viewport_height: u32,
        texture_path: &str,
    ) -> Result<Self, String> {
        let texture = Texture::load_png(texture_path)?;
        let image_aspect = texture.width() as f32 / texture.height() as f32;
        let ProgramBindings {
            program,
            position_attribute,
            uv_attribute,
            scale_uniform,
            sampler_uniform,
        } = create_program()?;
        let position_attribute = position_attribute as gl::types::GLuint;
        let uv_attribute = uv_attribute as gl::types::GLuint;

        let mut vertex_buffer = 0;
        unsafe { gl::GenBuffers(1, &mut vertex_buffer) };
        if vertex_buffer == 0 {
            unsafe { gl::DeleteProgram(program) };
            return Err("glGenBuffers returned buffer 0".to_owned());
        }

        // PNG rows start at the top; OpenGL treats the first uploaded row as the
        // bottom row, so the V coordinates are inverted to display it upright.
        let vertices: [f32; 24] = [
            // x, y, u, v
            -1.0, 1.0, 0.0, 0.0, // top left
            -1.0, -1.0, 0.0, 1.0, // bottom left
            1.0, 1.0, 1.0, 0.0, // top right
            1.0, 1.0, 1.0, 0.0, // top right
            -1.0, -1.0, 0.0, 1.0, // bottom left
            1.0, -1.0, 1.0, 1.0, // bottom right
        ];
        let stride = (4 * mem::size_of::<f32>()) as gl::types::GLsizei;

        unsafe {
            gl::BindBuffer(gl::ARRAY_BUFFER, vertex_buffer);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * mem::size_of::<f32>()) as gl::types::GLsizeiptr,
                vertices.as_ptr().cast(),
                gl::STATIC_DRAW,
            );

            gl::EnableVertexAttribArray(position_attribute);
            gl::VertexAttribPointer(
                position_attribute,
                2,
                gl::FLOAT,
                gl::FALSE,
                stride,
                ptr::null(),
            );
            gl::EnableVertexAttribArray(uv_attribute);
            gl::VertexAttribPointer(
                uv_attribute,
                2,
                gl::FLOAT,
                gl::FALSE,
                stride,
                (2 * mem::size_of::<f32>()) as *const _,
            );

            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
        }

        let mut renderer = Self {
            program,
            vertex_buffer,
            position_attribute,
            uv_attribute,
            scale_uniform,
            sampler_uniform,
            texture,
            image_aspect,
            scale: [1.0, 1.0],
        };
        renderer.resize(viewport_width, viewport_height);
        Ok(renderer)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        let viewport_aspect = width as f32 / height as f32;
        self.scale = if self.image_aspect >= viewport_aspect {
            [0.9, 0.9 * viewport_aspect / self.image_aspect]
        } else {
            [0.9 * self.image_aspect / viewport_aspect, 0.9]
        };

        unsafe { gl::Viewport(0, 0, width as i32, height as i32) };
    }

    pub fn render(&self) {
        unsafe {
            gl::ClearColor(0.02, 0.04, 0.08, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);

            gl::UseProgram(self.program);
            gl::Uniform2f(self.scale_uniform, self.scale[0], self.scale[1]);
            gl::Uniform1i(self.sampler_uniform, 0);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, self.texture.id());
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vertex_buffer);
            gl::DrawArrays(gl::TRIANGLES, 0, 6);
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            gl::DisableVertexAttribArray(self.position_attribute);
            gl::DisableVertexAttribArray(self.uv_attribute);
            gl::DeleteBuffers(1, &self.vertex_buffer);
            gl::DeleteProgram(self.program);
        }
    }
}

fn create_program() -> Result<ProgramBindings, String> {
    let vertex_shader = compile_shader(gl::VERTEX_SHADER, "vertex", VERTEX_SHADER_SOURCE)?;
    let fragment_shader =
        match compile_shader(gl::FRAGMENT_SHADER, "fragment", FRAGMENT_SHADER_SOURCE) {
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
        return Err("glCreateProgram returned program 0".to_owned());
    }

    let position_name = CString::new("a_position").expect("attribute name has no NUL byte");
    let uv_name = CString::new("a_uv").expect("attribute name has no NUL byte");
    let scale_name = CString::new("u_scale").expect("uniform name has no NUL byte");
    let sampler_name = CString::new("u_texture").expect("uniform name has no NUL byte");
    unsafe {
        gl::AttachShader(program, vertex_shader);
        gl::AttachShader(program, fragment_shader);
        gl::BindAttribLocation(program, 0, position_name.as_ptr());
        gl::BindAttribLocation(program, 1, uv_name.as_ptr());
        gl::LinkProgram(program);
    }

    let mut link_status = gl::FALSE as gl::types::GLint;
    unsafe { gl::GetProgramiv(program, gl::LINK_STATUS, &mut link_status) };

    unsafe {
        gl::DeleteShader(vertex_shader);
        gl::DeleteShader(fragment_shader);
    }

    if link_status != gl::TRUE as gl::types::GLint {
        let log = program_info_log(program);
        eprintln!("[shader] program link failed:\n{log}");
        unsafe { gl::DeleteProgram(program) };
        return Err(format!("shader program link failed: {log}"));
    }

    let bindings = ProgramBindings {
        program,
        position_attribute: unsafe { gl::GetAttribLocation(program, position_name.as_ptr()) },
        uv_attribute: unsafe { gl::GetAttribLocation(program, uv_name.as_ptr()) },
        scale_uniform: unsafe { gl::GetUniformLocation(program, scale_name.as_ptr()) },
        sampler_uniform: unsafe { gl::GetUniformLocation(program, sampler_name.as_ptr()) },
    };
    if bindings.position_attribute < 0
        || bindings.uv_attribute < 0
        || bindings.scale_uniform < 0
        || bindings.sampler_uniform < 0
    {
        unsafe { gl::DeleteProgram(program) };
        return Err("linked shader program is missing a required attribute or uniform".to_owned());
    }

    Ok(bindings)
}

struct ProgramBindings {
    program: gl::types::GLuint,
    position_attribute: gl::types::GLint,
    uv_attribute: gl::types::GLint,
    scale_uniform: gl::types::GLint,
    sampler_uniform: gl::types::GLint,
}

fn compile_shader(
    shader_type: gl::types::GLenum,
    label: &str,
    source: &str,
) -> Result<gl::types::GLuint, String> {
    let source = CString::new(source)
        .map_err(|error| format!("{label} shader source contains a NUL byte: {error}"))?;
    let shader = unsafe { gl::CreateShader(shader_type) };
    if shader == 0 {
        return Err(format!("glCreateShader returned 0 for the {label} shader"));
    }

    let source_pointer = source.as_ptr();
    unsafe {
        gl::ShaderSource(shader, 1, &source_pointer, ptr::null());
        gl::CompileShader(shader);
    }

    let mut compile_status = gl::FALSE as gl::types::GLint;
    unsafe { gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut compile_status) };
    if compile_status != gl::TRUE as gl::types::GLint {
        let log = shader_info_log(shader);
        eprintln!("[shader] {label} shader compilation failed:\n{log}");
        unsafe { gl::DeleteShader(shader) };
        return Err(format!("{label} shader compilation failed: {log}"));
    }

    Ok(shader)
}

fn shader_info_log(shader: gl::types::GLuint) -> String {
    let mut log_length = 0;
    unsafe { gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut log_length) };
    if log_length <= 1 {
        return "driver returned no shader log".to_owned();
    }

    let mut log = vec![0_u8; log_length as usize];
    let mut written = 0;
    unsafe { gl::GetShaderInfoLog(shader, log_length, &mut written, log.as_mut_ptr().cast()) };
    let written = written.clamp(0, log_length) as usize;
    String::from_utf8_lossy(&log[..written])
        .trim_end_matches('\0')
        .to_owned()
}

fn program_info_log(program: gl::types::GLuint) -> String {
    let mut log_length = 0;
    unsafe { gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut log_length) };
    if log_length <= 1 {
        return "driver returned no program log".to_owned();
    }

    let mut log = vec![0_u8; log_length as usize];
    let mut written = 0;
    unsafe { gl::GetProgramInfoLog(program, log_length, &mut written, log.as_mut_ptr().cast()) };
    let written = written.clamp(0, log_length) as usize;
    String::from_utf8_lossy(&log[..written])
        .trim_end_matches('\0')
        .to_owned()
}

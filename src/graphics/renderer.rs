use std::{ffi::CString, mem, ptr};

const VERTEX_SHADER_SOURCE: &str = r#"
attribute vec2 a_position;
attribute vec3 a_color;
varying vec3 v_color;

void main() {
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_color = a_color;
}
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
precision mediump float;
varying vec3 v_color;

void main() {
    gl_FragColor = vec4(v_color, 1.0);
}
"#;

pub struct Renderer {
    program: gl::types::GLuint,
    vertex_buffer: gl::types::GLuint,
    position_attribute: gl::types::GLuint,
    color_attribute: gl::types::GLuint,
}

impl Renderer {
    pub fn new(viewport_width: u32, viewport_height: u32) -> Result<Self, String> {
        let (program, position_attribute, color_attribute) = create_program()?;
        let mut vertex_buffer = 0;

        unsafe {
            gl::GenBuffers(1, &mut vertex_buffer);
        }
        if vertex_buffer == 0 {
            unsafe { gl::DeleteProgram(program) };
            return Err("glGenBuffers returned buffer 0".to_owned());
        }

        let vertices: [f32; 15] = [
            // position  x, y       color  r, g, b
            0.0, 0.65, 1.0, 0.2, 0.2, -0.65, -0.5, 0.2, 1.0, 0.2, 0.65, -0.5, 0.2, 0.4, 1.0,
        ];
        let stride = (5 * mem::size_of::<f32>()) as gl::types::GLsizei;

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
            gl::EnableVertexAttribArray(color_attribute);
            gl::VertexAttribPointer(
                color_attribute,
                3,
                gl::FLOAT,
                gl::FALSE,
                stride,
                (2 * mem::size_of::<f32>()) as *const _,
            );
        }

        let mut renderer = Self {
            program,
            vertex_buffer,
            position_attribute,
            color_attribute,
        };
        renderer.resize(viewport_width, viewport_height);
        Ok(renderer)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        unsafe {
            gl::Viewport(0, 0, width.max(1) as i32, height.max(1) as i32);
        }
    }

    pub fn render(&self) {
        unsafe {
            gl::ClearColor(0.02, 0.04, 0.08, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);

            gl::UseProgram(self.program);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vertex_buffer);
            gl::EnableVertexAttribArray(self.position_attribute);
            gl::EnableVertexAttribArray(self.color_attribute);
            gl::DrawArrays(gl::TRIANGLES, 0, 3);
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            gl::DisableVertexAttribArray(self.position_attribute);
            gl::DisableVertexAttribArray(self.color_attribute);
            gl::DeleteBuffers(1, &self.vertex_buffer);
            gl::DeleteProgram(self.program);
        }
    }
}

fn create_program() -> Result<(gl::types::GLuint, gl::types::GLuint, gl::types::GLuint), String> {
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
    let color_name = CString::new("a_color").expect("attribute name has no NUL byte");
    unsafe {
        gl::AttachShader(program, vertex_shader);
        gl::AttachShader(program, fragment_shader);
        gl::BindAttribLocation(program, 0, position_name.as_ptr());
        gl::BindAttribLocation(program, 1, color_name.as_ptr());
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

    let position_attribute = unsafe { gl::GetAttribLocation(program, position_name.as_ptr()) };
    let color_attribute = unsafe { gl::GetAttribLocation(program, color_name.as_ptr()) };
    if position_attribute < 0 || color_attribute < 0 {
        unsafe { gl::DeleteProgram(program) };
        return Err("linked shader program is missing a required vertex attribute".to_owned());
    }

    Ok((
        program,
        position_attribute as gl::types::GLuint,
        color_attribute as gl::types::GLuint,
    ))
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
    unsafe {
        gl::GetShaderInfoLog(shader, log_length, &mut written, log.as_mut_ptr().cast());
    }
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
    unsafe {
        gl::GetProgramInfoLog(program, log_length, &mut written, log.as_mut_ptr().cast());
    }
    let written = written.clamp(0, log_length) as usize;
    String::from_utf8_lossy(&log[..written])
        .trim_end_matches('\0')
        .to_owned()
}

use std::{error::Error, ffi::CStr, io};

use sdl2::{
    event::{Event, WindowEvent},
    keyboard::Keycode,
    video::GLProfile,
};

use crate::graphics::renderer::Renderer;

const WINDOW_WIDTH: u32 = 640;
const WINDOW_HEIGHT: u32 = 480;

type AppResult<T> = Result<T, Box<dyn Error>>;

pub fn run() -> AppResult<()> {
    let sdl = sdl2::init().map_err(|error| sdl_error("initialize SDL2", error))?;
    let video = sdl
        .video()
        .map_err(|error| sdl_error("initialize SDL2 video", error))?;

    configure_opengles(&video);

    let window = video
        .window("OpenBiohazard2-Rust", WINDOW_WIDTH, WINDOW_HEIGHT)
        .position_centered()
        .resizable()
        .opengl()
        .build()
        .map_err(|error| sdl_error("create the OpenGL ES window", error.to_string()))?;

    let gl_context = window
        .gl_create_context()
        .map_err(|error| sdl_error("create the OpenGL ES context", error))?;
    window
        .gl_make_current(&gl_context)
        .map_err(|error| sdl_error("make the OpenGL ES context current", error))?;

    gl::load_with(|name| video.gl_get_proc_address(name) as *const _);
    print_graphics_information(&video);

    let (drawable_width, drawable_height) = window.drawable_size();
    let mut renderer = Renderer::new(drawable_width, drawable_height);
    let mut events = sdl
        .event_pump()
        .map_err(|error| sdl_error("create the SDL2 event pump", error))?;

    'main_loop: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    repeat: false,
                    ..
                } => break 'main_loop,
                Event::Window {
                    win_event: WindowEvent::Resized(width, height),
                    ..
                }
                | Event::Window {
                    win_event: WindowEvent::SizeChanged(width, height),
                    ..
                } => renderer.resize(width.max(1) as u32, height.max(1) as u32),
                _ => {}
            }
        }

        renderer.render();
        window.gl_swap_window();
    }

    drop(renderer);
    drop(gl_context);
    Ok(())
}

fn configure_opengles(video: &sdl2::VideoSubsystem) {
    let gl_attributes = video.gl_attr();
    gl_attributes.set_accelerated_visual(true);
    gl_attributes.set_context_profile(GLProfile::GLES);
    gl_attributes.set_context_version(2, 0);
    gl_attributes.set_double_buffer(true);
}

fn print_graphics_information(video: &sdl2::VideoSubsystem) {
    let sdl_driver = video.current_video_driver();
    println!("SDL video driver: {sdl_driver}");
    println!(
        "OpenGL ES vendor: {}",
        gl_string(gl::VENDOR).as_deref().unwrap_or("unknown")
    );
    println!(
        "OpenGL ES renderer: {}",
        gl_string(gl::RENDERER).as_deref().unwrap_or("unknown")
    );
    println!(
        "OpenGL ES version: {}",
        gl_string(gl::VERSION).as_deref().unwrap_or("unknown")
    );
}

fn gl_string(name: u32) -> Option<String> {
    // The pointer returned by glGetString is owned by the active GL context.
    // It is copied immediately so it remains valid for the printed value.
    let value = unsafe { gl::GetString(name) };
    if value.is_null() {
        return None;
    }

    unsafe { CStr::from_ptr(value.cast()) }
        .to_str()
        .ok()
        .map(ToOwned::to_owned)
}

fn sdl_error(operation: &str, error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("failed to {operation}: {error}"))
}

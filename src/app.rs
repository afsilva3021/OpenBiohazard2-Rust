use std::{error::Error, ffi::CStr, io, path::PathBuf};

use sdl2::{
    event::{Event, WindowEvent},
    keyboard::Keycode,
    video::{GLProfile, SwapInterval},
};

use crate::graphics::renderer::Renderer;

const WINDOW_WIDTH: u32 = 640;
const WINDOW_HEIGHT: u32 = 480;

type AppResult<T> = Result<T, Box<dyn Error>>;

pub enum TextureSource {
    Png(PathBuf),
    Rgba {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        description: String,
    },
}

pub fn run(source: TextureSource) -> AppResult<()> {
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
    configure_vsync(&video);

    gl::load_with(|name| video.gl_get_proc_address(name) as *const _);
    print_graphics_information(&video);

    let (drawable_width, drawable_height) = window.drawable_size();
    let mut renderer = match source {
        TextureSource::Png(path) => Renderer::new(drawable_width, drawable_height, path)
            .map_err(|error| sdl_error("load the PNG texture", error))?,
        TextureSource::Rgba {
            width,
            height,
            pixels,
            description,
        } => Renderer::from_rgba(
            drawable_width,
            drawable_height,
            width,
            height,
            pixels,
            &description,
        )
        .map_err(|error| sdl_error("load the ADT texture", error))?,
    };
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
                    win_event: WindowEvent::Resized(..) | WindowEvent::SizeChanged(..),
                    ..
                } => {
                    let (drawable_width, drawable_height) = window.drawable_size();
                    renderer.resize(drawable_width, drawable_height);
                }
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

fn configure_vsync(video: &sdl2::VideoSubsystem) {
    match video.gl_set_swap_interval(SwapInterval::VSync) {
        Ok(()) => println!("SDL swap interval: VSync enabled"),
        Err(error) => eprintln!("SDL could not enable VSync; continuing without it: {error}"),
    }
}

fn print_graphics_information(video: &sdl2::VideoSubsystem) {
    let sdl_driver = video.current_video_driver();
    let vendor = gl_string(gl::VENDOR);
    let renderer = gl_string(gl::RENDERER);

    println!("SDL video driver: {sdl_driver}");
    println!("GL_VENDOR: {}", vendor.as_deref().unwrap_or("unknown"));
    println!("GL_RENDERER: {}", renderer.as_deref().unwrap_or("unknown"));
    println!(
        "GL_VERSION: {}",
        gl_string(gl::VERSION).as_deref().unwrap_or("unknown")
    );
    println!(
        "GL_SHADING_LANGUAGE_VERSION: {}",
        gl_string(gl::SHADING_LANGUAGE_VERSION)
            .as_deref()
            .unwrap_or("unknown")
    );
    println!(
        "GPU acceleration (GL_RENDERER heuristic): {}",
        acceleration_status(renderer.as_deref())
    );
}

fn acceleration_status(renderer: Option<&str>) -> &'static str {
    let Some(renderer) = renderer else {
        return "undetermined (GL_RENDERER unavailable)";
    };
    let renderer = renderer.to_ascii_lowercase();

    if [
        "llvmpipe",
        "softpipe",
        "swrast",
        "software rasterizer",
        "swiftshader",
        "lavapipe",
    ]
    .iter()
    .any(|software_name| renderer.contains(software_name))
    {
        return "software renderer detected";
    }

    if [
        "vc4",
        "v3d",
        "videocore",
        "broadcom",
        "mali",
        "adreno",
        "powervr",
        "nvidia",
        "radeon",
        "amd",
        "intel",
        "apple gpu",
    ]
    .iter()
    .any(|hardware_name| renderer.contains(hardware_name))
    {
        return "hardware renderer identified";
    }

    "undetermined (check the GL_RENDERER driver name)"
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

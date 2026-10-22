use std::{error::Error, ffi::CStr, io, path::PathBuf};

use sdl2::{
    event::{Event, WindowEvent},
    keyboard::Keycode,
    video::{GLProfile, SwapInterval},
};

use crate::fileio::rdt::{RdtCameraImage, RdtCameraSession};
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
    RdtCamera {
        session: RdtCameraSession,
        initial: RdtCameraImage,
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
    let mut active_camera = None;
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
        TextureSource::RdtCamera { session, initial } => {
            let camera_index = initial.camera_index;
            let description = format!(
                "{} camera {camera_index} -> {} entry {}",
                session.rdt().path().display(),
                session.archive_path().display(),
                initial.image_index
            );
            let renderer = Renderer::from_rgba(
                drawable_width,
                drawable_height,
                initial.image.width(),
                initial.image.height(),
                initial.image.to_rgba(),
                &description,
            )
            .map_err(|error| sdl_error("load the initial RDT camera texture", error))?;
            active_camera = Some((session, camera_index));
            renderer
        }
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
                Event::KeyDown {
                    keycode: Some(Keycode::Left),
                    repeat: false,
                    ..
                } => {
                    if let Some(active_camera) = active_camera.as_mut() {
                        switch_camera(active_camera, &mut renderer, CameraDirection::Previous);
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Right),
                    repeat: false,
                    ..
                } => {
                    if let Some(active_camera) = active_camera.as_mut() {
                        switch_camera(active_camera, &mut renderer, CameraDirection::Next);
                    }
                }
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

#[derive(Clone, Copy)]
enum CameraDirection {
    Previous,
    Next,
}

fn adjacent_camera_index(
    current: usize,
    count: usize,
    direction: CameraDirection,
) -> Option<usize> {
    if count == 0 || current >= count {
        return None;
    }
    match direction {
        CameraDirection::Previous => Some(if current == 0 { count - 1 } else { current - 1 }),
        CameraDirection::Next => Some(if current + 1 == count { 0 } else { current + 1 }),
    }
}

fn switch_camera(
    active_camera: &mut (RdtCameraSession, usize),
    renderer: &mut Renderer,
    direction: CameraDirection,
) {
    let (session, current_camera) = active_camera;
    let Some(next_camera) =
        adjacent_camera_index(*current_camera, session.camera_count(), direction)
    else {
        eprintln!("Cannot switch camera: the RDT has no valid camera selection");
        return;
    };
    if next_camera == *current_camera {
        return;
    }

    let result = (|| {
        let selection = session.load_camera(next_camera)?;
        let camera = session.rdt().camera(next_camera)?;
        let description = format!(
            "{} camera {next_camera} -> {} entry {}",
            session.rdt().path().display(),
            session.archive_path().display(),
            selection.image_index
        );
        renderer.replace_texture(
            selection.image.width(),
            selection.image.height(),
            selection.image.to_rgba(),
            &description,
        )?;
        Ok::<_, String>((selection, *camera))
    })();

    match result {
        Ok((selection, camera)) => {
            *current_camera = selection.camera_index;
            println!(
                "Camera {}/{} active: roomcut entry {}, offset {}, {} compressed bytes; from={:?}, to={:?}, FOV={:.6}°, masks={}",
                selection.camera_index,
                session.camera_count(),
                selection.image_index,
                selection.entry.offset,
                selection.entry.length,
                camera.from,
                camera.to,
                camera.fov_degrees,
                camera
                    .mask_count
                    .map_or_else(|| "none".to_owned(), |count| count.to_string())
            );
        }
        Err(error) => eprintln!(
            "Could not switch to camera {next_camera}; keeping camera {} and its texture: {error}",
            *current_camera
        ),
    }
}

pub fn report_camera(session: &RdtCameraSession, selection: &RdtCameraImage) -> Result<(), String> {
    let camera = session.rdt().camera(selection.camera_index)?;
    println!(
        "Camera {}/{} selected: roomcut entry {}, offset {}, {} compressed bytes; flag=0x{:04x}, distance={}, from={:?}, to={:?}, FOV={:.6}°, masks={}",
        selection.camera_index,
        session.camera_count(),
        selection.image_index,
        selection.entry.offset,
        selection.entry.length,
        camera.flag,
        camera.distance_to_screen,
        camera.from,
        camera.to,
        camera.fov_degrees,
        camera
            .mask_count
            .map_or_else(|| "none".to_owned(), |count| count.to_string())
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{adjacent_camera_index, CameraDirection};

    #[test]
    fn cycles_through_nine_room_cameras_in_both_directions() {
        let mut current = 0;
        let mut visited = Vec::new();
        for _ in 0..9 {
            current = adjacent_camera_index(current, 9, CameraDirection::Next)
                .expect("valid next camera");
            visited.push(current);
        }
        assert_eq!(visited, [1, 2, 3, 4, 5, 6, 7, 8, 0]);
        assert_eq!(
            adjacent_camera_index(0, 9, CameraDirection::Previous),
            Some(8)
        );
    }

    #[test]
    fn respects_camera_counts_and_rejects_invalid_current_indices() {
        assert_eq!(adjacent_camera_index(1, 3, CameraDirection::Next), Some(2));
        assert_eq!(
            adjacent_camera_index(0, 1, CameraDirection::Previous),
            Some(0)
        );
        assert_eq!(adjacent_camera_index(0, 0, CameraDirection::Next), None);
        assert_eq!(adjacent_camera_index(3, 3, CameraDirection::Next), None);
    }
}

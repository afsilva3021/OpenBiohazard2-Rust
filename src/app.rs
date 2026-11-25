use std::{error::Error, ffi::CStr, io, path::PathBuf};

use sdl2::{
    controller::{Axis, Button, GameController},
    event::{Event, WindowEvent},
    keyboard::{Keycode, Scancode},
    video::{GLProfile, SwapInterval},
};

use crate::animation::{bone_transforms, Animation};
use crate::fileio::rdt::{RdtCameraImage, RdtCameraSession};
use crate::fileio::{adt::AdtImage, pld::PldModel};
use crate::graphics::mask_renderer::MaskRenderer;
use crate::graphics::model_renderer::{ModelRenderer, Placement};
use crate::graphics::renderer::Renderer;
use crate::math::Mat4;
use crate::world::{CameraSwitches, CollisionEntity, Player, PlayerInput};

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
    RdtCamera(Box<RdtCameraSource>),
}

pub struct ModelOverlay {
    pub model: PldModel,
    pub placement: Placement,
}

/// Room gameplay: player, SCA collision and RVD camera switches.
pub struct Gameplay {
    pub player: Player,
    pub collision: Vec<CollisionEntity>,
    pub switches: CameraSwitches,
}

/// Animated model state owned by the event loop.
struct ModelScene {
    overlay: ModelOverlay,
    renderer: ModelRenderer,
    masks: MaskRenderer,
    /// Background of the active camera, kept to rebuild masks on resize.
    background: AdtImage,
    animation: Animation,
    /// Pose shown: `None` is the neutral skeleton.
    pose: Option<usize>,
    bones: Vec<Mat4>,
    gameplay: Option<Gameplay>,
}

impl ModelScene {
    fn update(&mut self, elapsed_seconds: f64) {
        if let Some(gameplay) = self.gameplay.as_ref() {
            self.pose = gameplay.player.pose;
            self.overlay.placement = Placement {
                position: gameplay.player.position,
                yaw_degrees: gameplay.player.rotation_degrees,
            };
        }
        let model = &self.overlay.model;
        self.animation
            .update(self.pose, &model.animations, elapsed_seconds);
        let frame = self
            .animation
            .current_frame()
            .and_then(|index| model.frames.get(index));
        bone_transforms(&model.skeleton, frame, &mut self.bones);
    }

    /// Moves the player; returns the camera to switch to, if the player
    /// entered an RVD region (Go `HandleCameraSwitch`).
    fn step_player(&mut self, input: PlayerInput, seconds: f32, camera: usize) -> Option<usize> {
        let gameplay = self.gameplay.as_mut()?;
        gameplay.player.update(input, &gameplay.collision, seconds);
        gameplay
            .switches
            .region(gameplay.player.position, camera)
            .map(|region| usize::from(region.cam1))
    }

    /// Points the model and masks at a camera. Nothing changes on error.
    fn set_camera(
        &mut self,
        rdt: &crate::fileio::rdt::RdtFile,
        camera_index: usize,
        background: AdtImage,
        aspect: f32,
        fit: [f32; 2],
    ) -> Result<(), String> {
        let camera = rdt.camera(camera_index)?;
        let view_projection = self.renderer.prepare_camera(camera, aspect, fit)?;
        if let Err(error) = self.masks.update(
            &background,
            rdt.masks(camera_index),
            camera,
            &view_projection,
            fit,
        ) {
            eprintln!("Camera {camera_index}: masks disabled: {error}");
        }
        self.renderer.set_view_projection(view_projection);
        self.background = background;
        Ok(())
    }

    fn render(&self) {
        unsafe {
            gl::DepthMask(gl::TRUE);
            gl::Clear(gl::DEPTH_BUFFER_BIT);
        }
        self.masks.render();
        self.renderer.render(&self.overlay.placement, &self.bones);
    }
}

pub struct RdtCameraSource {
    pub session: RdtCameraSession,
    pub initial: RdtCameraImage,
    pub model: Option<ModelOverlay>,
    pub gameplay: Option<Gameplay>,
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
    let mut model_overlay = None;
    let mut initial_background = None;
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
        TextureSource::RdtCamera(source) => {
            let RdtCameraSource {
                session,
                initial,
                model,
                gameplay,
            } = *source;
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
            model_overlay = model.map(|model| (model, gameplay));
            initial_background = Some(initial.image);
            renderer
        }
    };
    let mut model_renderer = if let Some((overlay, gameplay)) = model_overlay {
        let Some((session, camera_index)) = active_camera.as_ref() else {
            return Err(sdl_error(
                "initialize PLD model",
                "model overlay requires an RDT camera",
            )
            .into());
        };
        let model = ModelRenderer::new(&overlay.model, video.gl_attr().depth_size())
            .map_err(|error| sdl_error("initialize the PLD model renderer", error))?;
        let masks = MaskRenderer::new()
            .map_err(|error| sdl_error("initialize the camera mask renderer", error))?;
        let background = initial_background
            .take()
            .ok_or_else(|| sdl_error("initialize camera masks", "missing initial background"))?;
        let mut scene = ModelScene {
            overlay,
            renderer: model,
            masks,
            background: background.clone(),
            animation: Animation::new(),
            pose: None,
            bones: Vec::new(),
            gameplay,
        };
        scene
            .set_camera(
                session.rdt(),
                *camera_index,
                background,
                renderer.image_aspect(),
                renderer.fit_scale(),
            )
            .map_err(|error| sdl_error("set the initial model camera", error))?;
        Some(scene)
    } else {
        None
    };
    let mut events = sdl
        .event_pump()
        .map_err(|error| sdl_error("create the SDL2 event pump", error))?;
    let timer = sdl
        .timer()
        .map_err(|error| sdl_error("initialize the SDL2 timer", error))?;
    let mut last_ticks = timer.ticks64();
    // Game controllers (Console Zero / USB pads). Missing support is not fatal.
    let controller_subsystem = sdl.game_controller().ok();
    let mut controllers: Vec<GameController> = Vec::new();
    let mut exit_hold_seconds = 0.0_f64;

    'main_loop: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    repeat: false,
                    ..
                } => break 'main_loop,
                Event::ControllerDeviceAdded { which, .. } => {
                    if let Some(subsystem) = controller_subsystem.as_ref() {
                        match subsystem.open(which) {
                            Ok(controller) => {
                                println!("Controller connected: {}", controller.name());
                                controllers.push(controller);
                            }
                            Err(error) => eprintln!("Could not open controller {which}: {error}"),
                        }
                    }
                }
                Event::ControllerDeviceRemoved { which, .. } => {
                    controllers.retain(|controller| controller.instance_id() != which);
                }
                Event::KeyDown {
                    keycode: Some(Keycode::PageUp),
                    repeat: false,
                    ..
                } => {
                    if let Some(active_camera) = active_camera.as_mut() {
                        switch_camera(
                            active_camera,
                            &mut renderer,
                            model_renderer.as_mut(),
                            CameraDirection::Previous,
                        );
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::PageDown),
                    repeat: false,
                    ..
                } => {
                    if let Some(active_camera) = active_camera.as_mut() {
                        switch_camera(
                            active_camera,
                            &mut renderer,
                            model_renderer.as_mut(),
                            CameraDirection::Next,
                        );
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Left),
                    repeat: false,
                    ..
                } if !playing(&model_renderer) => {
                    if let Some(active_camera) = active_camera.as_mut() {
                        switch_camera(
                            active_camera,
                            &mut renderer,
                            model_renderer.as_mut(),
                            CameraDirection::Previous,
                        );
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Right),
                    repeat: false,
                    ..
                } if !playing(&model_renderer) => {
                    if let Some(active_camera) = active_camera.as_mut() {
                        switch_camera(
                            active_camera,
                            &mut renderer,
                            model_renderer.as_mut(),
                            CameraDirection::Next,
                        );
                    }
                }
                Event::Window {
                    win_event: WindowEvent::Resized(..) | WindowEvent::SizeChanged(..),
                    ..
                } => {
                    let (drawable_width, drawable_height) = window.drawable_size();
                    renderer.resize(drawable_width, drawable_height);
                    if let (Some(scene), Some((session, camera_index))) =
                        (model_renderer.as_mut(), active_camera.as_ref())
                    {
                        let background = scene.background.clone();
                        if let Err(error) = scene.set_camera(
                            session.rdt(),
                            *camera_index,
                            background,
                            renderer.image_aspect(),
                            renderer.fit_scale(),
                        ) {
                            eprintln!("Could not update the 3D view after resize: {error}");
                        }
                    }
                }
                _ => {}
            }
        }

        let now = timer.ticks64();
        let elapsed_seconds = (now.saturating_sub(last_ticks)) as f64 / 1000.0;
        last_ticks = now;

        let (input, exit_combo) = read_input(&events, &controllers);
        exit_hold_seconds = if exit_combo {
            exit_hold_seconds + elapsed_seconds
        } else {
            0.0
        };
        if exit_hold_seconds >= 1.0 {
            break 'main_loop;
        }
        // Large gaps (loading a camera) must not teleport the player.
        let step = elapsed_seconds.min(0.1) as f32;
        if let (Some(scene), Some(active)) = (model_renderer.as_mut(), active_camera.as_mut()) {
            if let Some(target) = scene.step_player(input, step, active.1) {
                if target != active.1 && target < active.0.camera_count() {
                    change_camera(active, &mut renderer, Some(scene), target);
                }
            }
        }

        renderer.render();
        if let Some(scene) = model_renderer.as_mut() {
            scene.update(elapsed_seconds);
            scene.render();
        }
        window.gl_swap_window();
    }

    drop(model_renderer);
    drop(renderer);
    drop(gl_context);
    Ok(())
}

fn playing(scene: &Option<ModelScene>) -> bool {
    scene.as_ref().is_some_and(|scene| scene.gameplay.is_some())
}

/// Keyboard arrows / controller D-pad and left stick drive the tank
/// controls. START+SELECT held together reports the exit combination used
/// by the Console Zero launcher.
fn read_input(events: &sdl2::EventPump, controllers: &[GameController]) -> (PlayerInput, bool) {
    let keys = events.keyboard_state();
    let mut input = PlayerInput {
        forward: keys.is_scancode_pressed(Scancode::Up),
        backward: keys.is_scancode_pressed(Scancode::Down),
        left: keys.is_scancode_pressed(Scancode::Left),
        right: keys.is_scancode_pressed(Scancode::Right),
    };
    let mut exit_combo =
        keys.is_scancode_pressed(Scancode::Return) && keys.is_scancode_pressed(Scancode::Backspace);
    const DEAD_ZONE: i16 = 16_000;
    for pad in controllers {
        let x = pad.axis(Axis::LeftX);
        let y = pad.axis(Axis::LeftY);
        input.forward |= pad.button(Button::DPadUp) || y < -DEAD_ZONE;
        input.backward |= pad.button(Button::DPadDown) || y > DEAD_ZONE;
        input.left |= pad.button(Button::DPadLeft) || x < -DEAD_ZONE;
        input.right |= pad.button(Button::DPadRight) || x > DEAD_ZONE;
        exit_combo |= pad.button(Button::Start) && pad.button(Button::Back);
    }
    (input, exit_combo)
}

fn configure_opengles(video: &sdl2::VideoSubsystem) {
    let gl_attributes = video.gl_attr();
    gl_attributes.set_accelerated_visual(true);
    gl_attributes.set_context_profile(GLProfile::GLES);
    gl_attributes.set_context_version(2, 0);
    gl_attributes.set_double_buffer(true);
    gl_attributes.set_depth_size(16);
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
    scene: Option<&mut ModelScene>,
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
    change_camera(active_camera, renderer, scene, next_camera);
}

/// Loads and shows `next_camera`. On any error the previous camera, texture
/// and masks stay active.
fn change_camera(
    active_camera: &mut (RdtCameraSession, usize),
    renderer: &mut Renderer,
    scene: Option<&mut ModelScene>,
    next_camera: usize,
) {
    let (session, current_camera) = active_camera;
    let result = (|| {
        let selection = session.load_camera(next_camera)?;
        let camera = *session.rdt().camera(next_camera)?;
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
        Ok::<_, String>((selection, camera))
    })();
    let result = result.and_then(|(selection, camera)| {
        if let Some(scene) = scene {
            let aspect = selection.image.width() as f32 / selection.image.height() as f32;
            scene.set_camera(
                session.rdt(),
                selection.camera_index,
                selection.image.clone(),
                aspect,
                renderer.fit_scale(),
            )?;
        }
        Ok((selection, camera))
    });

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

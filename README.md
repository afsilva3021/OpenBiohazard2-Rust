# OpenBiohazard2-Rust

Native Rust migration work for [OpenBiohazard2](../OpenBiohazard2). The original Go project remains untouched and is the behavioral reference for later migration stages.

This first stage only creates a 640×480 SDL2 window with an OpenGL ES 2.0 context, initializes the viewport, clears the framebuffer with the GPU, swaps buffers, and exits on `Esc` or the window close event. It does not load game data, textures, shaders, models, or Resident Evil 2 files.

## Build on WSL

The project uses the Rust `sdl2` crate without the bundled SDL2 build. This keeps the platform library supplied by the operating system, which is useful on Raspberry Pi OS where SDL2 must expose the distro's KMSDRM/EGL support. The WSL build therefore needs the SDL2 development library and a native linker.

```bash
cd /home/alex/residentEvil2Arm/OpenBiohazard2-Rust
sudo apt update
sudo apt install build-essential libsdl2-dev

cargo fmt --check
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

Run it from the project directory so the SDL library can use the WSL GUI backend selected by the environment:

```bash
cargo run --release
```

The application prints the SDL video driver and the OpenGL ES vendor, renderer, and version after the context is created. `DISPLAY` or `WAYLAND_DISPLAY` must be available for a visible window. If the WSL session has no usable graphical backend, the compile checks still validate the code, while the graphical run is expected to fail during SDL window or context creation.

## Raspberry Pi Zero 2 W

The target is a 64-bit ARM Cortex-A53 system with 512 MB RAM and VideoCore IV OpenGL ES 1.1/2.0 support. Confirm the runtime architecture and the active SDL video drivers on the device before running:

```bash
uname -m
which cargo
SDL_VIDEODRIVER=dummy cargo run --release
```

The `dummy` command only checks process startup paths and cannot create the requested OpenGL ES window. For a console session with KMS/DRM, use the physical display and the `kmsdrm` backend:

```bash
sudo apt update
sudo apt install build-essential pkg-config libsdl2-dev libdrm-dev libgbm-dev

cd /home/alex/residentEvil2Arm/OpenBiohazard2-Rust
cargo build --release
SDL_VIDEODRIVER=kmsdrm ./target/release/openbiohazard2-rust
```

The process needs access to the DRM device and input devices. Run it locally on the Pi while diagnosing permissions, and check `ls -l /dev/dri/card* /dev/input/event*` before changing group membership. No Xorg installation is required for the KMSDRM path. The actual Pi runtime and ARM cross-compilation have not been validated in this workspace.

## Migration notes

See [docs/MIGRATION.md](docs/MIGRATION.md) for the Go code analysis, the Go-to-Rust module map, the OpenGL ES adaptation points, and the current validation report.

The original project is MIT licensed and includes the following attribution: Copyright (c) 2020 Samuel Yuan. Game data is proprietary and must be supplied by the user; it is not included in this repository.

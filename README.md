# OpenBiohazard2-Rust

Native Rust migration work for [OpenBiohazard2](../OpenBiohazard2). The original Go project remains untouched and is the behavioral reference for later migration stages.

The current stage creates a 640×480 SDL2 window with an OpenGL ES 2.0 context and displays `assets/test_texture.png` on a two-triangle rectangle using a VBO and GLSL ES 1.00 shaders. It keeps the blue background, preserves the image aspect ratio using the drawable framebuffer size, requests VSync when SDL supports it, exits on `Esc` or the window close event, and prints the GL vendor, renderer, API version, shading-language version, and a renderer-name-based hardware/software assessment. The PNG decoder produces RGBA pixels for a direct `glTexImage2D` upload; no game data or proprietary files are loaded.

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

Run it from the project directory so the application can find the PNG and SDL can use the WSL GUI backend selected by the environment:

```bash
cargo run --release
```

After the context is created, the application prints the SDL video driver and `GL_VENDOR`, `GL_RENDERER`, `GL_VERSION`, and `GL_SHADING_LANGUAGE_VERSION`. The acceleration label uses known renderer-name patterns: `llvmpipe`/`softpipe` indicate software, and names such as `VC4`, `V3D`, `VideoCore`, or `Broadcom` indicate hardware; unfamiliar names are reported as undetermined. `DISPLAY` or `WAYLAND_DISPLAY` must be available for a visible window. If the WSL session has no usable graphical backend, the compile checks still validate the code, while the graphical run may fail during SDL window or context creation.

## Raspberry Pi Zero 2 W

The Zero 2 W uses a quad-core ARM Cortex-A53 and VideoCore IV with OpenGL ES 1.1/2.0 support. Raspberry Pi OS may be 32-bit or 64-bit; compiling directly on the Pi selects the installed OS ABI. Confirm it and the active SDL video driver before running:

```bash
uname -m
which cargo
```

For a console session with KMS/DRM, use the physical display and the `kmsdrm` backend:

```bash
sudo apt update
sudo apt install build-essential pkg-config libsdl2-dev libdrm-dev libgbm-dev

cd ~/OpenBiohazard2-Rust
CARGO_BUILD_JOBS=1 cargo fmt --check
CARGO_BUILD_JOBS=1 cargo check
CARGO_BUILD_JOBS=1 cargo clippy --all-targets --all-features -- -D warnings
CARGO_BUILD_JOBS=1 cargo build --release
SDL_VIDEODRIVER=kmsdrm ./target/release/openbiohazard2-rust
```

Run the final command from the project directory; the test PNG is loaded from `assets/test_texture.png`.

## Verify the texture visually

The test image should appear centered in the blue 640×480 window, with its 16:9 shape intact. Its top half is red and green, its bottom half blue and yellow, and its transparent outer border reveals the blue clear color. Resizing the window should change the image size while keeping the same proportions. Press `Esc` to exit.

The process needs access to the DRM device and input devices. Run it locally on the Pi while diagnosing permissions, and check `ls -l /dev/dri/card* /dev/input/event*` before changing group membership. No Xorg installation is required for the KMSDRM path. On the Pi 0–3, Mesa's VC4 driver supports GLES2; its renderer name should identify VC4/VideoCore/Broadcom. The renderer classification is a string-based diagnostic, so inspect the printed `GL_RENDERER` value if it reports “undetermined”. This workspace has no Pi graphics device, so the textured rectangle and runtime driver strings still need to be confirmed on the Pi.

## Migration notes

See [docs/MIGRATION.md](docs/MIGRATION.md) for the Go code analysis, the Go-to-Rust module map, the OpenGL ES adaptation points, and the current validation report.

The original project is MIT licensed and includes the following attribution: Copyright (c) 2020 Samuel Yuan. Game data is proprietary and must be supplied by the user; it is not included in this repository.

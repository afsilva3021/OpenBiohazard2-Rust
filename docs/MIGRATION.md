# OpenBiohazard2 Go → Rust

## Scope of this document

The source of truth inspected for this first stage is the existing project at `../OpenBiohazard2`. Its directory was not modified, reorganized, or copied into the Rust project. The Rust code currently implements only the platform and graphics bootstrap requested for stage one.

## Current Go architecture

`main.go` is the application entry point. It validates the expected game-data directories through `resource.ValidateFilesExist`, locks the OS thread for GLFW/OpenGL, initializes GLFW, creates a `1024×768` window through `client.NewWindowHandler`, constructs the renderer and game state, then dispatches the state machine each frame.

The main runtime path is:

```text
main.go
├── resource.ValidateFilesExist()
├── client.WindowHandler
│   ├── GLFW window and desktop OpenGL 3.3 core context
│   └── keyboard/mouse input callbacks
├── render.InitRenderer()
│   ├── OpenGL function loading
│   ├── shader.ShaderSystem
│   ├── ViewSystem / SceneSystem
│   └── OpenGLRenderer
├── game.NewGame() and player setup
└── state handlers → RenderFrame()
```

The principal modules are:

| Go path | Responsibility | Planned Rust location | Migration status |
| --- | --- | --- | --- |
| `main.go` | Startup, game initialization, state dispatch | `src/main.rs`, `src/app.rs` | Bootstrap only |
| `client/windowhandler.go` | GLFW window, context, swap, resize, frame timing | `src/app.rs` plus a future `platform` module | SDL2 window and event loop implemented |
| `client/inputhandler.go` | Key and mouse action mapping | Future `src/input.rs` | Not started |
| `render/rendermain.go` | Renderer setup and frame orchestration | Future `src/graphics/renderer.rs` | Clear-only renderer implemented |
| `render/opengl_renderer.go` | VAO/VBO/attributes, textures, draw calls | Future graphics modules | Not started |
| `shader/shader.go` | Shader file loading, compilation, linking | Future `src/graphics/shader.rs` | Not started |
| `shader/shadersystem.go` | Program lifetime and uniform cache | Future `src/graphics/shader.rs` | Not started |
| `shader/openbiohazard2.vert` | Desktop GLSL vertex shader | Future GLES shader asset | Requires adaptation |
| `shader/openbiohazard2.frag` | Desktop GLSL fragment shader | Future GLES shader asset | Requires adaptation |
| `resource/` | Game-data paths, validation, image loading | Future `src/resource/` | Not started |
| `fileio/` | Binary formats such as RDT, TIM, MD1, EMD, and VAB | Future `src/fileio/` | Not started |
| `state/`, `game/`, `world/`, `ui/` | Game state, simulation, collision, menus | Future matching Rust modules | Not started |

The Go dependency configuration is in `go.mod`: Go 1.23, `go-gl/gl`, `go-gl/glfw`, and `go-gl/mathgl`. The Rust bootstrap deliberately uses SDL2 for the platform layer and the `gl` crate for runtime function loading. No Go dependency is executed from Rust.

## Window, renderer, and shader findings

The existing `client/windowhandler.go` requests OpenGL 3.3 core, forward-compatible, through GLFW. It swaps buffers before polling events and closes when the mapped Escape action is active. The Rust bootstrap changes the window size to the stage requirement of `640×480`, requests `GLProfile::GLES` version `2.0` through SDL2, polls SDL events, handles `Esc` and `Quit`, updates the viewport after resize, clears the color buffer, and swaps buffers.

The existing `render/rendermain.go` loads desktop OpenGL functions, sets a black clear color, enables depth testing, multisampling, and alpha blending, then initializes shaders and scene systems. Those features are intentionally outside this stage. The Rust renderer currently uses only `Viewport`, `ClearColor`, and `Clear`, all available in OpenGL ES 2.0.

The current Go shaders are not portable to the requested GLES 2.0 profile without changes. They declare `#version 410`, use desktop shader interfaces such as `layout` and `out`, and rely on operations such as matrix inversion. A later GLES shader pass must use the syntax and limits supported by GLSL ES 1.00, add the required precision declarations, replace unsupported desktop constructs, and verify compilation on the Pi driver. The Rust bootstrap does not load these shaders yet.

The Go renderer also assumes desktop OpenGL objects and functions. In particular, VAO use, shader interfaces, multisampling, and the complete texture/depth pipeline need an explicit GLES compatibility pass. VAOs may require an extension on GLES 2.0, so they must not be treated as available until the runtime extension string is checked or the renderer is written around VBO state without them.

## Platform decision

The Pi Zero 2 W hardware specification lists a 64-bit quad-core Cortex-A53 at 1 GHz, 512 MB of SDRAM, and OpenGL ES 1.1/2.0 graphics. The intended headless display path is SDL2 with KMSDRM/EGL, selected at runtime with `SDL_VIDEODRIVER=kmsdrm`. The device, its kernel, and its DRM permissions are not present in this WSL workspace, so ARM runtime compatibility remains unverified.

WSL exposes both `DISPLAY` and `WAYLAND_DISPLAY` in the current environment, but SDL2 development packages, `pkg-config`, and a C linker are not installed. The Rust project links against SDL2 supplied by the operating system rather than embedding a second SDL build. This keeps the platform backend explicit: WSL can use its Wayland/X11 integration, while Raspberry Pi OS can supply the SDL2 build that exposes KMSDRM/EGL. The Pi instructions document the required DRM/GBM development packages and require an actual device test.

## Stage-one file set

| File | Purpose |
| --- | --- |
| `Cargo.toml` | Rust package metadata and SDL2/OpenGL function-loading dependencies |
| `Cargo.lock` | Locked dependency versions generated by Cargo |
| `src/main.rs` | Process entry point and top-level error reporting |
| `src/app.rs` | SDL2 initialization, GLES 2.0 context, event loop, driver information, and buffer swaps |
| `src/graphics/mod.rs` | Graphics module boundary |
| `src/graphics/renderer.rs` | Viewport and clear-only GPU renderer |
| `README.md` | Build/run instructions, Pi notes, and stage boundary |
| `docs/MIGRATION.md` | Go analysis, module mapping, portability findings, and validation report |

No Resident Evil 2 game data, proprietary files, textures, models, triangles, or scenario loaders were added.

## Validation report

The commands below were run in the WSL workspace after the initial implementation:

| Command | Result |
| --- | --- |
| `cargo fmt --check` | Passed |
| `cargo check --offline` | Blocked before Rust type checking: the WSL image has no host linker (`cc`) or C runtime development files |
| `cargo clippy --offline --all-targets --all-features -- -D warnings` | Blocked by the same missing host linker before linting |
| `cargo build --release` | Blocked: the WSL image has no host linker (`cc`) or SDL2 development library |
| Graphical window run | Not reached because the binary could not be built |
| Raspberry Pi ARM build/runtime | Not run; no Pi or ARM toolchain is available here |

The graphical run is a separate validation from compilation. A WSL context creation failure must be reported as an environment limitation rather than treated as evidence that the Rust source fails to compile.

## Next migration step

After this stage is accepted, port the platform-independent input and timing boundary, then add a small GLES 2.0 shader compilation probe before translating the full renderer. The probe should verify shader language support and runtime extensions on both WSL and the Pi before any game assets are loaded.

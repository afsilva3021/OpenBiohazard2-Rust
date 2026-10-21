# OpenBiohazard2 Go → Rust


## Scope of this document

The source of truth inspected for the migration is the existing project at `../OpenBiohazard2`. Its directory was not modified, reorganized, or copied into the Rust project. The Rust implementation currently covers the SDL2/GLES 2.0 bootstrap and a PNG texture-rendering probe.

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
| `render/rendermain.go` | Renderer setup and frame orchestration | `src/graphics/renderer.rs` | Blue clear and textured test rectangle only |
| `render/opengl_renderer.go` | VAO/VBO/attributes, textures, draw calls | Future graphics modules | GLES2 VBO, UVs, PNG texture upload and two-triangle draw only |
| `shader/shader.go` | Shader file loading, compilation, linking | `src/graphics/renderer.rs` for this probe; future `src/graphics/shader.rs` | In-memory GLSL ES 1.00 texture shaders with compile/link error logs |
| `shader/shadersystem.go` | Program lifetime and uniform cache | Future `src/graphics/shader.rs` | Not started |
| `shader/openbiohazard2.vert` | Desktop GLSL vertex shader | Future GLES shader asset | Requires adaptation |
| `shader/openbiohazard2.frag` | Desktop GLSL fragment shader | Future GLES shader asset | Requires adaptation |
| `resource/` | Game-data paths, validation, image loading | Future `src/resource/` | Not started |
| `fileio/` | Binary formats such as RDT, TIM, MD1, EMD, and VAB | Future `src/fileio/` | Not started |
| `state/`, `game/`, `world/`, `ui/` | Game state, simulation, collision, menus | Future matching Rust modules | Not started |

The Go dependency configuration is in `go.mod`: Go 1.23, `go-gl/gl`, `go-gl/glfw`, and `go-gl/mathgl`. The Rust bootstrap deliberately uses SDL2 for the platform layer and the `gl` crate for runtime function loading. No Go dependency is executed from Rust.

## Window, renderer, and shader findings

The existing `client/windowhandler.go` requests OpenGL 3.3 core, forward-compatible, through GLFW. It swaps buffers before polling events and closes when the mapped Escape action is active. The Rust bootstrap changes the window size to the stage requirement of `640×480`, requests `GLProfile::GLES` version `2.0` through SDL2, attempts SDL VSync with a non-fatal warning if unavailable, polls SDL events, handles `Esc` and `Quit`, and updates the viewport from the drawable framebuffer dimensions after resize.

The existing `render/rendermain.go` loads desktop OpenGL functions, sets a black clear color, enables depth testing, multisampling, and alpha blending, then initializes shaders and scene systems. The Rust renderer retains its blue clear, decodes `assets/test_texture.png` to RGBA, uploads it with `glTexImage2D`, and draws a UV-mapped rectangle as six VBO vertices. The GLSL ES 1.00 fragment shader samples it with `texture2D`; filtering and clamp-to-edge parameters support non-power-of-two dimensions in GLES 2.0. The viewport-fit scale preserves the image aspect ratio. The PNG pixel buffer is passed directly to the GL upload and released after the call; the GPU texture is deleted when the renderer is dropped. No VAO or desktop-only API is used.

The current Go shaders are not portable to the requested GLES 2.0 profile without changes. They declare `#version 410`, use desktop shader interfaces such as `layout` and `out`, and rely on operations such as matrix inversion. The Rust texture probe uses GLSL ES 1.00 syntax (`attribute`, `varying`, `texture2D`, `gl_FragColor`) and a fragment precision declaration. It logs compile and link info logs on failure. The Go shaders remain unported.

The Go renderer also assumes desktop OpenGL objects and functions. In particular, VAO use, shader interfaces, multisampling, and the complete texture/depth pipeline need an explicit GLES compatibility pass. VAOs may require an extension on GLES 2.0, so they must not be treated as available until the runtime extension string is checked or the renderer is written around VBO state without them.

## Platform decision

The Pi Zero 2 W hardware specification lists a quad-core Cortex-A53 at 1 GHz, 512 MB of SDRAM, and OpenGL ES 1.1/2.0 graphics. The intended headless display path is SDL2 with KMSDRM/EGL, selected at runtime with `SDL_VIDEODRIVER=kmsdrm`. Mesa's VC4 driver supports GLES2 on Raspberry Pi 0–3. The application prints `GL_VENDOR`, `GL_RENDERER`, `GL_VERSION`, and `GL_SHADING_LANGUAGE_VERSION`. Acceleration reporting is a heuristic: known software strings (such as `llvmpipe`/`softpipe`) are labeled software, known GPU strings (such as `VC4`, `V3D`, or `VideoCore`) hardware, and other values undetermined.

WSL exposes both `DISPLAY` and `WAYLAND_DISPLAY` in the current environment, but SDL2 development packages, `pkg-config`, and a C linker are not installed. The Rust project links against SDL2 supplied by the operating system rather than embedding a second SDL build. This keeps the platform backend explicit: WSL can use its Wayland/X11 integration, while Raspberry Pi OS can supply the SDL2 build that exposes KMSDRM/EGL. The Pi instructions document the required DRM/GBM development packages and require an actual device test.

## Rust files for the current stage

| File | Purpose |
| --- | --- |
| `Cargo.toml` | Rust package metadata, SDL2/OpenGL function loading, and pure-Rust PNG decoding |
| `Cargo.lock` | Locked dependency versions generated by Cargo |
| `src/main.rs` | Process entry point and top-level error reporting |
| `src/app.rs` | SDL2 initialization, GLES 2.0 context, event loop, GL context details, and acceleration assessment |
| `src/graphics/mod.rs` | Graphics module boundary |
| `src/graphics/renderer.rs` | GLSL ES 1.00 compile/link, VBO/UV setup, aspect-preserving viewport fit, blue clear, and rectangle draw |
| `src/graphics/texture.rs` | PNG decode to RGBA, GLES2-compatible texture upload, texture lifetime |
| `assets/test_texture.png` | Non-proprietary 320×180 RGBA test image with transparency and orientation colors |
| `README.md` | Build/run instructions, Pi notes, and stage boundary |
| `docs/MIGRATION.md` | Go analysis, module mapping, portability findings, and validation report |

No Resident Evil 2 game data, proprietary files, models, or scenario loaders were added. The only image is a generated test pattern.

## Local and remote branch comparison

The fetched GitHub `main` was `927697e`; local `main` is `eb75276`, three commits ahead and zero behind. The fetched renderer draws three vertices and has no texture upload; the local commits contain the PNG decoder and textured rectangle. The local branch has no configured upstream. The pre-existing working-tree deletion of `ResidentEvil2-Data.zip` was left untouched.

## Validation report

The commands below were run in the WSL workspace after implementing PNG texturing:

| Command | Result |
| --- | --- |
| `cargo fmt` / `cargo fmt --check` | Passed |
| `cargo check --offline` | Blocked before project type checking: `cc` is not installed in this WSL image |
| `cargo clippy --offline --all-targets --all-features -- -D warnings` | Blocked before linting by the missing `cc` linker |
| `cargo build --release` | Blocked before project compilation because `cc` is missing; SDL2 development libraries are also absent in WSL |
| `cargo metadata --offline --format-version 1` / `cargo tree --offline` | Passed |
| `git diff --check` | Passed |
| WSL graphics run | Not reached; the binary could not be built in this workspace |
| Raspberry Pi build/runtime | Earlier stages were reported working by the user; VSync and framebuffer resize changes plus the textured rectangle still need a Pi run |

The graphical run is a separate validation from compilation. A WSL context creation failure must be reported as an environment limitation rather than treated as evidence that the Rust source fails to compile.

## Next migration step

Do not start other migration work until the PNG texture has been confirmed on the Pi.

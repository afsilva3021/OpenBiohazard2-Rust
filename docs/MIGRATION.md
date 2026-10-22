# OpenBiohazard2 Go → Rust


## Scope of this document

The source of truth inspected for the migration is the existing project at `../OpenBiohazard2`. Its directory was not modified, reorganized, or copied into the Rust project. The Rust implementation covers the SDL2/OpenGL ES 2.0 renderer, PNG test mode, ADT decompression and reconstruction, BIN entry lookup, PNG diagnostics, and direct ADT texture upload.

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
| `render/opengl_renderer.go` | VAO/VBO/attributes, textures, draw calls | Future graphics modules | GLES2 VBO, UVs, PNG/ADT texture upload and two-triangle draw only |
| `shader/shader.go` | Shader file loading, compilation, linking | `src/graphics/renderer.rs` for this probe; future `src/graphics/shader.rs` | In-memory GLSL ES 1.00 texture shaders with compile/link error logs |
| `shader/shadersystem.go` | Program lifetime and uniform cache | Future `src/graphics/shader.rs` | Not started |
| `shader/openbiohazard2.vert` | Desktop GLSL vertex shader | Future GLES shader asset | Requires adaptation |
| `shader/openbiohazard2.frag` | Desktop GLSL fragment shader | Future GLES shader asset | Requires adaptation |
| `resource/` | Game-data paths, validation, image loading | Future `src/resource/` | Not started |
| `fileio/adt.go` | ADT stream decompression, background layout, render words, PNG conversion | `src/fileio/adt.rs`, `src/fileio/bitreader.rs` | ADT image path migrated; PNG channel edge case documented below |
| `fileio/bin.go` | BIN offset table and room-background stream extraction | `src/fileio/bin.rs` | Minimal indexed ADT entry reading migrated; TIM masks are not parsed |
| `fileio/bitreader.go`, `fileio/streamreader.go` | Bounded binary and bit reads | `src/fileio/bitreader.rs`, bounded `Read`/`Seek` in `bin.rs` | ADT bit reader and BIN range validation migrated |
| Other `fileio/` modules | RDT, TIM, MD1, EMD, VAB, and other formats | Future modules | Not started |
| `state/`, `game/`, `world/`, `ui/` | Game state, simulation, collision, menus | Future matching Rust modules | Not started |

The Go dependency configuration is in `go.mod`: Go 1.23, `go-gl/gl`, `go-gl/glfw`, and `go-gl/mathgl`. The Rust bootstrap deliberately uses SDL2 for the platform layer and the `gl` crate for runtime function loading. No Go dependency is executed from Rust.

## Window, renderer, and shader findings

The existing `client/windowhandler.go` requests OpenGL 3.3 core, forward-compatible, through GLFW. It swaps buffers before polling events and closes when the mapped Escape action is active. The Rust bootstrap changes the window size to the stage requirement of `640×480`, requests `GLProfile::GLES` version `2.0` through SDL2, attempts SDL VSync with a non-fatal warning if unavailable, polls SDL events, handles `Esc` and `Quit`, and updates the viewport from the drawable framebuffer dimensions after resize.

The existing `render/rendermain.go` loads desktop OpenGL functions, sets a black clear color, enables depth testing, multisampling, and alpha blending, then initializes shaders and scene systems. The Rust renderer retains its blue clear and can select the original PNG probe or an ADT image. Both sources upload RGBA with `glTexImage2D` and draw a UV-mapped rectangle as six VBO vertices. The GLSL ES 1.00 fragment shader samples it with `texture2D`; filtering and clamp-to-edge parameters support non-power-of-two dimensions in GLES 2.0. The viewport-fit scale preserves the image aspect ratio. ADT data is decoded once before the event loop and the RGBA buffer is consumed during texture creation. The GPU texture and VBO are deleted when their owners are dropped. No VAO or desktop-only API is used.

The current Go shaders are not portable to the requested GLES 2.0 profile without changes. They declare `#version 410`, use desktop shader interfaces such as `layout` and `out`, and rely on operations such as matrix inversion. The Rust texture probe uses GLSL ES 1.00 syntax (`attribute`, `varying`, `texture2D`, `gl_FragColor`) and a fragment precision declaration. It logs compile and link info logs on failure. The Go shaders remain unported.

The Go renderer also assumes desktop OpenGL objects and functions. In particular, VAO use, shader interfaces, multisampling, and the complete texture/depth pipeline need an explicit GLES compatibility pass. VAOs may require an extension on GLES 2.0, so they must not be treated as available until the runtime extension string is checked or the renderer is written around VBO state without them.

## Platform decision

The Pi Zero 2 W hardware specification lists a quad-core Cortex-A53 at 1 GHz, 512 MB of SDRAM, and OpenGL ES 1.1/2.0 graphics. The intended headless display path is SDL2 with KMSDRM/EGL, selected at runtime with `SDL_VIDEODRIVER=kmsdrm`. Mesa's VC4 driver supports GLES2 on Raspberry Pi 0–3. The application prints `GL_VENDOR`, `GL_RENDERER`, `GL_VERSION`, and `GL_SHADING_LANGUAGE_VERSION`. Acceleration reporting is a heuristic: known software strings (such as `llvmpipe`/`softpipe`) are labeled software, known GPU strings (such as `VC4`, `V3D`, or `VideoCore`) hardware, and other values undetermined.

WSL exposes both `DISPLAY` and `WAYLAND_DISPLAY` in the current environment, but SDL2 development packages, `pkg-config`, and a C linker are not installed. The Rust project links against SDL2 supplied by the operating system rather than embedding a second SDL build. This keeps the platform backend explicit: WSL can use its Wayland/X11 integration, while Raspberry Pi OS can supply the SDL2 build that exposes KMSDRM/EGL. The Pi instructions document the required DRM/GBM development packages and require an actual device test.

## ADT source analysis and migrated behavior

The Go code inspected read-only was `fileio/adt.go`, `fileio/bin.go`, `fileio/bitreader.go`, and `fileio/streamreader.go`; indirect behavior was followed into `resource/image16bit.go`, `resource/paths.go`, `game/roomscript.go`, `state/gamestate.go`, `render/texture.go`, `render/cameramask.go`, and `cmd/fileconv/fileconv.go`.

| Go function/behavior | Rust counterpart | Notes |
| --- | --- | --- |
| `LoadADTFile`, `LoadADTStream` | `load_adt_file`, `decode_adt_stream`, `decode_adt_bytes` | Contextual `Result` errors, compressed-input and output caps |
| `unpackADT`, `initUnpackBlock`, `readBinaryNumber`, `BitReader` | `unpack_adt`, `initialize_unpack_block`, `BitReader` | MSB-first bit stream after the first four bytes; dynamic Huffman tables and a 16 KiB LZ ring |
| `restoreImage` | `reconstruct_image` | Packed 256×240 left region plus two interleaved 64-pixel-wide regions on the right, reconstructed as 320×240 row-major words |
| `ConvertToRenderData` | `AdtImage::pixels` | Flat row-major 16-bit A1B5G5R5 words |
| `ConvertToPNG` | `AdtImage::write_png` / `to_diagnostic_rgba` | Diagnostic conversion matches Go's exported PNG behavior, including its signed 5-bit parsing quirk |
| `resource.convert16BitColor` | `AdtImage::to_rgba` | Runtime texture conversion: each 5-bit channel is multiplied by 8; an all-zero word is transparent |
| `LoadBINFile`, `LoadBIN` | `BinArchive::open` | Reads a bounded offset table, preserves empty slots, validates monotonically ordered offsets and file bounds |
| `ExtractRoomBackground` | `BinArchive::entry`, `decode_adt_entry` | Extracts and decodes the ADT part only; the trailing TIM mask is deliberately not parsed |
| `GetBackgroundImageNumber` | User-selected BIN slot | Go computes `((stage - 1) * 512) + (room * 16) + camera`; stage 1, room 0, camera 0 selects slot 0 |

The ADT stream has a four-byte prefix, then bit-packed blocks. Each nonzero block gives a decoded-symbol count and initializes three Huffman tables; literal symbols emit bytes, while larger symbols select an LZ back-reference length and distance. A zero-length block terminates the stream. Output bytes form little-endian 16-bit words. Rust rejects truncated bitstreams, malformed code tables, impossible run lengths, overlarge offsets, and output beyond configured limits instead of relying on unchecked Go reads. It requires enough output for the 320×240 background and retains any trailing payload in `raw_data` for diagnostics.

The installed game-data tree was inspected without renaming or copying proprietary files. It contains `data/Common/bin/`, `data/Common/Data/`, `data/Common/file/`, `data/Pl0/Rdt/`, and `data/Pl1/Rdt/`. The actual capitalization is `Data` and `Rdt`; the Go path constants `Common/DATP` and `RDP` do not match this installation and were not substituted or renamed. Standalone ADTs and BIN resources both exist. Two real resources were decoded:

| Resource | On-disk details | Decoded result |
| --- | --- | --- |
| `data/Common/Data/Title_bg.adt` | 29,675 compressed bytes | 163,840 decoded bytes (81,920 words); 320×240 image |
| `data/Common/bin/roomcut.bin`, entry 0 | Archive is 71,948,686 bytes; table has 3,584 slots; entry offset 14,336 and length 78,083 | 229,920 decoded bytes (114,960 words), including 66,080 bytes after the 163,840-byte background; 320×240 image |

For the BIN image, Go's formula maps slot 0 to stage 1, room 0, camera 0. The first 163,840 bytes are the packed background; Go treats subsequent bytes as a TIM image mask. Rust currently decodes and retains that trailing data but does not parse the TIM mask. The mask is not required to validate or display the extracted background.

### Go/Rust comparison and color behavior

The Go decoder was executed from a temporary Go container with the Go repository mounted read-only. For both the standalone ADT and `roomcut.bin` entry 0, Rust and Go produced byte-identical decompressed payloads and byte-identical reconstructed 16-bit pixel arrays (76,800 image words each). The diagnostic PNGs are also pixel-identical when Rust's diagnostic conversion reproduces Go's `ConvertToPNG` behavior.

That diagnostic helper has a bug: it parses each 5-bit channel with `strconv.ParseInt(..., bitSize=5)`, whose signed range ends at 15. Values 16–31 therefore saturate to 15 after the ignored range error. The runtime Go rendering path uses `resource.convert16BitColor`, which correctly multiplies all 0–31 channel values by 8 and makes only the all-zero source word transparent. Rust preserves both behaviors explicitly: `write_png` reproduces the legacy Go diagnostic output for comparison, while `to_rgba` follows the runtime renderer for OpenGL upload. Thus the diagnostic PNG intentionally differs in some channel values from the live texture; this is a compatibility choice, not a decompression mismatch.

## Rust files for the current stage

| File | Purpose |
| --- | --- |
| `Cargo.toml` | Rust package metadata, SDL2/OpenGL function loading, and pure-Rust PNG decoding |
| `Cargo.lock` | Locked dependency versions generated by Cargo |
| `src/main.rs` | Process entry point and top-level error reporting |
| `src/app.rs` | SDL2 initialization, GLES 2.0 context, event loop, GL context details, and acceleration assessment |
| `src/graphics/mod.rs` | Graphics module boundary |
| `src/graphics/renderer.rs` | GLSL ES 1.00 compile/link, VBO/UV setup, aspect-preserving viewport fit, blue clear, and rectangle draw |
| `src/graphics/texture.rs` | PNG decode and shared RGBA texture upload, GLES2-compatible formats, texture lifetime |
| `src/fileio/mod.rs` | ADT/BIN module boundary |
| `src/fileio/bitreader.rs` | Checked MSB-first bit reads and Go-style prefix number |
| `src/fileio/adt.rs` | ADT decompression, 320×240 reconstruction, runtime RGBA conversion, diagnostic PNG output |
| `src/fileio/bin.rs` | Checked BIN offset table and bounded ADT-entry stream |
| `assets/test_texture.png` | Non-proprietary 320×180 RGBA test image with transparency and orientation colors |
| `README.md` | Build/run instructions, Pi notes, and stage boundary |
| `docs/MIGRATION.md` | Go analysis, module mapping, portability findings, and validation report |

Game data is present only in the user's local ignored `data/` directory. No proprietary files or extracted images are tracked. The PNG test pattern remains available with the default no-argument mode or `--png`.

## Validation report

The Rust validation was executed in a disposable `rust:latest` container with build tools and SDL2 development packages installed; Cargo artifacts were directed to the container's `/tmp`. The Go comparison used a separate temporary container with the original Go repository mounted read-only. Diagnostic PNGs were written under ignored `target/diagnostics/`.

| Command | Result |
| --- | --- |
| `cargo fmt --check` | Passed |
| `cargo check` | Passed |
| `cargo clippy -- -D warnings` | Passed, no warnings |
| `cargo test` | Passed: 11 unit tests |
| `cargo build --release` | Passed |
| Rust decode: `roomcut.bin` entry 0 | Passed: 320×240; 229,920 raw bytes; diagnostic PNG created |
| Rust decode: `Title_bg.adt` | Passed: 320×240; 163,840 raw bytes; diagnostic PNG created |
| Go/Rust raw payload and reconstructed 16-bit comparison | Passed for both resources: exact byte equality |
| Go/Rust diagnostic PNG pixel comparison | Passed for both resources after mirroring Go `ConvertToPNG` channel clamping |
| SDL/OpenGL ES runtime on Raspberry Pi | Not run in this environment; the Pi hardware is unavailable here |
| GL driver/context strings for this ADT stage | Not captured; run the `--bin` command on the Pi to record `GL_VENDOR`, `GL_RENDERER`, `GL_VERSION`, and `GL_SHADING_LANGUAGE_VERSION` |

The CLI and pixel data were validated without a graphics context. The live GL upload, blue clear behind transparent ADT words, KMSDRM presentation, VSync behavior, and runtime driver strings still require the Raspberry Pi test.

## Next migration step

Do not start another migration module until the ADT texture and graphics-context information have been confirmed on the Raspberry Pi. RDT parsing, TIM mask composition, models, characters, collision, and game logic remain out of scope.

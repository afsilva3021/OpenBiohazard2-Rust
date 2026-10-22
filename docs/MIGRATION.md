# OpenBiohazard2 Go → Rust


## Scope of this document

The source of truth inspected for the migration is the existing project at `../OpenBiohazard2`. Its directory was not modified, reorganized, or copied into the Rust project. The Rust implementation covers the SDL2/OpenGL ES 2.0 renderer, PNG test mode, ADT decompression and reconstruction, BIN entry lookup, minimal RDT/RID camera metadata, RDT-to-background mapping, PNG diagnostics, and direct ADT texture upload.

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
| `client/windowhandler.go` | GLFW window, context, swap, resize, frame timing | `src/app.rs` plus a future `platform` module | SDL2 window, event loop, and Left/Right RDT camera navigation implemented |
| `client/inputhandler.go` | Key and mouse action mapping | `src/app.rs` for this stage; future `src/input.rs` | Escape/Quit and non-repeating arrow-key camera changes |
| `render/rendermain.go` | Renderer setup and frame orchestration | `src/graphics/renderer.rs` | Blue clear, textured rectangle, aspect-preserving resize, and transactional texture replacement |
| `render/opengl_renderer.go` | VAO/VBO/attributes, textures, draw calls | Future graphics modules | GLES2 VBO, UVs, PNG/ADT texture upload, two-triangle draw, and one active room texture |
| `shader/shader.go` | Shader file loading, compilation, linking | `src/graphics/renderer.rs` for this probe; future `src/graphics/shader.rs` | In-memory GLSL ES 1.00 texture shaders with compile/link error logs |
| `shader/shadersystem.go` | Program lifetime and uniform cache | Future `src/graphics/shader.rs` | Not started |
| `shader/openbiohazard2.vert` | Desktop GLSL vertex shader | Future GLES shader asset | Requires adaptation |
| `shader/openbiohazard2.frag` | Desktop GLSL fragment shader | Future GLES shader asset | Requires adaptation |
| `resource/` | Game-data paths, validation, image loading | Future `src/resource/` | Not started |
| `fileio/adt.go` | ADT stream decompression, background layout, render words, PNG conversion | `src/fileio/adt.rs`, `src/fileio/bitreader.rs` | ADT image path migrated; PNG channel edge case documented below |
| `fileio/bin.go` | BIN offset table and room-background stream extraction | `src/fileio/bin.rs` | Minimal indexed ADT entry reading migrated; TIM masks are not parsed |
| `fileio/bitreader.go`, `fileio/streamreader.go` | Bounded binary and bit reads | `src/fileio/bitreader.rs`, bounded `Read`/`Seek` in `bin.rs` | ADT bit reader and BIN range validation migrated |
| `fileio/rdt.go`, `fileio/rdt_rid.go`, `fileio/rdt_pri.go` | RDT header/offset table, camera RID records, PRI camera-mask counts | `src/fileio/rdt.rs` | Core header, camera positions/FOV, PRI count/bounds only; other RDT payloads and mask composition are not migrated |
| `fileio/rdt_lit.go` | 40-byte per-camera light records used by Go's environment lighting | Not migrated | Inspected; lighting does not affect selecting the 2D room background and remains outside this stage |
| Other `fileio/` modules | TIM, MD1, EMD, VAB, and other formats | Future modules | Not started |
| `state/`, `game/`, `world/`, `ui/` | Game state, simulation, collision, menus | Future matching Rust modules | Not started |

The Go dependency configuration is in `go.mod`: Go 1.23, `go-gl/gl`, `go-gl/glfw`, and `go-gl/mathgl`. The Rust bootstrap deliberately uses SDL2 for the platform layer and the `gl` crate for runtime function loading. No Go dependency is executed from Rust.

## Window, renderer, and shader findings

The existing `client/windowhandler.go` requests OpenGL 3.3 core, forward-compatible, through GLFW. It swaps buffers before polling events and closes when the mapped Escape action is active. The Rust bootstrap changes the window size to the stage requirement of `640×480`, requests `GLProfile::GLES` version `2.0` through SDL2, attempts SDL VSync with a non-fatal warning if unavailable, polls SDL events, handles `Esc` and `Quit`, and updates the viewport from the drawable framebuffer dimensions after resize.

The existing `render/rendermain.go` loads desktop OpenGL functions, sets a black clear color, enables depth testing, multisampling, and alpha blending, then initializes shaders and scene systems. The Rust renderer retains its blue clear and can select the original PNG probe or an ADT image. Both sources upload RGBA with `glTexImage2D` and draw a UV-mapped rectangle as six VBO vertices. The GLSL ES 1.00 fragment shader samples it with `texture2D`; filtering and clamp-to-edge parameters support non-power-of-two dimensions in GLES 2.0. The viewport-fit scale preserves the image aspect ratio. For RDT mode, only the initial camera is decoded before entering the event loop; subsequent backgrounds decode only on a Left/Right key press. A replacement GPU texture is fully created before the renderer swaps out the previous one, so decode or upload errors keep the current camera visible. The GPU texture and VBO are deleted when their owners are dropped. No VAO or desktop-only API is used.

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
| `GetBackgroundImageNumber` | `RoomIdentity::background_image_index` and `--rdt` | Go computes `((stage - 1) * 512) + (room * 16) + camera`; Rust parses stage/room/player from `ROOM%01d%02x%01d.RDT` and applies that exact formula |

## RDT room and camera data

The read-only Go analysis covered `fileio/rdt.go`, `fileio/rdt_rid.go`, `fileio/rdt_pri.go`, and `fileio/rdt_lit.go`, plus the call sites in `game/roomscript.go` and `state/gamestate.go`, the RDT filename constant in `resource/paths.go`, and BIN extraction in `fileio/bin.go`. `rdt_lit.go` stores 40 bytes of per-camera light settings and Go applies those when building its environment light; Rust does not yet need those values to select or display the 2D background. Go `LoadRDTFile` loads many unrelated sections as well; this Rust stage intentionally migrates only the fields needed to identify and inspect room cameras.

An RDT begins with an 8-byte count header followed by 23 little-endian `u32` offsets, for a 100-byte core header. The RID offset is the eighth offset field (byte 36 from the start of the file). Each camera RID record is 32 bytes: flags and screen distance (`u16`), camera-from and camera-to XYZ (`i32` each), and a mask/PRI offset (`u32`). Rust bounds-checks the entire table, validates all nonzero section offsets against file length, and caps RDT input at 16 MiB. It follows each non-sentinel PRI pointer far enough to validate its header and relative-offset table and report its declared mask count; it does not decode mask rectangles or compose TIM masks.

Go computes FOV using `distance = DistanceToScreen >> 7` and `2 * atan(120 / distance)` in degrees for the game's 320×240 backgrounds. Rust follows that calculation and reports the raw fixed-point screen-distance field, coordinates, FOV, mask count, and camera index. The `ROOM%01d%02x%01d.RDT` filename identifies stage, room, and player. Go's `GameDef.GetBackgroundImageNumber` then maps it as `((stage - 1) * 512) + (room * 16) + camera`; Rust uses that same formula and refuses camera indices at or above the 16-entry per-room stride because those would collide with the next room's slots.

The real inspected file is `data/Pl0/Rdt/ROOM1000.RDT` (114,320 bytes). Its header declares 9 cameras and its RID table begins at offset 100. The filename identifies stage 1, room `0x00`, player 0, so the Go formula maps its cameras directly to `roomcut.bin` entries 0 through 8. Those entries exist in the actual `data/Common/bin/roomcut.bin`; Rust successfully decodes all nine as 320×240 backgrounds. Entries 0–6 have 229,920 bytes of decompressed ADT payload, including trailing mask data; entries 7–8 have 163,840 bytes. The texture path reconstructs the 320×240 background in each case, while mask composition remains unimplemented. The actual directory capitalization is `Rdt`; it was not renamed to match the Go `RDP` path constant.

Go's own `LoadRDTFile` was run against this same RDT in a disposable container with the original Go tree mounted read-only. Go and Rust agree on all nine RID camera positions, FOV (59.963276 degrees), PRI offsets, and declared mask counts. Every camera has a raw screen-distance value of 26,684 (`>> 7` gives 208); flags are zero except camera 8, whose flag is 1. The camera table is:

| Camera | From XYZ | To XYZ | PRI offset | Masks | `roomcut.bin` slot |
| ---: | --- | --- | ---: | ---: | ---: |
| 0 | (11268, -2664, -3600) | (22356, -2466, 522) | 1252 | 32 | 0 |
| 1 | (10512, -4428, -8478) | (14328, 360, 8046) | 1632 | 47 | 1 |
| 2 | (2610, -4428, -7974) | (6246, 360, 7992) | 2128 | 51 | 2 |
| 3 | (-1206, -4428, -8766) | (-4554, 234, 5490) | 2688 | 39 | 3 |
| 4 | (-9342, -4428, -8280) | (-12834, 108, 4950) | 3048 | 34 | 4 |
| 5 | (-14958, -4752, -10278) | (-20736, -54, 2592) | 3384 | 18 | 5 |
| 6 | (-15624, -3024, -16722) | (-20502, -882, 3006) | 3580 | 17 | 6 |
| 7 | (-15318, -3132, -4968) | (-18846, -2592, -16722) | 3752 | 0 (PRI sentinel) | 7 |
| 8 | (-12996, -3006, -16020) | (-18324, -2844, -20898) | 3756 | 0 (PRI sentinel) | 8 |

Use `--rdt PATH` to list cameras without starting SDL. `--rdt PATH CAMERA [ROOMCUT.BIN]` selects and renders the formula-derived background through the existing ADT decoder and renderer. `--decode-rdt PATH CAMERA OUTPUT.PNG [ROOMCUT.BIN]` exercises the same selection and exports a diagnostic PNG without requiring a graphics context. The default archive is `data/Common/bin/roomcut.bin`; the user can override it explicitly. An RDT whose basename does not match Go's filename pattern can still be listed, but Rust will not guess its room identity or map it to a BIN image.

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
| `src/main.rs` | Process entry point, ADT/PNG modes, RDT camera listing/selection, and top-level error reporting |
| `src/app.rs` | SDL2 initialization, GLES 2.0 context, event loop, camera selection, GL context details, and acceleration assessment |
| `src/graphics/mod.rs` | Graphics module boundary |
| `src/graphics/renderer.rs` | GLSL ES 1.00 compile/link, VBO/UV setup, aspect-preserving viewport fit, blue clear, rectangle draw, and safe texture replacement |
| `src/graphics/texture.rs` | PNG decode and shared RGBA texture upload, GLES2-compatible formats, texture lifetime |
| `src/fileio/mod.rs` | ADT/BIN/RDT module boundary |
| `src/fileio/bitreader.rs` | Checked MSB-first bit reads and Go-style prefix number |
| `src/fileio/adt.rs` | ADT decompression, 320×240 reconstruction, runtime RGBA conversion, diagnostic PNG output |
| `src/fileio/bin.rs` | Checked BIN offset table and bounded ADT-entry stream |
| `src/fileio/rdt.rs` | Bounded RDT core/RID/PRI metadata reader, filename identity, Go camera-to-BIN mapping, and lazy selected-camera loading |
| `assets/test_texture.png` | Non-proprietary 320×180 RGBA test image with transparency and orientation colors |
| `README.md` | Build/run instructions, Pi notes, and stage boundary |
| `docs/MIGRATION.md` | Go analysis, module mapping, portability findings, and validation report |

Game data is present only in the user's local ignored `data/` directory. No proprietary files or extracted images are tracked. The PNG test pattern remains available with the default no-argument mode or `--png`.

## Validation report

The Rust validation was executed in a disposable `rust:latest` container with build tools, SDL2 development packages, `rustfmt`, and Clippy installed. Cargo artifacts were written under the ignored workspace `target/` directory. The Go comparison used a separate temporary container with the original Go repository mounted read-only. Diagnostic PNGs were written under ignored `target/diagnostics/`.

| Command | Result |
| --- | --- |
| `cargo fmt --check` | Passed |
| `cargo check` | Passed |
| `cargo clippy -- -D warnings` | Passed, no warnings |
| `cargo test` | Passed: 20 unit tests, including truncated RDT/RID/PRI data, invalid offsets and camera indices, filename parsing, Go mapping, and camera cycling |
| `cargo build --release` | Passed |
| Rust decode: `roomcut.bin` entry 0 | Passed: 320×240; 229,920 raw bytes; diagnostic PNG created |
| Rust decode: `Title_bg.adt` | Passed: 320×240; 163,840 raw bytes; diagnostic PNG created |
| Go/Rust raw payload and reconstructed 16-bit comparison | Passed for both resources: exact byte equality |
| Go/Rust diagnostic PNG pixel comparison | Passed for both resources after mirroring Go `ConvertToPNG` channel clamping |
| RDT camera comparison with Go `LoadRDTFile` | Passed for all 9 cameras: position, FOV `59.963276°`, PRI offsets, and mask counts match |
| Rust `--rdt data/Pl0/Rdt/ROOM1000.RDT` | Passed: lists all 9 cameras and maps them to entries 0–8; arrows navigate that set with wraparound |
| Rust `--decode-rdt` for ROOM1000 cameras 0–8 | Passed in the SDL2 runtime container: all nine mapped BIN entries decode to 320×240 PNGs; entries 0–6 yield 229,920 raw bytes and entries 7–8 yield 163,840 bytes. The prior Go/Rust byte comparison specifically covered entry 0. |
| Variable-count RDT inspection | Passed: `ROOM1010.RDT` has 13 cameras (entries 16–28); `ROOM30C0.RDT` has 17 records, with the 16 uniquely mapped cameras at entries 1216–1231; its record 16 is intentionally not selectable because it aliases the next room under Go's 16-slot stride |
| Rust `--decode-rdt data/Pl0/Rdt/ROOM30C0.RDT 0` | Passed: camera 0 maps to entry 1216 and decodes to 320×240 |
| SDL/OpenGL ES runtime for camera switching on Raspberry Pi | Not run in this environment; this turn had no Pi/KMSDRM display attached |
| GL driver/context strings | Previously confirmed by the user on Pi for the existing ADT render path: OpenGL ES 2.0 and Broadcom VC4; arrow-key switching still needs an end-to-end Pi run |

The CLI and pixel data were validated without a graphics context. The existing renderer was already confirmed by the user on the Pi using a direct `roomcut.bin` entry. The new `--rdt` camera-selection route needs the Pi run below to confirm its end-to-end SDL/GLES presentation.

### `sdl2-sys` UTF-8 build failure

The reported `sdl2-sys` UTF-8 failure was not reproduced in the Rust validation container. The project uses `sdl2-sys 0.38.0`; its Linux build script converts the output of `cc -dumpversion` with `std::str::from_utf8(...).unwrap()` (in the Cargo registry copy of `sdl2-sys/build.rs`). Non-UTF-8 compiler output can therefore panic inside that dependency before project tests run. This is separate from the RDT parser and is not patched in the repository. In the current container, the compiler version output is ordinary ASCII and the tests proceed. If it recurs on the Pi, capture `LC_ALL=C cc -dumpversion` and `cargo test -vv` output; the exact compiler wrapper/locale then determines whether upgrading or patching the dependency is appropriate. The WSL host used here also lacks `cc` and `pkg-config`, so its direct Cargo build cannot reach this dependency code.

## Etapa 6 — runtime camera switching

The SDL event loop now handles non-repeating Left/Right key-down events only in RDT camera mode. Navigation wraps around the cameras that have a unique slot under Go's 16-entry-per-room mapping; Escape and window-close behavior remain unchanged. The initial camera still comes from `--rdt RDT CAMERA [ROOMCUT.BIN]` and is validated against both the RDT camera list and the roomcut formula before the BIN is opened. For each key press, the session decodes only the target ADT. It does not pre-decode or cache the room's other backgrounds, and it does no decode work in the frame loop.

Texture replacement is transactional: RGBA conversion and GL allocation/upload complete into a temporary texture before the renderer replaces its active texture. A decode, dimension, or GL upload error is reported to stderr and leaves the previous texture and current-camera index intact. During the brief replacement operation the old and candidate GPU textures coexist; each room background is only 320×240, and no per-camera texture cache is retained. Camera changes print the selected camera/count, derived BIN entry and byte range, RDT coordinates, FOV, and mask count.

The real `data/Pl0/Rdt/ROOM1000.RDT` has 9 cameras. All nine corresponding `roomcut.bin` entries (0–8) were decoded successfully as 320×240 images using the current Rust session path. `ROOM1010.RDT` has 13 cameras, confirming the selection count is not fixed at 9. The inspected `ROOM30C0.RDT` has 17 camera records. Its indices 0–15 map to entries 1216–1231; camera 16 is displayed as `roomcut entry=unavailable` and excluded from arrow navigation because it collides with the next room's first slot under the confirmed Go formula. Across the local `data/Pl0/Rdt/` folder, 250 RDT files were found, with camera counts from 2 to 17; no fixed count is assumed.

The pure index-cycle tests cover forward wrap from camera 8 to 0, reverse wrap from 0 to 8, single-camera rooms, zero counts, and invalid current indices. The runtime Pi test remains necessary to verify that SDL delivers the arrow keys in KMSDRM and that each uploaded image appears on the VC4 display. Use:

```bash
SDL_VIDEODRIVER=kmsdrm ./target/release/openbiohazard2-rust --rdt data/Pl0/Rdt/ROOM1000.RDT 0
```

Press Left/Right and observe the terminal's camera/entry logs; press Escape to exit. The previously confirmed GLES2 renderer and VC4 context are reused. No Go source, game data, or generated diagnostic PNG is modified or tracked.

## Etapa 7 — static PLD geometry overlay

### Go analysis and real resource

The read-only Go review covered `fileio/pld.go`, `fileio/edd.go`, `fileio/emr.go`, `fileio/md1.go`, `fileio/tim.go`, `fileio/streamreader.go`, `geometry/md1geometry.go`, `geometry/triangle.go`, `geometry/quad.go`, `render/AnimatedEntity.go`, `render/animation.go`, `render/camera.go`, `render/viewsystem.go`, `render/constants.go`, `render/rendermain.go`, `game/player.go`, `game/debug.go`, `main.go`, and the GLSL files under `shader/`. The `cmd/modeldumper` utility was run against the original PLD with the Go tree mounted read-only in a disposable Go container.

The user's real `data/Pl0/PLD/PL00.PLD` is 145,304 bytes. Its directory has four sections in this order: EDD at byte 8, EMR at 1,436, MD1 at 25,052, and TIM at 45,416; the directory itself begins at byte 145,288. The Go reader identifies 15 EMR bones, 15 MD1 mesh components, 293 animation frames, 10 EDD pose entries, and a 384×256 TIM with three palettes. This verifies the PL00 resource structure for this installed PC edition only; it does not establish compatibility with every regional or release variant.

Go reports 187 triangles and 267 quads across the 15 MD1 components. Following the Go neutral-pose hierarchy and the Go quad split yields 721 triangles and 2,163 expanded triangle-list vertices. A separate comparison over the Go dumper's indexed geometry and the Rust loader produced the same neutral-pose bounds: min `[-267, -3044, -559]`, max `[290, -7, 559]`. This checks indexed vertex selection, neutral hierarchy translations, and triangle/quad expansion for this file. It does not compare animated poses or texture sampling.

One PL00 MD1 header declares a section length of 13,848 bytes even though its referenced object regions continue through the MD1-to-TIM boundary. Rust follows the PLD directory and bounds MD1 reads by the next section offset, validates each referenced range against that boundary, and does not trust the smaller internal length as the section extent. This behavior is based on the observed resource and Go loader layout; more PLD variants still need inspection.

### Rust implementation and rendering

| Go implementation | Rust implementation | Migrated behavior |
| --- | --- | --- |
| `fileio.PLDFile` / PLD section directory | `fileio::pld::parse_static_pld`, `load_static_pld` | Bounded directory and section parsing; input limited to 32 MiB |
| `LoadEMRFile`, `EMROutput`, `ArmatureChildren`, `RelativePositionData` | `parse_neutral_skeleton`, `visit_skeleton` | Validated hierarchy, cycle/parent checks, cumulative neutral translations |
| `LoadMD1File`, component arrays, triangle/quad index records | `parse_md1` and bounded record helpers | Validated mesh tables and indices; expands indexed triangles and Go's quad split into one static position buffer |
| `render.buildComponentTransforms` with `Animation.CurPose == -1` | Neutral offsets in `StaticPldModel` | Static neutral pose only; EDD frames and rotations are not loaded for rendering |
| `render.camera` and `render.viewsystem` | `graphics::model_renderer::camera_mvp` | RDT camera From/To, Go's up vector, FOV, near/far values, and the aspect fit used by the existing background renderer |
| Existing renderer's camera background draw | `ModelRenderer` pass after the 2D draw | GLES2 / GLSL ES 1.00 solid-color shader, static VBO, depth testing, and a 16-bit SDL depth-buffer request |

The loader does not parse or upload PLD TIM palettes/textures, normals, material flags, EDD pose sequences, or animation rotations. The overlay uses a solid diagnostic color. It uses the source Go debug transform found in `game/debug.go` and `main.go`: position `(18800, 0, -3160)`, yaw `180°`. That known location is accepted only for `ROOM1000.RDT`; other room identities are rejected rather than receiving an invented transform. Camera switching reuses the model VBO and updates its matrix; it does not re-decode model data or rebuild the VBO per frame. The 2D renderer resets its vertex attribute binding before drawing the next background, since GLES2 attribute state is shared between the background and model passes.

Run the parser without SDL/OpenGL, then view the static overlay on the Pi:

```bash
./target/release/openbiohazard2-rust --inspect-pld data/Pl0/PLD/PL00.PLD
SDL_VIDEODRIVER=kmsdrm ./target/release/openbiohazard2-rust --rdt-model data/Pl0/Rdt/ROOM1000.RDT 0 data/Pl0/PLD/PL00.PLD
```

In the window, Left/Right select cameras and Escape exits. Startup logs report component/face/vertex counts, neutral bounds, the diagnostic placement, selected RDT camera and BIN entry, and the configured depth-buffer size. PRI/TIM masks are still needed for correct scenery occlusion, so the model can draw over foreground scenery.

### Etapa 7 validation and limitations

| Check | Result |
| --- | --- |
| Go `cmd/modeldumper` on `data/Pl0/PLD/PL00.PLD` | Passed: 15 components; 187 triangles, 267 quads; 15 bones; 293 animation frames; TIM 384×256, 3 palettes |
| Go indexed geometry vs Rust neutral expanded mesh | Passed for counts and bounds: 721 triangles, 2,163 expanded vertices, min `[-267,-3044,-559]`, max `[290,-7,559]` |
| `cargo fmt --check` | Passed |
| `cargo check` | Passed |
| `cargo clippy -- -D warnings` | Passed with no warnings |
| `cargo test` | Passed: 27 unit tests, including valid synthetic PLD geometry and malformed/truncated section, offset, index, and hierarchy cases |
| `cargo build --release` | Passed |
| Rust `--inspect-pld data/Pl0/PLD/PL00.PLD` | Passed; reports 15 components, 721 triangulated faces, and the neutral bounds above |
| SDL/OpenGL ES overlay on Raspberry Pi Zero 2 W | Not run here; no Pi/KMSDRM display is attached. The previously user-confirmed VC4/GLES2 base renderer is reused, but the new depth-tested model pass still needs visual verification on the Pi. |

CPU parsing and the Go geometry comparison show that PL00's static mesh is structurally valid and that its neutral hierarchy translation matches the Go output. They do not prove ideal framing on every camera or hardware depth-buffer behavior. PLD variants other than the installed `PL00.PLD`, animated poses, material/texture rendering, masks, and gameplay placement remain unverified or out of scope. No game files, generated diagnostic JSON/PNGs, or Go source changes are tracked.

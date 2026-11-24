# OpenBiohazard2-Rust

Rust migration of [OpenBiohazard2](../OpenBiohazard2). The Go repository is the read-only behavioral reference. The current Rust stage keeps the SDL2/OpenGL ES 2.0 renderer, decodes ADT backgrounds from independent `.adt` files or `roomcut.bin`, and reads RDT room headers and camera records.

The 640×480 SDL2 window uses GLSL ES 1.00, a VBO rectangle made of two triangles, and a direct RGBA texture upload. The image is decoded once at startup, fitted to the drawable framebuffer without changing its aspect ratio, and released with the renderer. The blue clear color and Escape/close controls remain available. With no arguments the application still shows `assets/test_texture.png`.

## Build and run on Raspberry Pi Zero 2 W

Install SDL2 development files and build on the Pi so Cargo uses the Pi OS ABI. Run commands from the Rust project root. Keep the proprietary game files in the ignored local `data/` directory using their original capitalization and layout.

```bash
sudo apt update
sudo apt install build-essential pkg-config libsdl2-dev libdrm-dev libgbm-dev

cd ~/OpenBiohazard2-Rust
CARGO_BUILD_JOBS=1 cargo fmt --check
CARGO_BUILD_JOBS=1 cargo check
CARGO_BUILD_JOBS=1 cargo clippy -- -D warnings
CARGO_BUILD_JOBS=1 cargo test
CARGO_BUILD_JOBS=1 cargo build --release
```

Run an ADT stored in the room background archive, or an independent ADT:

```bash
SDL_VIDEODRIVER=kmsdrm ./target/release/openbiohazard2-rust --bin data/Common/bin/roomcut.bin 0
SDL_VIDEODRIVER=kmsdrm ./target/release/openbiohazard2-rust --adt data/Common/Data/Title_bg.adt
```

List the cameras in an RDT, including the corresponding `roomcut.bin` entry derived from the Go room/camera formula:

```bash
./target/release/openbiohazard2-rust --rdt data/Pl0/Rdt/ROOM1000.RDT
```

Render camera 0 from that RDT. The command derives entry 0 from `ROOM1000.RDT`; the optional final argument overrides the default BIN path:

```bash
SDL_VIDEODRIVER=kmsdrm ./target/release/openbiohazard2-rust --rdt data/Pl0/Rdt/ROOM1000.RDT 0
SDL_VIDEODRIVER=kmsdrm ./target/release/openbiohazard2-rust --rdt data/Pl0/Rdt/ROOM1000.RDT 0 data/Common/bin/roomcut.bin
```

While this window is open, press **Left** or **Right** to move to the previous or next camera. Navigation wraps across the cameras that have an unambiguous Go `roomcut.bin` slot. The selected background is decoded and uploaded only after a key press; the program keeps one room image/texture at a time. If reading or uploading a camera fails, it logs the error and leaves the current image visible. Press **Esc** to exit.

## Inspect a PLD model

Inspect the verified player model before opening a graphics context:

```bash
./target/release/openbiohazard2-rust --inspect-pld data/Pl0/PLD/PL00.PLD
```

## Play a room

Walk the player model through a room with textures, animation, camera masks, collision and automatic camera switching:

```bash
SDL_VIDEODRIVER=kmsdrm ./target/release/openbiohazard2-rust --play data/Pl0/Rdt/ROOM1000.RDT 0 data/Pl0/PLD/PL00.PLD
```

`--rdt-model` is kept as an alias of `--play`.

| Input | Keyboard | Controller |
| --- | --- | --- |
| Walk forward / backward | Up / Down | D-pad or left stick |
| Turn | Left / Right | D-pad or left stick |
| Previous / next camera (debug) | Page Up / Page Down | — |
| Quit | Esc, or Enter+Backspace held 1 s | Start+Select (Back) held 1 s |

What runs in this mode:

- **Model:** PLD mesh with its TIM texture (4/8-bit palettes, Go's per-page UV offset), drawn per MD1 component with that component's bone matrix.
- **Animation:** EDD tables and EMR frames (12-bit angles), 30 ms per frame. Idle uses the neutral skeleton (Go pose −1), walking uses pose 0 and walking backward pose 1, as in Go `game/player.go`.
- **Camera masks:** PRI tiles from the RDT and the mask TIM stored after the background in `roomcut.bin` are drawn into the depth buffer at Go's `depth × 32` distance, so the player walks behind scenery.
- **Collision:** SCA rectangles, triangles, circles and ellipses per floor, plus ramps/stairs height and box climbing (Go `world/collision.go`).
- **Camera switching:** RVD regions switch to the region's `cam1` when the player enters them (Go `world/cameraswitch.go`).
- **Start position:** Go's debug locations (`game/debug.go`) for ROOM1000, 1010, 1020, 1180, 1190, 11A0 and 11B0 with Go's 180° start rotation. Other rooms are rejected until script-driven spawns are migrated.

Not migrated yet: doors and room transitions (SCD scripts/AOT), items, enemies, environment lighting (RDT LIT; the model is shown at full texture brightness), sound and menus.

### Console Zero cartridge

`console-zero/` contains a native cartridge for the Console Zero launcher: copy `info.lua`, `jogo`, the aarch64 release binary and your own `data/` folder into one folder on the JOGOS partition. The Console Zero image needs SDL2 with KMSDRM and OpenGL ES (`BR2_PACKAGE_SDL2=y`, `BR2_PACKAGE_SDL2_KMSDRM=y`, `BR2_PACKAGE_SDL2_OPENGLES=y`). Start+Select held for one second returns to the launcher.

The RDT reader parses the core header, RID cameras, PRI mask tiles, SCA collision and RVD camera switches. To validate/export a selected camera without opening SDL/OpenGL:

```bash
mkdir -p target/diagnostics
./target/release/openbiohazard2-rust --decode-rdt data/Pl0/Rdt/ROOM1000.RDT 0 target/diagnostics/ROOM1000-camera0.png
```

Entry `0` in `roomcut.bin` maps to stage 1, room 0, camera 0 using the Go formula `((stage - 1) * 512) + (room * 16) + camera`. The Rust BIN reader keeps empty table slots so requested indices continue to refer to the original table positions.

SDL prints its active video driver and `GL_VENDOR`, `GL_RENDERER`, `GL_VERSION`, and `GL_SHADING_LANGUAGE_VERSION`. The hardware/software label is a heuristic based on `GL_RENDERER`; check the printed driver string if it reports “undetermined”. Press `Esc` or close the window to exit.

## Decode an ADT to a diagnostic PNG

The diagnostic path decodes without opening a graphics context and writes under ignored `target/diagnostics/` by default in the examples below:

```bash
cargo run --release -- --decode-bin data/Common/bin/roomcut.bin 0 target/diagnostics/room0.png
cargo run --release -- --decode-adt data/Common/Data/Title_bg.adt target/diagnostics/title_bg.png
file target/diagnostics/room0.png target/diagnostics/title_bg.png
```

Both tested resources decode to a 320×240 RGBA image. The diagnostic PNG intentionally matches Go's `ADTOutput.ConvertToPNG`; that Go helper clamps 5-bit channels above 15 because it parses them as signed 5-bit integers. The live texture follows Go's runtime `resource/image16bit.go` conversion instead, preserving the full 0–31 channel range and making an all-zero word transparent.

For another explicit source, use `--adt INPUT.ADT`, `--bin INPUT.BIN ENTRY_INDEX`, `--decode-adt INPUT.ADT OUTPUT.PNG`, or `--decode-bin INPUT.BIN ENTRY_INDEX OUTPUT.PNG`. `--png [PATH]` selects the PNG test texture. `--help` lists all forms. Relative paths are resolved from the current project directory; missing or invalid files return contextual errors.

## Local data and scope

The verified local layout includes `data/Common/bin/roomcut.bin` and standalone images under `data/Common/Data/`. The names are case-sensitive: the inspected install has `Data` and `Rdt`; do not rename them to `DATP` or `RDP`. `data/` and `target/` are ignored by Git. No game files or extracted PNGs are part of the repository.

ADT backgrounds, the minimal RDT/RID camera metadata needed to identify room backgrounds, the confirmed RDT-to-BIN index mapping, and untextured static PLD geometry are in scope. TIM mask composition, PLD textures and animation, other RDT sections, room scripts, collision, and gameplay are not implemented. See [docs/MIGRATION.md](docs/MIGRATION.md) for the Go/Rust mapping, camera/model data, validation results, and known limitations.

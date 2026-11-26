//! Windows (MSVC) only: link against the prebuilt SDL2 Visual C++ libraries
//! and copy `SDL2.dll` next to the executables. Linux/Raspberry Pi builds use
//! the system SDL2 from pkg-config/libsdl2-dev and are not affected.
//!
//! The SDL2 folder is `windows/SDL2` (filled by `scripts/setup-windows.ps1`)
//! or the directory named by the `SDL2_DIR` environment variable.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=SDL2_DIR");
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let sdl_dir = env::var_os("SDL2_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("windows").join("SDL2"));
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        _ => "x64",
    };
    let lib_dir = sdl_dir.join("lib").join(arch);
    let dll = lib_dir.join("SDL2.dll");
    // Rerun when the library appears or moves; a stale cached result (e.g. a
    // `target/` copied from another checkout) would otherwise skip linking.
    println!("cargo:rerun-if-changed={}", lib_dir.join("SDL2.lib").display());

    if !lib_dir.join("SDL2.lib").is_file() {
        println!(
            "cargo:warning=SDL2.lib not found in {}. Run `powershell -ExecutionPolicy Bypass -File scripts\\setup-windows.ps1` or set SDL2_DIR.",
            lib_dir.display()
        );
        return;
    }
    println!("cargo:rerun-if-changed={}", dll.display());
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out → target/<profile>
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let profile_dir = out_dir
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "build"))
        .and_then(|build| build.parent());
    if let Some(profile_dir) = profile_dir {
        for destination in [profile_dir.to_path_buf(), profile_dir.join("deps")] {
            if fs::create_dir_all(&destination).is_ok() {
                if let Err(error) = fs::copy(&dll, destination.join("SDL2.dll")) {
                    println!("cargo:warning=could not copy SDL2.dll: {error}");
                }
            }
        }
    }
}

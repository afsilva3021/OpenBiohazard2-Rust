mod app;
mod fileio;
mod graphics;

use std::{env, ffi::OsString, path::PathBuf, process};

fn main() {
    if let Err(error) = run() {
        eprintln!("OpenBiohazard2-Rust: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let command = args.first().and_then(|value| value.to_str()).unwrap_or("");

    match command {
        "" | "--png" => {
            let path = match args.len() {
                0 => PathBuf::from("assets/test_texture.png"),
                1 if command == "--png" => PathBuf::from("assets/test_texture.png"),
                2 if command == "--png" => PathBuf::from(&args[1]),
                _ => return Err(usage_error()),
            };
            app::run(app::TextureSource::Png(path)).map_err(|error| error.to_string())
        }
        "--adt" if args.len() == 2 => {
            let path = PathBuf::from(&args[1]);
            let image = fileio::adt::load_adt_file(&path)?;
            let description = path.display().to_string();
            report_image(&description, &image);
            app::run(app::TextureSource::Rgba {
                width: image.width(),
                height: image.height(),
                pixels: image.to_rgba(),
                description,
            })
            .map_err(|error| error.to_string())
        }
        "--bin" if args.len() == 3 => {
            let path = PathBuf::from(&args[1]);
            let index = parse_index(&args[2])?;
            let mut archive = fileio::bin::BinArchive::open(&path)?;
            let entry = archive.entry(index)?;
            let image = archive.decode_adt_entry(index)?;
            let description = format!(
                "{} entry {index}/{} (offset {}, {} compressed bytes)",
                path.display(),
                archive.entry_count(),
                entry.offset,
                entry.length
            );
            report_image(&description, &image);
            app::run(app::TextureSource::Rgba {
                width: image.width(),
                height: image.height(),
                pixels: image.to_rgba(),
                description,
            })
            .map_err(|error| error.to_string())
        }
        "--decode-adt" if args.len() == 3 => {
            let input = PathBuf::from(&args[1]);
            let output = PathBuf::from(&args[2]);
            let image = fileio::adt::load_adt_file(&input)?;
            image.write_png(&output)?;
            report_image(&input.display().to_string(), &image);
            println!("Diagnostic PNG written to {}", output.display());
            Ok(())
        }
        "--decode-bin" if args.len() == 4 => {
            let input = PathBuf::from(&args[1]);
            let index = parse_index(&args[2])?;
            let output = PathBuf::from(&args[3]);
            let mut archive = fileio::bin::BinArchive::open(&input)?;
            let entry = archive.entry(index)?;
            let image = archive.decode_adt_entry(index)?;
            image.write_png(&output)?;
            println!(
                "Decoded {} entry {index}/{}: offset {}, {} compressed bytes",
                archive.path().display(),
                archive.entry_count(),
                entry.offset,
                entry.length
            );
            report_image(&input.display().to_string(), &image);
            println!("Diagnostic PNG written to {}", output.display());
            Ok(())
        }
        "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => Err(usage_error()),
    }
}

fn parse_index(value: &OsString) -> Result<usize, String> {
    value
        .to_str()
        .ok_or_else(|| "BIN entry index is not valid UTF-8".to_owned())?
        .parse::<usize>()
        .map_err(|error| format!("invalid BIN entry index {:?}: {error}", value))
}

fn report_image(source: &str, image: &fileio::adt::AdtImage) {
    println!(
        "Decoded ADT from {source}: {}×{} pixels, {} decompressed bytes, {} image words",
        image.width(),
        image.height(),
        image.raw_data().len(),
        image.pixels().len()
    );
}

fn usage_error() -> String {
    "invalid arguments; run with --help for usage".to_owned()
}

fn print_usage() {
    println!(
        "OpenBiohazard2-Rust\n\
         Usage:\n\
         cargo run --release -- [--png [PNG]]\n\
         cargo run --release -- --adt INPUT.ADT\n\
         cargo run --release -- --bin INPUT.BIN ENTRY_INDEX\n\
         cargo run --release -- --decode-adt INPUT.ADT OUTPUT.PNG\n\
         cargo run --release -- --decode-bin INPUT.BIN ENTRY_INDEX OUTPUT.PNG\n\n\
         With no arguments, the SDL2 window uses assets/test_texture.png.\n\
         ADT diagnostic output can be placed under target/diagnostics/ to keep it ignored by Git."
    );
}

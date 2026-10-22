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
        "--rdt" if (2..=4).contains(&args.len()) => {
            let rdt_path = PathBuf::from(&args[1]);
            let rdt = fileio::rdt::RdtFile::open(&rdt_path)?;
            print_rdt(&rdt);
            if args.len() == 2 {
                return Ok(());
            }

            let camera_index = parse_index(&args[2])?;
            let bin_path = args
                .get(3)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("data/Common/bin/roomcut.bin"));
            let (image_index, entry, image) = load_rdt_camera_image(&rdt, camera_index, &bin_path)?;
            let camera = rdt.camera(camera_index)?;
            let description = format!(
                "{} camera {camera_index} -> {} entry {image_index} (offset {}, {} compressed bytes)",
                rdt_path.display(),
                bin_path.display(),
                entry.offset,
                entry.length
            );
            println!(
                "Selected camera {camera_index}: from={:?}, to={:?}, FOV={:.6}°, masks={}",
                camera.from,
                camera.to,
                camera.fov_degrees,
                camera
                    .mask_count
                    .map_or_else(|| "none".to_owned(), |count| count.to_string())
            );
            println!("Go roomcut mapping: image entry {image_index}");
            report_image(&description, &image);
            app::run(app::TextureSource::Rgba {
                width: image.width(),
                height: image.height(),
                pixels: image.to_rgba(),
                description,
            })
            .map_err(|error| error.to_string())
        }
        "--decode-rdt" if (4..=5).contains(&args.len()) => {
            let rdt_path = PathBuf::from(&args[1]);
            let camera_index = parse_index(&args[2])?;
            let output = PathBuf::from(&args[3]);
            let bin_path = args
                .get(4)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("data/Common/bin/roomcut.bin"));
            let rdt = fileio::rdt::RdtFile::open(&rdt_path)?;
            let (image_index, entry, image) = load_rdt_camera_image(&rdt, camera_index, &bin_path)?;
            image.write_png(&output)?;
            println!(
                "Decoded {} camera {camera_index} via roomcut entry {image_index}: offset {}, {} compressed bytes",
                rdt_path.display(),
                entry.offset,
                entry.length
            );
            report_image(&bin_path.display().to_string(), &image);
            println!("Diagnostic PNG written to {}", output.display());
            Ok(())
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
        .ok_or_else(|| "index is not valid UTF-8".to_owned())?
        .parse::<usize>()
        .map_err(|error| format!("invalid index {:?}: {error}", value))
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

fn print_rdt(rdt: &fileio::rdt::RdtFile) {
    let header = rdt.header;
    println!(
        "RDT {}: sprites={}, cameras={}, models={}, items={}, doors={}, rooms={}, reverb={}, sprite_max={}",
        rdt.path().display(),
        header.num_sprites,
        header.num_cameras,
        header.num_models,
        header.num_items,
        header.num_doors,
        header.num_rooms,
        header.num_reverb,
        header.sprite_max
    );
    if let Some(identity) = rdt.identity {
        println!(
            "Room identity from filename: stage={}, room=0x{:02x}, player={}",
            identity.stage, identity.room, identity.player
        );
    } else {
        println!(
            "Room identity unavailable: filename does not match Go's ROOM%01d%02x%01d.RDT format"
        );
    }

    for (index, camera) in rdt.cameras.iter().enumerate() {
        let image_index = rdt
            .identity
            .and_then(|identity| identity.background_image_index(index).ok());
        let image_mapping = image_index.map_or_else(
            || "roomcut entry=unavailable".to_owned(),
            |slot| format!("roomcut entry={slot}"),
        );
        let mask_description = match (camera.mask_offset, camera.mask_count) {
            (Some(offset), Some(count)) => format!("{count} (PRI @ {offset})"),
            _ => "none".to_owned(),
        };
        println!(
            "camera {index}: flag=0x{:04x}, distance={}, from={:?}, to={:?}, FOV={:.6}°, masks={mask_description}, {image_mapping}",
            camera.flag,
            camera.distance_to_screen,
            camera.from,
            camera.to,
            camera.fov_degrees
        );
    }
}

fn load_rdt_camera_image(
    rdt: &fileio::rdt::RdtFile,
    camera_index: usize,
    bin_path: &std::path::Path,
) -> Result<(usize, fileio::bin::BinEntry, fileio::adt::AdtImage), String> {
    rdt.camera(camera_index)?;
    let identity = rdt.identity.ok_or_else(|| {
        format!(
            "cannot map camera to roomcut entry: RDT filename {} does not match Go's ROOM%01d%02x%01d.RDT format",
            rdt.path().display()
        )
    })?;
    let image_index = identity.background_image_index(camera_index)?;
    let mut archive = fileio::bin::BinArchive::open(bin_path)?;
    let entry = archive.entry(image_index).map_err(|error| {
        format!(
            "RDT {} camera {camera_index} maps to roomcut entry {image_index}, but {}: {error}",
            rdt.path().display(),
            bin_path.display()
        )
    })?;
    let image = archive.decode_adt_entry(image_index)?;
    Ok((image_index, entry, image))
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
         cargo run --release -- --rdt INPUT.RDT [CAMERA_INDEX [ROOMCUT.BIN]]\n\
         cargo run --release -- --decode-rdt INPUT.RDT CAMERA_INDEX OUTPUT.PNG [ROOMCUT.BIN]\n\
         cargo run --release -- --decode-adt INPUT.ADT OUTPUT.PNG\n\
         cargo run --release -- --decode-bin INPUT.BIN ENTRY_INDEX OUTPUT.PNG\n\n\
         With no arguments, the SDL2 window uses assets/test_texture.png.\n\
         --rdt without a camera lists metadata; with a camera it renders the Go-mapped roomcut image.\n\
         Diagnostic PNG output can be placed under target/diagnostics/ to keep it ignored by Git."
    );
}

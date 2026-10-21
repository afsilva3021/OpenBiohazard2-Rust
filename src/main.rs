mod app;
mod graphics;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("OpenBiohazard2-Rust: {error}");
        std::process::exit(1);
    }
}

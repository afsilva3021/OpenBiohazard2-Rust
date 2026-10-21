use std::{fs::File, io::BufReader, path::Path};

pub struct Texture {
    id: gl::types::GLuint,
    width: u32,
    height: u32,
}

impl Texture {
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let file = File::open(path)
            .map_err(|error| format!("failed to open PNG {}: {error}", path.display()))?;
        let mut decoder = png::Decoder::new(BufReader::new(file));
        decoder.set_transformations(
            png::Transformations::EXPAND
                | png::Transformations::STRIP_16
                | png::Transformations::ALPHA,
        );
        decoder.set_ignore_text_chunk(true);

        let mut reader = decoder
            .read_info()
            .map_err(|error| format!("failed to read PNG {}: {error}", path.display()))?;
        let (width, height) = (reader.info().width, reader.info().height);
        validate_dimensions(path, width, height)?;

        let rgba_size = pixel_count(width, height)?
            .checked_mul(4)
            .ok_or_else(|| format!("PNG {} is too large", path.display()))?;
        if rgba_size > isize::MAX as usize {
            return Err(format!("PNG {} is too large for OpenGL", path.display()));
        }

        // Reserve the final RGBA size once. The decoder writes into this buffer;
        // grayscale-alpha input is expanded in place below.
        let decoded_size = reader.output_buffer_size();
        if decoded_size > rgba_size {
            return Err(format!(
                "PNG decoder returned an invalid output size for {}",
                path.display()
            ));
        }
        let mut pixels = Vec::with_capacity(rgba_size);
        pixels.resize(decoded_size, 0);
        let frame = reader
            .next_frame(&mut pixels)
            .map_err(|error| format!("failed to decode PNG {}: {error}", path.display()))?;

        match (frame.color_type, frame.bit_depth) {
            (png::ColorType::Rgba, png::BitDepth::Eight) => {
                if frame.buffer_size() != rgba_size {
                    return Err(format!(
                        "PNG decoder returned an unexpected RGBA buffer size for {}",
                        path.display()
                    ));
                }
                pixels.truncate(rgba_size);
            }
            (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => {
                let source_size = pixel_count(width, height)?
                    .checked_mul(2)
                    .ok_or_else(|| format!("PNG {} is too large", path.display()))?;
                if frame.buffer_size() != source_size {
                    return Err(format!(
                        "PNG decoder returned an unexpected grayscale buffer size for {}",
                        path.display()
                    ));
                }

                pixels.truncate(source_size);
                pixels.resize(rgba_size, 0);
                for pixel_index in (0..pixel_count(width, height)?).rev() {
                    let source = pixel_index * 2;
                    let destination = pixel_index * 4;
                    let gray = pixels[source];
                    let alpha = pixels[source + 1];
                    pixels[destination..destination + 4]
                        .copy_from_slice(&[gray, gray, gray, alpha]);
                }
            }
            (color_type, bit_depth) => {
                return Err(format!(
                    "unsupported PNG output format for {}: {color_type:?}, {bit_depth:?}",
                    path.display()
                ));
            }
        }

        upload_rgba(path, width, height, &pixels)
    }

    pub fn id(&self) -> gl::types::GLuint {
        self.id
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        if self.id != 0 {
            unsafe { gl::DeleteTextures(1, &self.id) };
        }
    }
}

fn validate_dimensions(path: &Path, width: u32, height: u32) -> Result<(), String> {
    let mut max_texture_size = 0;
    unsafe { gl::GetIntegerv(gl::MAX_TEXTURE_SIZE, &mut max_texture_size) };
    if max_texture_size <= 0 {
        return Err("OpenGL ES did not report a valid maximum texture size".to_owned());
    }
    if width == 0
        || height == 0
        || width > max_texture_size as u32
        || height > max_texture_size as u32
    {
        return Err(format!(
            "PNG {} dimensions {width}×{height} exceed the supported texture size {max_texture_size}",
            path.display()
        ));
    }
    Ok(())
}

fn pixel_count(width: u32, height: u32) -> Result<usize, String> {
    (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "PNG dimensions overflow the address space".to_owned())
}

fn upload_rgba(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<Texture, String> {
    let mut id = 0;
    unsafe { gl::GenTextures(1, &mut id) };
    if id == 0 {
        return Err("glGenTextures returned texture 0".to_owned());
    }

    let texture = Texture { id, width, height };
    unsafe {
        gl::BindTexture(gl::TEXTURE_2D, id);
        gl::TexParameteri(
            gl::TEXTURE_2D,
            gl::TEXTURE_MIN_FILTER,
            gl::LINEAR as gl::types::GLint,
        );
        gl::TexParameteri(
            gl::TEXTURE_2D,
            gl::TEXTURE_MAG_FILTER,
            gl::LINEAR as gl::types::GLint,
        );
        // Clamp-to-edge plus linear filtering supports non-power-of-two images
        // on OpenGL ES 2.0 without mipmaps.
        gl::TexParameteri(
            gl::TEXTURE_2D,
            gl::TEXTURE_WRAP_S,
            gl::CLAMP_TO_EDGE as gl::types::GLint,
        );
        gl::TexParameteri(
            gl::TEXTURE_2D,
            gl::TEXTURE_WRAP_T,
            gl::CLAMP_TO_EDGE as gl::types::GLint,
        );
        gl::PixelStorei(gl::UNPACK_ALIGNMENT, 4);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA as gl::types::GLint,
            width as gl::types::GLsizei,
            height as gl::types::GLsizei,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            pixels.as_ptr().cast(),
        );
    }

    let error = unsafe { gl::GetError() };
    if error != gl::NO_ERROR {
        return Err(format!(
            "OpenGL ES failed to upload PNG {} (error 0x{error:04x})",
            path.display()
        ));
    }

    println!("Loaded PNG texture: {} ({width}×{height})", path.display());
    Ok(texture)
}

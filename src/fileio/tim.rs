//! PlayStation TIM textures (4/8-bit palettized), migrated from Go `fileio/tim.go`.
//!
//! Go keeps the palette-expanded 16-bit A1B5G5R5 words and uploads them with
//! `UNSIGNED_SHORT_1_5_5_5_REV`. GLES 2.0 has no such upload format, so Rust
//! keeps the same words and converts them to RGBA8 before upload.

const TIM_MAGIC: u32 = 16;
const TIM_BPP_4: u32 = 8;
const TIM_BPP_8: u32 = 9;
const TIM_HEADER_SIZE: usize = 20;
const TIM_IMAGE_HEADER_SIZE: usize = 12;
const MAX_TIM_DIMENSION: usize = 4096;

#[derive(Debug, Clone)]
pub struct TimImage {
    width: usize,
    height: usize,
    palette_count: usize,
    /// Row-major A1B5G5R5 words after palette lookup.
    pixels: Vec<u16>,
    /// Bytes consumed by header, palettes and pixel data (Go `NumBytes`).
    byte_length: usize,
}

impl TimImage {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn palette_count(&self) -> usize {
        self.palette_count
    }

    pub fn pixels(&self) -> &[u16] {
        &self.pixels
    }

    pub fn byte_length(&self) -> usize {
        self.byte_length
    }

    /// Runtime conversion used by Go `resource.convert16BitColor`: every
    /// 5-bit channel is multiplied by 8 and only the all-zero word is
    /// transparent.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for &pixel in &self.pixels {
            rgba.extend_from_slice(&a1b5g5r5_to_rgba(pixel));
        }
        rgba
    }
}

pub fn a1b5g5r5_to_rgba(pixel: u16) -> [u8; 4] {
    let red = (pixel & 0x1f) as u8 * 8;
    let green = ((pixel >> 5) & 0x1f) as u8 * 8;
    let blue = ((pixel >> 10) & 0x1f) as u8 * 8;
    let alpha = if pixel == 0 { 0 } else { 255 };
    [red, green, blue, alpha]
}

pub fn parse_tim(bytes: &[u8]) -> Result<TimImage, String> {
    if bytes.len() < TIM_HEADER_SIZE {
        return Err("TIM is shorter than its 20-byte header".to_owned());
    }
    let magic = read_u32(bytes, 0)?;
    if magic != TIM_MAGIC {
        return Err(format!("TIM magic is {magic}; expected {TIM_MAGIC}"));
    }
    let bpp = read_u32(bytes, 4)?;
    let color_count = usize::from(read_u16(bytes, 16)?);
    let palette_count = usize::from(read_u16(bytes, 18)?);
    let (expected_colors, pixels_per_unit) = match bpp {
        TIM_BPP_4 => (16, 4),
        TIM_BPP_8 => (256, 2),
        other => {
            return Err(format!(
                "TIM BPP code {other} is not supported (Go supports 4/8-bit)"
            ))
        }
    };
    if color_count != expected_colors {
        return Err(format!(
            "{}-bit TIM declares {color_count} palette colors; expected {expected_colors}",
            if bpp == TIM_BPP_4 { 4 } else { 8 }
        ));
    }
    if palette_count == 0 {
        return Err("TIM has no palettes".to_owned());
    }

    let palette_bytes = color_count * palette_count * 2;
    let palette_end = TIM_HEADER_SIZE + palette_bytes;
    let mut palettes = Vec::with_capacity(palette_count);
    for palette in 0..palette_count {
        let mut colors = Vec::with_capacity(color_count);
        for color in 0..color_count {
            colors.push(read_u16(
                bytes,
                TIM_HEADER_SIZE + (palette * color_count + color) * 2,
            )?);
        }
        palettes.push(colors);
    }

    let image_header = palette_end;
    if bytes.len() < image_header + TIM_IMAGE_HEADER_SIZE {
        return Err("TIM is truncated before its image header".to_owned());
    }
    let width = usize::from(read_u16(bytes, image_header + 8)?) * pixels_per_unit;
    let height = usize::from(read_u16(bytes, image_header + 10)?);
    if width == 0 || height == 0 || width > MAX_TIM_DIMENSION || height > MAX_TIM_DIMENSION {
        return Err(format!("TIM dimensions {width}×{height} are invalid"));
    }
    if width % palette_count != 0 {
        return Err(format!(
            "TIM width {width} is not divisible by its {palette_count} palettes"
        ));
    }

    let pixel_count = width * height;
    let data_start = image_header + TIM_IMAGE_HEADER_SIZE;
    let data_length = if bpp == TIM_BPP_4 {
        pixel_count / 2
    } else {
        pixel_count
    };
    let data = bytes
        .get(data_start..data_start + data_length)
        .ok_or_else(|| {
            format!("TIM pixel data needs {data_length} bytes after offset {data_start}")
        })?;

    // Go selects the palette by horizontal band: the image is split into
    // `palette_count` equally wide columns, one palette each.
    let band_width = width / palette_count;
    let mut pixels = vec![0_u16; pixel_count];
    for (index, pixel) in pixels.iter_mut().enumerate() {
        // For 4-bit images Go picks one palette per byte, from the even
        // (low-nibble) pixel's column; keep that exact choice.
        let x = if bpp == TIM_BPP_4 {
            (index & !1) % width
        } else {
            index % width
        };
        let palette = &palettes[x / band_width];
        let color_index = if bpp == TIM_BPP_4 {
            let byte = data[index / 2];
            if index % 2 == 0 {
                byte & 0x0f
            } else {
                byte >> 4
            }
        } else {
            data[index]
        };
        *pixel = palette[usize::from(color_index)];
    }

    Ok(TimImage {
        width,
        height,
        palette_count,
        pixels,
        byte_length: 32 + palette_bytes + data_length,
    })
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, String> {
    bytes
        .get(at..at + 2)
        .map(|field| u16::from_le_bytes([field[0], field[1]]))
        .ok_or_else(|| format!("truncated TIM field at byte {at}"))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|field| u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
        .ok_or_else(|| format!("truncated TIM field at byte {at}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{a1b5g5r5_to_rgba, parse_tim};

    /// Builds a 4-bit TIM with `palettes` palettes. Palette p, color c has the
    /// value `(p << 8) | c` so lookups are easy to verify.
    pub(crate) fn tim_4bpp(width_units: u16, height: u16, palettes: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(&palettes.to_le_bytes());
        for palette in 0..palettes {
            for color in 0..16_u16 {
                bytes.extend_from_slice(&((palette << 8) | color).to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&width_units.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn expands_4bpp_nibbles_low_first_with_banded_palettes() {
        // width 2 units * 4 = 8 pixels, 1 row, 2 palettes -> 4-pixel bands.
        let tim = parse_tim(&tim_4bpp(2, 1, 2, &[0x21, 0x43, 0x65, 0x87])).expect("valid TIM");
        assert_eq!((tim.width(), tim.height(), tim.palette_count()), (8, 1, 2));
        assert_eq!(
            tim.pixels(),
            [0x001, 0x002, 0x003, 0x004, 0x105, 0x106, 0x107, 0x108]
        );
    }

    #[test]
    fn rejects_bad_magic_and_truncated_pixels() {
        let mut bytes = tim_4bpp(1, 1, 1, &[0, 0]);
        bytes[0] = 15;
        assert!(parse_tim(&bytes).is_err());
        assert!(parse_tim(&tim_4bpp(1, 2, 1, &[0, 0])).is_err());
    }

    #[test]
    fn converts_runtime_colors_and_zero_transparency() {
        assert_eq!(a1b5g5r5_to_rgba(0), [0, 0, 0, 0]);
        assert_eq!(a1b5g5r5_to_rgba(0x7fff), [248, 248, 248, 255]);
        assert_eq!(a1b5g5r5_to_rgba(0x001f), [248, 0, 0, 255]);
    }
}

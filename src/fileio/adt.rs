use std::{
    fs::{self, File},
    io::{BufWriter, Read},
    path::Path,
};

use crate::fileio::bitreader::BitReader;

pub const ADT_IMAGE_WIDTH: u32 = 320;
pub const ADT_IMAGE_HEIGHT: u32 = 240;
const PACKED_WORDS: usize = 320 * 256;
const PACKED_IMAGE_BYTES: usize = PACKED_WORDS * 2;
const WINDOW_SIZE: usize = 16 * 1024;
const MAX_COMPRESSED_BYTES: usize = 16 * 1024 * 1024;
const MAX_DECOMPRESSED_BYTES: usize = 4 * 1024 * 1024;
const MAX_BLOCKS: usize = 4096;

#[derive(Debug)]
pub struct AdtImage {
    width: u32,
    height: u32,
    raw_data: Vec<u8>,
    pixels: Vec<u16>,
}

impl AdtImage {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Decompressed ADT payload, including any data following the background.
    pub fn raw_data(&self) -> &[u8] {
        &self.raw_data
    }

    /// Reconstructed 320×240 image in row-major A1B5G5R5 source-word order.
    pub fn pixels(&self) -> &[u16] {
        &self.pixels
    }

    /// Converts the packed words as the Go runtime does in
    /// `resource/image16bit.go`: 5-bit channels are multiplied by 8, and only
    /// an all-zero word is transparent.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for &pixel in &self.pixels {
            let red = (pixel & 0x1f) as u8 * 8;
            let green = ((pixel >> 5) & 0x1f) as u8 * 8;
            let blue = ((pixel >> 10) & 0x1f) as u8 * 8;
            let alpha = if pixel == 0 { 0 } else { 255 };
            rgba.extend_from_slice(&[red, green, blue, alpha]);
        }
        rgba
    }

    /// Preserves the legacy Go `ADTOutput.ConvertToPNG` channel conversion for
    /// pixel-for-pixel diagnostic comparisons. Its signed 5-bit ParseInt
    /// clamps channel values above 15; runtime rendering uses `to_rgba`.
    fn to_diagnostic_rgba(&self) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for &pixel in &self.pixels {
            let red = ((pixel & 0x1f).min(15) as u8) * 8;
            let green = (((pixel >> 5) & 0x1f).min(15) as u8) * 8;
            let blue = (((pixel >> 10) & 0x1f).min(15) as u8) * 8;
            rgba.extend_from_slice(&[red, green, blue, 255]);
        }
        rgba
    }

    pub fn write_png(&self, output_path: impl AsRef<Path>) -> Result<(), String> {
        let output_path = output_path.as_ref();
        if let Some(parent) = output_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create diagnostic directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let file = File::create(output_path).map_err(|error| {
            format!(
                "failed to create diagnostic PNG {}: {error}",
                output_path.display()
            )
        })?;
        let mut encoder = png::Encoder::new(BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| {
            format!(
                "failed to write PNG header {}: {error}",
                output_path.display()
            )
        })?;
        writer
            .write_image_data(&self.to_diagnostic_rgba())
            .map_err(|error| {
                format!(
                    "failed to encode diagnostic PNG {}: {error}",
                    output_path.display()
                )
            })
    }
}

pub fn load_adt_file(path: impl AsRef<Path>) -> Result<AdtImage, String> {
    let path = path.as_ref();
    let file = File::open(path)
        .map_err(|error| format!("failed to open ADT {}: {error}", path.display()))?;
    decode_adt_stream(file).map_err(|error| format!("{}: {}", path.display(), error))
}

pub fn decode_adt_stream(input: impl Read) -> Result<AdtImage, String> {
    let mut compressed = Vec::new();
    input
        .take((MAX_COMPRESSED_BYTES + 1) as u64)
        .read_to_end(&mut compressed)
        .map_err(|error| format!("failed to read ADT stream: {error}"))?;
    if compressed.len() > MAX_COMPRESSED_BYTES {
        return Err(format!(
            "ADT stream exceeds the {} MiB input limit",
            MAX_COMPRESSED_BYTES / (1024 * 1024)
        ));
    }
    decode_adt_bytes(&compressed)
}

pub fn decode_adt_bytes(compressed: &[u8]) -> Result<AdtImage, String> {
    if compressed.len() < 6 {
        return Err("ADT stream is truncated before its first block".to_owned());
    }
    if compressed.len() > MAX_COMPRESSED_BYTES {
        return Err(format!(
            "ADT stream is larger than the {} MiB input limit",
            MAX_COMPRESSED_BYTES / (1024 * 1024)
        ));
    }

    let raw_data = unpack_adt(compressed)?;
    if raw_data.len() < PACKED_IMAGE_BYTES {
        return Err(format!(
            "ADT payload has {} bytes; at least {PACKED_IMAGE_BYTES} are required for a 320×240 background",
            raw_data.len()
        ));
    }
    if raw_data.len() % 2 != 0 {
        return Err(format!(
            "ADT payload has an odd byte count ({}) for 16-bit pixels",
            raw_data.len()
        ));
    }

    let pixels = reconstruct_image(&raw_data)?;
    Ok(AdtImage {
        width: ADT_IMAGE_WIDTH,
        height: ADT_IMAGE_HEIGHT,
        raw_data,
        pixels,
    })
}

fn unpack_adt(compressed: &[u8]) -> Result<Vec<u8>, String> {
    // The original reader skips the first four bytes and then reads block
    // lengths from the bitstream, including when a block begins mid-byte.
    let mut bits = BitReader::new(&compressed[4..]);
    let mut output = Vec::with_capacity(PACKED_IMAGE_BYTES);
    let mut window = [0_u8; WINDOW_SIZE];
    let mut window_position = 0_usize;
    let mut block_count = 0_usize;

    loop {
        let low = bits.read_bits(8).map_err(bit_error)? as u16;
        let high = bits.read_bits(8).map_err(bit_error)? as u16;
        let block_length = low | (high << 8);
        if block_length == 0 {
            return Ok(output);
        }
        block_count += 1;
        if block_count > MAX_BLOCKS {
            return Err(format!("ADT contains more than {MAX_BLOCKS} blocks"));
        }

        let (literal_length_tree, distance_tree) = initialize_unpack_block(&mut bits)?;
        for _ in 0..block_length {
            let code = literal_length_tree
                .read_symbol(&mut bits)
                .map_err(bit_error)?;
            if code < 256 {
                append_byte(code as u8, &mut output, &mut window, &mut window_position)?;
                continue;
            }

            let copy_length = code
                .checked_sub(0xfd)
                .ok_or_else(|| format!("invalid ADT back-reference code {code}"))?;
            if copy_length == 0 {
                return Err("ADT back-reference has zero length".to_owned());
            }
            let distance_code = distance_tree.read_symbol(&mut bits).map_err(bit_error)?;
            let distance = if distance_code == 0 {
                0_usize
            } else {
                let suffix_bits = distance_code - 1;
                let suffix = bits.read_bits(suffix_bits as u32).map_err(bit_error)?;
                (1_usize << suffix_bits) | suffix as usize
            };

            let new_length = output
                .len()
                .checked_add(copy_length as usize)
                .ok_or_else(|| "ADT decompressed length overflow".to_owned())?;
            if new_length > MAX_DECOMPRESSED_BYTES {
                return Err(format!(
                    "ADT output exceeds the {} MiB safety limit",
                    MAX_DECOMPRESSED_BYTES / (1024 * 1024)
                ));
            }

            let mut source_position =
                window_position.wrapping_sub(distance).wrapping_sub(1) & (WINDOW_SIZE - 1);
            for _ in 0..copy_length {
                let byte = window[source_position];
                append_byte(byte, &mut output, &mut window, &mut window_position)?;
                source_position = (source_position + 1) & (WINDOW_SIZE - 1);
            }
        }
    }
}

fn append_byte(
    byte: u8,
    output: &mut Vec<u8>,
    window: &mut [u8; WINDOW_SIZE],
    window_position: &mut usize,
) -> Result<(), String> {
    if output.len() >= MAX_DECOMPRESSED_BYTES {
        return Err(format!(
            "ADT output exceeds the {} MiB safety limit",
            MAX_DECOMPRESSED_BYTES / (1024 * 1024)
        ));
    }
    output.push(byte);
    window[*window_position] = byte;
    *window_position = (*window_position + 1) & (WINDOW_SIZE - 1);
    Ok(())
}

fn initialize_unpack_block(bits: &mut BitReader<'_>) -> Result<(HuffmanTree, HuffmanTree), String> {
    let first_table = read_delta_length_table(bits, 16)?;
    let mut encoded_lengths = [0_u16; 512];
    let mut mode = bits.read_bit().map_err(bit_error)?;
    let mut filled = 0_usize;

    while filled < encoded_lengths.len() {
        let run_length = bits.read_binary_number().map_err(bit_error)? as usize;
        if run_length == 0 || run_length > encoded_lengths.len() - filled {
            return Err(format!(
                "invalid ADT code-length run {run_length} at position {filled}"
            ));
        }
        if mode == 1 {
            for value in &mut encoded_lengths[filled..filled + run_length] {
                *value = first_table.read_symbol(bits).map_err(bit_error)?;
            }
            mode = 0;
        } else {
            mode = 1;
        }
        filled += run_length;
    }

    let mut previous = 0_u16;
    let mut second_lengths = Vec::with_capacity(encoded_lengths.len());
    for delta in encoded_lengths {
        previous ^= delta;
        if previous > 16 {
            return Err(format!(
                "invalid ADT Huffman code length {previous} in the 512-symbol table"
            ));
        }
        second_lengths.push(previous as u8);
    }
    let second_table = HuffmanTree::from_lengths(&second_lengths)?;
    let third_table = read_delta_length_table(bits, 16)?;
    Ok((second_table, third_table))
}

fn read_delta_length_table(
    bits: &mut BitReader<'_>,
    symbol_count: usize,
) -> Result<HuffmanTree, String> {
    let mut previous = 0_u32;
    let mut lengths = Vec::with_capacity(symbol_count);
    for _ in 0..symbol_count {
        if bits.read_bit().map_err(bit_error)? == 1 {
            previous ^= bits.read_binary_number().map_err(bit_error)?;
        }
        if previous > 16 {
            return Err(format!(
                "invalid ADT Huffman code length {previous} in a {symbol_count}-symbol table"
            ));
        }
        lengths.push(previous as u8);
    }
    HuffmanTree::from_lengths(&lengths)
}

#[derive(Clone, Copy, Default)]
struct HuffmanNode {
    children: [Option<usize>; 2],
    symbol: Option<u16>,
}

struct HuffmanTree {
    nodes: Vec<HuffmanNode>,
}

impl HuffmanTree {
    fn from_lengths(lengths: &[u8]) -> Result<Self, String> {
        if lengths.is_empty() {
            return Err("ADT Huffman table is empty".to_owned());
        }
        let mut counts = [0_u32; 17];
        for &length in lengths {
            if length > 16 {
                return Err(format!("invalid ADT Huffman code length {length}"));
            }
            counts[length as usize] += 1;
        }

        let mut next_code = [0_u32; 17];
        let mut code = 0_u32;
        for bit_length in 1..=16 {
            let capacity = 1_u32 << bit_length;
            if code > capacity || counts[bit_length] > capacity - code {
                return Err("oversubscribed ADT Huffman code lengths".to_owned());
            }
            next_code[bit_length] = code;
            code = (code + counts[bit_length]) << 1;
        }

        let mut tree = Self {
            nodes: vec![HuffmanNode::default()],
        };
        let node_limit = lengths
            .len()
            .checked_mul(2)
            .ok_or_else(|| "ADT Huffman node count overflow".to_owned())?;
        let mut symbols_added = 0_usize;

        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let length = length as usize;
            let symbol_code = next_code[length];
            next_code[length] += 1;
            let mut node_index = 0_usize;

            for bit_offset in 0..length {
                if tree.nodes[node_index].symbol.is_some() {
                    return Err("ADT Huffman code is a prefix of another code".to_owned());
                }
                let branch = ((symbol_code >> (length - bit_offset - 1)) & 1) as usize;
                let child_index = match tree.nodes[node_index].children[branch] {
                    Some(child) => child,
                    None => {
                        if tree.nodes.len() >= node_limit {
                            return Err("ADT Huffman tree exceeds its node limit".to_owned());
                        }
                        let child = tree.nodes.len();
                        tree.nodes.push(HuffmanNode::default());
                        tree.nodes[node_index].children[branch] = Some(child);
                        child
                    }
                };
                node_index = child_index;
            }

            let leaf = &mut tree.nodes[node_index];
            if leaf.symbol.is_some() || leaf.children.iter().any(Option::is_some) {
                return Err("conflicting ADT Huffman codes".to_owned());
            }
            leaf.symbol = Some(symbol as u16);
            symbols_added += 1;
        }

        if symbols_added == 0 {
            return Err("ADT Huffman table has no decodable symbols".to_owned());
        }
        Ok(tree)
    }

    fn read_symbol(
        &self,
        bits: &mut BitReader<'_>,
    ) -> Result<u16, crate::fileio::bitreader::BitstreamError> {
        let mut node_index = 0_usize;
        for _ in 0..16 {
            let branch = bits.read_bit()? as usize;
            let Some(child) = self.nodes[node_index].children[branch] else {
                return Err(crate::fileio::bitreader::BitstreamError::new(
                    "ADT bitstream follows a missing Huffman branch",
                ));
            };
            node_index = child;
            if let Some(symbol) = self.nodes[node_index].symbol {
                return Ok(symbol);
            }
        }
        Err(crate::fileio::bitreader::BitstreamError::new(
            "ADT Huffman code exceeds 16 bits",
        ))
    }
}

fn reconstruct_image(raw_data: &[u8]) -> Result<Vec<u16>, String> {
    let (word_bytes, _) = raw_data[..PACKED_IMAGE_BYTES].as_chunks::<2>();
    let packed: Vec<u16> = word_bytes
        .iter()
        .map(|bytes| u16::from_le_bytes(*bytes))
        .collect();
    if packed.len() != PACKED_WORDS {
        return Err("ADT packed image has an unexpected word count".to_owned());
    }

    let mut image = vec![0_u16; (ADT_IMAGE_WIDTH * ADT_IMAGE_HEIGHT) as usize];

    // The left 256×240 area is stored linearly at the start of the payload.
    for y in 0..240_usize {
        let source = 256 * y;
        let destination = 320 * y;
        image[destination..destination + 256].copy_from_slice(&packed[source..source + 256]);
    }

    // The rightmost 64 columns are stored as two interleaved 64×128 regions.
    for row_pair in 0..64_usize {
        let source_row = 256 + row_pair;
        let upper = source_row * 256;
        let destination_top = (row_pair * 2) * 320 + 256;
        let destination_bottom = destination_top + 320;
        image[destination_top..destination_top + 64].copy_from_slice(&packed[upper..upper + 64]);
        image[destination_bottom..destination_bottom + 64]
            .copy_from_slice(&packed[upper + 128..upper + 192]);
    }
    for row_pair in 0..56_usize {
        let source_row = 256 + row_pair;
        let source = source_row * 256;
        let destination_top = (128 + row_pair * 2) * 320 + 256;
        let destination_bottom = destination_top + 320;
        image[destination_top..destination_top + 64]
            .copy_from_slice(&packed[source + 64..source + 128]);
        image[destination_bottom..destination_bottom + 64]
            .copy_from_slice(&packed[source + 192..source + 256]);
    }

    Ok(image)
}

fn bit_error(error: impl std::fmt::Display) -> String {
    format!("invalid ADT bitstream: {error}")
}

#[cfg(test)]
mod tests {
    use super::{decode_adt_bytes, reconstruct_image, PACKED_IMAGE_BYTES, PACKED_WORDS};

    #[test]
    fn rejects_truncated_streams_without_panicking() {
        assert!(decode_adt_bytes(&[]).is_err());
        assert!(decode_adt_bytes(&[0; 5]).is_err());
        assert!(decode_adt_bytes(&[0; 6]).is_err());
    }

    #[test]
    fn rejects_a_valid_terminator_without_a_complete_image() {
        assert!(decode_adt_bytes(&[0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn reconstructs_go_strip_layout_into_320_by_240_rows() {
        let mut raw = vec![0_u8; PACKED_IMAGE_BYTES];
        for word_index in 0..PACKED_WORDS {
            let word = word_index as u16;
            raw[word_index * 2..word_index * 2 + 2].copy_from_slice(&word.to_le_bytes());
        }
        let image = reconstruct_image(&raw).expect("synthetic packed image should reconstruct");
        assert_eq!(image.len(), 320 * 240);
        assert_eq!(image[0], 0);
        assert_eq!(image[255], 255);
        assert_eq!(image[256], (256 * 256) as u16);
        assert_eq!(image[320], 256);
        assert_eq!(image[320 + 256], (256 * 256 + 128) as u16);
        assert_eq!(image[(128 * 320) + 256], (256 * 256 + 64) as u16);
    }

    #[test]
    fn converts_a1b5g5r5_channels_like_the_go_runtime_renderer() {
        let image = super::AdtImage {
            width: 3,
            height: 1,
            raw_data: Vec::new(),
            pixels: vec![0x7fff, 0x801f, 0],
        };
        assert_eq!(
            image.to_rgba(),
            [248, 248, 248, 255, 248, 0, 0, 255, 0, 0, 0, 0]
        );
    }

    #[test]
    fn matches_go_adt_png_signed_five_bit_channel_clamping() {
        let image = super::AdtImage {
            width: 1,
            height: 1,
            raw_data: Vec::new(),
            pixels: vec![0x7fff],
        };
        assert_eq!(image.to_diagnostic_rgba(), [120, 120, 120, 255]);
    }
}

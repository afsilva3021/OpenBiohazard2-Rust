use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BitstreamError {
    message: String,
}

impl BitstreamError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for BitstreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub(crate) struct BitReader<'a> {
    bytes: &'a [u8],
    bit_position: usize,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            bit_position: 0,
        }
    }

    pub(crate) fn read_bit(&mut self) -> Result<u8, BitstreamError> {
        let byte_index = self.bit_position / 8;
        let bit_in_byte = self.bit_position % 8;
        let byte = self
            .bytes
            .get(byte_index)
            .ok_or_else(|| BitstreamError::new("unexpected end of ADT bitstream"))?;
        let bit = (byte >> (7 - bit_in_byte)) & 1;
        self.bit_position = self
            .bit_position
            .checked_add(1)
            .ok_or_else(|| BitstreamError::new("ADT bit position overflow"))?;
        Ok(bit)
    }

    pub(crate) fn read_bits(&mut self, count: u32) -> Result<u32, BitstreamError> {
        if count > 32 {
            return Err(BitstreamError::new(format!(
                "cannot read {count} bits into a 32-bit value"
            )));
        }

        let mut value = 0_u32;
        for _ in 0..count {
            value = (value << 1) | u32::from(self.read_bit()?);
        }
        Ok(value)
    }

    /// Reads the Go decoder's unary-zero prefix followed by its binary suffix.
    pub(crate) fn read_binary_number(&mut self) -> Result<u32, BitstreamError> {
        let mut zero_count = 0_u32;
        while self.read_bit()? == 0 {
            zero_count += 1;
            if zero_count > 31 {
                return Err(BitstreamError::new(
                    "ADT unary prefix exceeds the supported 32-bit range",
                ));
            }
        }

        let suffix = self.read_bits(zero_count)?;
        (1_u32 << zero_count)
            .checked_add(suffix)
            .ok_or_else(|| BitstreamError::new("ADT binary number overflow"))
    }
}

#[cfg(test)]
mod tests {
    use super::BitReader;

    #[test]
    fn reads_bits_most_significant_first() {
        let mut reader = BitReader::new(&[0b1011_0010]);
        assert_eq!(reader.read_bits(4), Ok(0b1011));
        assert_eq!(reader.read_bits(4), Ok(0b0010));
    }

    #[test]
    fn reports_truncated_bitstreams() {
        let mut reader = BitReader::new(&[]);
        assert!(reader.read_bit().is_err());
    }

    #[test]
    fn decodes_the_go_unary_binary_number() {
        // 001 01: two leading zeroes, terminator one, suffix 01 -> binary 5.
        let mut reader = BitReader::new(&[0b0010_1000]);
        assert_eq!(reader.read_binary_number(), Ok(5));
    }
}

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use super::adt::{decode_adt_stream, AdtImage};

const MAX_BIN_INDEX_BYTES: usize = 4 * 1024 * 1024;
const MAX_BIN_ENTRIES: usize = MAX_BIN_INDEX_BYTES / 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinEntry {
    pub offset: u64,
    pub length: u64,
}

pub struct BinArchive {
    path: PathBuf,
    file: File,
    entries: Vec<Option<BinEntry>>,
}

impl BinArchive {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let mut file = File::open(path)
            .map_err(|error| format!("failed to open BIN {}: {error}", path.display()))?;
        let file_length = file
            .metadata()
            .map_err(|error| format!("failed to inspect BIN {}: {error}", path.display()))?
            .len();
        if file_length < 4 {
            return Err(format!(
                "BIN {} is shorter than its offset header",
                path.display()
            ));
        }

        let mut first_offset_bytes = [0_u8; 4];
        file.read_exact(&mut first_offset_bytes)
            .map_err(|error| format!("failed to read BIN {} header: {error}", path.display()))?;
        let first_offset = u32::from_le_bytes(first_offset_bytes) as usize;
        let table_length = validate_table_length(first_offset, file_length)?;
        let mut table_bytes = vec![0; table_length];
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.read_exact(&mut table_bytes))
            .map_err(|error| {
                format!(
                    "failed to read BIN {} offset table: {error}",
                    path.display()
                )
            })?;
        let entries = parse_offset_table(&table_bytes, file_length)
            .map_err(|error| format!("invalid BIN {} offset table: {error}", path.display()))?;

        Ok(Self {
            path: path.to_owned(),
            file,
            entries,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn entry(&self, index: usize) -> Result<BinEntry, String> {
        entry_at(&self.entries, index)
    }

    pub fn decode_adt_entry(&mut self, index: usize) -> Result<AdtImage, String> {
        let entry = self.entry(index)?;
        self.file
            .seek(SeekFrom::Start(entry.offset))
            .map_err(|error| {
                format!(
                    "failed to seek to BIN entry {index} in {}: {error}",
                    self.path.display()
                )
            })?;
        let limited = (&mut self.file).take(entry.length);
        decode_adt_stream(limited).map_err(|error| {
            format!(
                "failed to decode ADT at BIN entry {index} (offset {}, {} bytes) in {}: {error}",
                entry.offset,
                entry.length,
                self.path.display()
            )
        })
    }
}

fn validate_table_length(first_offset: usize, file_length: u64) -> Result<usize, String> {
    if first_offset < 8 || !first_offset.is_multiple_of(4) {
        return Err(format!(
            "first data offset {first_offset} is not a valid 4-byte offset table size"
        ));
    }
    let entry_count = first_offset / 4;
    if entry_count > MAX_BIN_ENTRIES {
        return Err(format!(
            "BIN offset table has {entry_count} entries, exceeding the limit {MAX_BIN_ENTRIES}"
        ));
    }
    if first_offset as u64 > file_length {
        return Err(format!(
            "BIN offset table ends at {first_offset}, beyond the {file_length}-byte file"
        ));
    }
    Ok(first_offset)
}

fn parse_offset_table(table: &[u8], file_length: u64) -> Result<Vec<Option<BinEntry>>, String> {
    if table.len() < 4 {
        return Err("offset table is truncated before its first offset".to_owned());
    }
    let first_offset = u32::from_le_bytes([table[0], table[1], table[2], table[3]]) as usize;
    let expected_length = validate_table_length(first_offset, file_length)?;
    if table.len() != expected_length {
        return Err(format!(
            "offset table contains {} bytes; expected {expected_length}",
            table.len()
        ));
    }

    let (offset_chunks, _) = table.as_chunks::<4>();
    let offsets: Vec<u64> = offset_chunks
        .iter()
        .map(|chunk| u64::from(u32::from_le_bytes(*chunk)))
        .collect();
    let mut previous = 0_u64;
    for (slot, &offset) in offsets.iter().enumerate() {
        if offset == 0 {
            continue;
        }
        if offset < expected_length as u64 || offset > file_length {
            return Err(format!(
                "entry {slot} offset {offset} is outside the data region {expected_length}..={file_length}"
            ));
        }
        if offset < previous {
            return Err(format!(
                "entry {slot} offset {offset} precedes the previous data offset {previous}"
            ));
        }
        previous = offset;
    }

    let mut entries = vec![None; offsets.len()];
    let mut next_offset = file_length;
    for slot in (0..offsets.len()).rev() {
        let start = offsets[slot];
        if start == 0 {
            continue;
        }
        if next_offset < start {
            return Err(format!(
                "entry {slot} ends at {next_offset}, before its start offset {start}"
            ));
        }
        if next_offset > start {
            entries[slot] = Some(BinEntry {
                offset: start,
                length: next_offset - start,
            });
        }
        next_offset = start;
    }
    Ok(entries)
}

fn entry_at(entries: &[Option<BinEntry>], index: usize) -> Result<BinEntry, String> {
    let Some(entry) = entries.get(index) else {
        return Err(format!(
            "BIN entry index {index} is out of range ({} entries)",
            entries.len()
        ));
    };
    entry.ok_or_else(|| format!("BIN entry index {index} is empty"))
}

#[cfg(test)]
mod tests {
    use super::{entry_at, parse_offset_table, BinEntry};

    #[test]
    fn preserves_empty_slots_and_computes_entry_ranges() {
        let offsets = [16_u32, 32, 0, 48];
        let table: Vec<u8> = offsets
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let entries = parse_offset_table(&table, 64).expect("valid table");
        assert_eq!(
            entries[0],
            Some(BinEntry {
                offset: 16,
                length: 16
            })
        );
        assert_eq!(
            entries[1],
            Some(BinEntry {
                offset: 32,
                length: 16
            })
        );
        assert_eq!(entries[2], None);
        assert_eq!(
            entries[3],
            Some(BinEntry {
                offset: 48,
                length: 16
            })
        );
    }

    #[test]
    fn rejects_truncated_and_out_of_file_offset_tables() {
        assert!(parse_offset_table(&[16, 0, 0], 64).is_err());

        let table = [32_u32, 40, 48, 64, 80, 96, 112, 128]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        assert!(parse_offset_table(&table, 64).is_err());
    }

    #[test]
    fn rejects_invalid_and_empty_entry_indices() {
        let entries = [
            Some(BinEntry {
                offset: 16,
                length: 8,
            }),
            None,
        ];
        assert_eq!(
            entry_at(&entries, 0),
            Ok(BinEntry {
                offset: 16,
                length: 8
            })
        );
        assert!(entry_at(&entries, 1).is_err());
        assert!(entry_at(&entries, 2).is_err());
    }
}

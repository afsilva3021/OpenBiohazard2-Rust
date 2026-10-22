use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use super::{
    adt::AdtImage,
    bin::{BinArchive, BinEntry},
};

const RDT_HEADER_SIZE: usize = 8;
const RDT_OFFSET_COUNT: usize = 23;
const RDT_CORE_SIZE: usize = RDT_HEADER_SIZE + RDT_OFFSET_COUNT * 4;
const RID_CAMERA_SIZE: usize = 32;
const PRI_HEADER_SIZE: usize = 4;
const PRI_RELATIVE_OFFSET_SIZE: usize = 8;
const NO_MASK_OFFSET: u32 = u32::MAX;
const MAX_RDT_BYTES: u64 = 16 * 1024 * 1024;
const ROOMCUT_STAGE_STRIDE: usize = 512;
const ROOMCUT_ROOM_STRIDE: usize = 16;

const RDT_OFFSET_NAMES: [&str; RDT_OFFSET_COUNT] = [
    "room sound",
    "room VAB header",
    "room VAB data",
    "enemy VAB header",
    "enemy VAB data",
    "OTA",
    "collision data",
    "camera positions (RID)",
    "camera switches (RVD)",
    "lights (LIT)",
    "items",
    "floor sound",
    "blocks",
    "language 1",
    "language 2",
    "scroll texture",
    "init script",
    "execute script",
    "sprite animations",
    "sprite animation offsets",
    "sprite image",
    "model image",
    "RBJ",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RdtHeader {
    pub num_sprites: u8,
    pub num_cameras: u8,
    pub num_models: u8,
    pub num_items: u8,
    pub num_doors: u8,
    pub num_rooms: u8,
    pub num_reverb: u8,
    pub sprite_max: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomIdentity {
    pub stage: u8,
    pub room: u8,
    pub player: u8,
}

impl RoomIdentity {
    /// Parses the Go filename pattern `ROOM%01d%02x%01d.RDT`.
    pub fn from_rdt_path(path: &Path) -> Result<Self, String> {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("RDT filename is not valid UTF-8: {}", path.display()))?;
        let bytes = stem.as_bytes();
        if bytes.len() != 8 || !bytes[..4].eq_ignore_ascii_case(b"ROOM") {
            return Err(format!(
                "RDT filename {stem:?} does not match Go format ROOM%01d%02x%01d.RDT"
            ));
        }

        let stage = parse_decimal_digit(bytes[4], "stage", stem)?;
        let room = parse_hex_byte(bytes[5], bytes[6], stem)?;
        let player = parse_decimal_digit(bytes[7], "player", stem)?;
        if stage == 0 {
            return Err(format!("RDT filename {stem:?} has invalid stage 0"));
        }
        if player > 1 {
            return Err(format!(
                "RDT filename {stem:?} has player {player}; expected 0 or 1"
            ));
        }

        Ok(Self {
            stage,
            room,
            player,
        })
    }

    /// Matches Go's GameDef.GetBackgroundImageNumber implementation.
    pub fn background_image_index(self, camera: usize) -> Result<usize, String> {
        if camera >= ROOMCUT_ROOM_STRIDE {
            return Err(format!(
                "camera index {camera} cannot map uniquely: Go's roomcut layout reserves {ROOMCUT_ROOM_STRIDE} entries per room"
            ));
        }

        usize::from(self.stage - 1)
            .checked_mul(ROOMCUT_STAGE_STRIDE)
            .and_then(|stage_base| {
                usize::from(self.room)
                    .checked_mul(ROOMCUT_ROOM_STRIDE)
                    .and_then(|room_base| stage_base.checked_add(room_base))
            })
            .and_then(|room_base| room_base.checked_add(camera))
            .ok_or_else(|| "RDT roomcut image index overflowed".to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RdtCamera {
    pub flag: u16,
    pub distance_to_screen: u16,
    pub from: [i32; 3],
    pub to: [i32; 3],
    pub mask_offset: Option<u32>,
    pub mask_count: Option<u16>,
    pub fov_degrees: f32,
}

pub struct RdtFile {
    path: PathBuf,
    pub header: RdtHeader,
    pub cameras: Vec<RdtCamera>,
    pub identity: Option<RoomIdentity>,
}

pub struct RdtCameraImage {
    pub camera_index: usize,
    pub image_index: usize,
    pub entry: BinEntry,
    pub image: AdtImage,
}

pub struct RdtCameraSession {
    rdt: RdtFile,
    archive: BinArchive,
}

impl RdtCameraSession {
    pub fn open(
        rdt: RdtFile,
        archive_path: impl AsRef<Path>,
        initial_camera: usize,
    ) -> Result<Self, String> {
        // Resolve the camera and its Go-derived slot before opening or seeking the BIN.
        rdt.background_image_index(initial_camera)?;
        let archive = BinArchive::open(archive_path)?;
        Ok(Self { rdt, archive })
    }

    pub fn rdt(&self) -> &RdtFile {
        &self.rdt
    }

    pub fn archive_path(&self) -> &Path {
        self.archive.path()
    }

    pub fn camera_count(&self) -> usize {
        self.rdt.mapped_camera_count()
    }

    pub fn load_camera(&mut self, camera_index: usize) -> Result<RdtCameraImage, String> {
        let image_index = self.rdt.background_image_index(camera_index)?;
        let entry = self.archive.entry(image_index).map_err(|error| {
            format!(
                "RDT {} camera {camera_index} maps to roomcut entry {image_index}, but {}: {error}",
                self.rdt.path.display(),
                self.archive.path().display()
            )
        })?;
        let image = self.archive.decode_adt_entry(image_index)?;
        Ok(RdtCameraImage {
            camera_index,
            image_index,
            entry,
            image,
        })
    }
}

impl RdtFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let mut file = File::open(path)
            .map_err(|error| format!("failed to open RDT {}: {error}", path.display()))?;
        let length = file
            .metadata()
            .map_err(|error| format!("failed to inspect RDT {}: {error}", path.display()))?
            .len();
        if length > MAX_RDT_BYTES {
            return Err(format!(
                "RDT {} is {length} bytes; limit is {MAX_RDT_BYTES}",
                path.display()
            ));
        }
        let capacity = usize::try_from(length)
            .map_err(|_| format!("RDT {} is too large for this platform", path.display()))?;
        let mut bytes = Vec::with_capacity(capacity);
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.take(MAX_RDT_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|error| format!("failed to read RDT {}: {error}", path.display()))?;
        if bytes.len() as u64 > MAX_RDT_BYTES {
            return Err(format!(
                "RDT {} exceeds the {MAX_RDT_BYTES}-byte input limit",
                path.display()
            ));
        }

        Self::parse(path, &bytes)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn camera(&self, index: usize) -> Result<&RdtCamera, String> {
        self.cameras.get(index).ok_or_else(|| {
            format!(
                "RDT camera index {index} is out of range ({} cameras)",
                self.cameras.len()
            )
        })
    }

    pub fn background_image_index(&self, camera: usize) -> Result<usize, String> {
        self.camera(camera)?;
        let identity = self.identity.ok_or_else(|| {
            format!(
                "cannot map camera to roomcut entry: RDT filename {} does not match Go's ROOM%01d%02x%01d.RDT format",
                self.path.display()
            )
        })?;
        identity.background_image_index(camera)
    }

    pub fn mapped_camera_count(&self) -> usize {
        if self.identity.is_none() {
            return 0;
        }
        self.cameras.len().min(ROOMCUT_ROOM_STRIDE)
    }

    fn parse(path: &Path, bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < RDT_CORE_SIZE {
            return Err(format!(
                "RDT {} is truncated: {} bytes, need at least {RDT_CORE_SIZE} for header and offsets",
                path.display(),
                bytes.len()
            ));
        }

        let header = RdtHeader {
            num_sprites: bytes[0],
            num_cameras: bytes[1],
            num_models: bytes[2],
            num_items: bytes[3],
            num_doors: bytes[4],
            num_rooms: bytes[5],
            num_reverb: bytes[6],
            sprite_max: bytes[7],
        };

        let offsets_bytes = &bytes[RDT_HEADER_SIZE..RDT_CORE_SIZE];
        let offsets: Vec<u32> = offsets_bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect();
        for (name, offset) in RDT_OFFSET_NAMES.iter().zip(&offsets) {
            if *offset != 0 && (*offset as usize) < RDT_CORE_SIZE {
                return Err(format!(
                    "RDT {} {name} offset {offset} overlaps its {RDT_CORE_SIZE}-byte core header",
                    path.display()
                ));
            }
            if u64::from(*offset) > bytes.len() as u64 {
                return Err(format!(
                    "RDT {} {name} offset {offset} is beyond the {}-byte file",
                    path.display(),
                    bytes.len()
                ));
            }
        }

        let camera_offset = offsets[7];
        if header.num_cameras > 0 && camera_offset == 0 {
            return Err(format!(
                "RDT {} declares {} cameras but its RID offset is zero",
                path.display(),
                header.num_cameras
            ));
        }
        if header.num_cameras > 0 && (camera_offset as usize) < RDT_CORE_SIZE {
            return Err(format!(
                "RDT {} RID camera offset {camera_offset} overlaps its {RDT_CORE_SIZE}-byte core header",
                path.display()
            ));
        }
        let camera_bytes_len = usize::from(header.num_cameras)
            .checked_mul(RID_CAMERA_SIZE)
            .ok_or_else(|| format!("RDT {} camera table size overflowed", path.display()))?;
        let camera_bytes = if header.num_cameras == 0 {
            &[][..]
        } else {
            checked_slice(bytes, camera_offset, camera_bytes_len, "RID camera table")
                .map_err(|error| format!("RDT {}: {error}", path.display()))?
        };

        let mut cameras = Vec::with_capacity(usize::from(header.num_cameras));
        for (index, record) in camera_bytes
            .as_chunks::<RID_CAMERA_SIZE>()
            .0
            .iter()
            .enumerate()
        {
            let flag = read_u16(record, 0);
            let distance_to_screen = read_u16(record, 2);
            let from = [
                read_i32(record, 4),
                read_i32(record, 8),
                read_i32(record, 12),
            ];
            let to = [
                read_i32(record, 16),
                read_i32(record, 20),
                read_i32(record, 24),
            ];
            let raw_mask_offset = read_u32(record, 28);
            let mask_count = if raw_mask_offset == NO_MASK_OFFSET {
                None
            } else {
                Some(
                    parse_pri_mask_count(bytes, raw_mask_offset).map_err(|error| {
                        format!("RDT {} camera {index}: {error}", path.display())
                    })?,
                )
            };
            cameras.push(RdtCamera {
                flag,
                distance_to_screen,
                from,
                to,
                mask_offset: (raw_mask_offset != NO_MASK_OFFSET).then_some(raw_mask_offset),
                mask_count,
                fov_degrees: calculate_fov_degrees(distance_to_screen >> 7),
            });
        }

        let identity = RoomIdentity::from_rdt_path(path).ok();
        Ok(Self {
            path: path.to_owned(),
            header,
            cameras,
            identity,
        })
    }
}

fn parse_pri_mask_count(bytes: &[u8], offset: u32) -> Result<u16, String> {
    if (offset as usize) < RDT_CORE_SIZE {
        return Err(format!(
            "PRI mask offset {offset} overlaps the {RDT_CORE_SIZE}-byte RDT core header"
        ));
    }
    let header = checked_slice(bytes, offset, PRI_HEADER_SIZE, "PRI mask header")?;
    let offset_count = u16::from_le_bytes([header[0], header[1]]);
    let mask_count = u16::from_le_bytes([header[2], header[3]]);
    if offset_count == u16::MAX || mask_count == u16::MAX {
        return Ok(0);
    }

    let relative_table_size = usize::from(offset_count)
        .checked_mul(PRI_RELATIVE_OFFSET_SIZE)
        .and_then(|size| size.checked_add(PRI_HEADER_SIZE))
        .ok_or_else(|| "PRI relative-offset table size overflowed".to_owned())?;
    let table = checked_slice(
        bytes,
        offset,
        relative_table_size,
        "PRI relative-offset table",
    )?;
    let listed_masks = table[PRI_HEADER_SIZE..]
        .as_chunks::<PRI_RELATIVE_OFFSET_SIZE>()
        .0
        .iter()
        .try_fold(0_u32, |total, record| {
            total.checked_add(u32::from(u16::from_le_bytes([record[0], record[1]])))
        })
        .ok_or_else(|| "PRI mask count overflowed".to_owned())?;
    if listed_masks > u32::from(mask_count) {
        return Err(format!(
            "PRI relative offsets describe {listed_masks} masks, exceeding header count {mask_count}"
        ));
    }
    Ok(mask_count)
}

fn checked_slice<'a>(
    bytes: &'a [u8],
    offset: u32,
    length: usize,
    structure: &str,
) -> Result<&'a [u8], String> {
    let start = usize::try_from(offset)
        .map_err(|_| format!("{structure} offset {offset} does not fit in memory"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| format!("{structure} bounds overflow"))?;
    bytes.get(start..end).ok_or_else(|| {
        format!(
            "{structure} at offset {offset} needs {length} bytes, outside the {}-byte file",
            bytes.len()
        )
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn calculate_fov_degrees(distance_to_screen: u16) -> f32 {
    if distance_to_screen == 0 {
        return 180.0;
    }
    let radians = (2.0 * (120.0 / f64::from(distance_to_screen)).atan()) as f32;
    radians * 180.0 / std::f32::consts::PI
}

fn parse_decimal_digit(value: u8, field: &str, stem: &str) -> Result<u8, String> {
    if value.is_ascii_digit() {
        Ok(value - b'0')
    } else {
        Err(format!(
            "RDT filename {stem:?} has a non-decimal {field} field"
        ))
    }
}

fn parse_hex_byte(high: u8, low: u8, stem: &str) -> Result<u8, String> {
    let high = hex_digit(high)
        .ok_or_else(|| format!("RDT filename {stem:?} has an invalid hexadecimal room field"))?;
    let low = hex_digit(low)
        .ok_or_else(|| format!("RDT filename {stem:?} has an invalid hexadecimal room field"))?;
    Ok((high << 4) | low)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        calculate_fov_degrees, parse_pri_mask_count, RdtCameraSession, RdtFile, RoomIdentity,
        RDT_CORE_SIZE, RID_CAMERA_SIZE,
    };

    fn minimal_rdt(camera_count: u8, rid_offset: u32) -> Vec<u8> {
        let mut bytes = vec![0; RDT_CORE_SIZE];
        bytes[1] = camera_count;
        bytes[8 + 7 * 4..8 + 8 * 4].copy_from_slice(&rid_offset.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_room_identity_and_go_roomcut_mapping() {
        let identity =
            RoomIdentity::from_rdt_path(Path::new("ROOM1000.RDT")).expect("valid Go room filename");
        assert_eq!(identity.stage, 1);
        assert_eq!(identity.room, 0);
        assert_eq!(identity.player, 0);
        assert_eq!(identity.background_image_index(8), Ok(8));

        let later_room = RoomIdentity {
            stage: 2,
            room: 3,
            player: 1,
        };
        assert_eq!(later_room.background_image_index(4), Ok(564));
        assert!(identity.background_image_index(16).is_err());
    }

    #[test]
    fn leaves_only_roomcut_cameras_with_unambiguous_slots_selectable() {
        let mut bytes = minimal_rdt(17, RDT_CORE_SIZE as u32);
        bytes.resize(RDT_CORE_SIZE + 17 * RID_CAMERA_SIZE, 0);
        for camera in 0..17 {
            let mask_offset = RDT_CORE_SIZE + camera * RID_CAMERA_SIZE + 28;
            bytes[mask_offset..mask_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        let rdt = RdtFile::parse(Path::new("ROOM1000.RDT"), &bytes)
            .expect("well-formed RDT with 17 camera records");
        assert_eq!(rdt.cameras.len(), 17);
        assert_eq!(rdt.mapped_camera_count(), 16);
        assert_eq!(rdt.background_image_index(15), Ok(15));
        assert!(rdt.background_image_index(16).is_err());
        let error = RdtCameraSession::open(rdt, Path::new("missing-roomcut.bin"), 16)
            .err()
            .expect("camera without a unique slot must be rejected before opening the BIN");
        assert!(error.contains("camera index 16 cannot map uniquely"));
    }

    #[test]
    fn rejects_truncated_core_and_rid_camera_records() {
        assert!(RdtFile::parse(Path::new("ROOM1000.RDT"), &[0; RDT_CORE_SIZE - 1]).is_err());

        let bytes = minimal_rdt(1, RDT_CORE_SIZE as u32);
        assert!(RdtFile::parse(Path::new("ROOM1000.RDT"), &bytes).is_err());
    }

    #[test]
    fn rejects_offsets_beyond_end_and_invalid_camera_indices() {
        let mut bytes = minimal_rdt(0, 0);
        bytes[8..12].copy_from_slice(&((RDT_CORE_SIZE + 1) as u32).to_le_bytes());
        assert!(RdtFile::parse(Path::new("ROOM1000.RDT"), &bytes).is_err());

        let bytes = minimal_rdt(0, 0);
        let rdt = RdtFile::parse(Path::new("ROOM1000.RDT"), &bytes).expect("empty valid RDT");
        assert!(rdt.camera(0).is_err());
    }

    #[test]
    fn validates_pri_relative_table_bounds_and_counts() {
        let mut bytes = vec![0; 16];
        bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
        assert!(parse_pri_mask_count(&bytes, 4).is_err());

        bytes[4..8].copy_from_slice(&[1, 0, 0, 0]);
        bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert!(parse_pri_mask_count(&bytes, 4).is_err());
    }

    #[test]
    fn computes_go_style_camera_fov() {
        let fov = calculate_fov_degrees(26_684 >> 7);
        assert_eq!(fov, 59.963276_f32);
        assert_eq!(calculate_fov_degrees(0), 180.0);
    }

    #[test]
    fn rejects_non_go_rdt_names_for_room_mapping() {
        assert!(RoomIdentity::from_rdt_path(Path::new("room_test.rdt")).is_err());
        assert!(RoomIdentity::from_rdt_path(Path::new("ROOM1010.RDT")).is_ok());
        assert!(RoomIdentity::from_rdt_path(Path::new("ROOMé00.RDT")).is_err());
    }
}

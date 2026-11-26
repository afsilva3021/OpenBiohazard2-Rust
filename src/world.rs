//! Room gameplay data and player movement, migrated from Go
//! `fileio/rdt_sca.go`, `fileio/rdt_rvd.go`, `world/collision.go`,
//! `world/cameraswitch.go` and `game/player.go`.

use crate::fileio::rdt::RdtFile;
use crate::math::rotation_y;

/// Go `FLOOR_HEIGHT_UNIT`: one floor level is 1800 units upward (negative Y).
pub const FLOOR_HEIGHT_UNIT: f32 = -1800.0;
const SCA_TYPE_SLOPE: u16 = 11;
const SCA_TYPE_STAIRS: u16 = 12;
const SHAPE_CLIMB_UP: u16 = 9;
const SHAPE_JUMP_DOWN: u16 = 10;

/// RDT offset-table indices (Go `RDTOffsets`).
const OFFSET_COLLISION: usize = 6;
const OFFSET_CAMERA_SWITCHES: usize = 8;

const SCA_HEADER_SIZE: usize = 16;
const SCA_ELEMENT_SIZE: usize = 16;
const MAX_SCA_ELEMENTS: usize = 4096;
const RVD_RECORD_SIZE: usize = 20;
const MAX_RVD_RECORDS: usize = 100;

/// Go player speeds (units per second) and turn rate (degrees per second).
pub const PLAYER_FORWARD_SPEED: f32 = 4000.0;
pub const PLAYER_BACKWARD_SPEED: f32 = 1000.0;
pub const PLAYER_TURN_SPEED: f32 = 100.0;
/// Go animation poses: -1 idle (neutral skeleton), 0 walk, 1 walk backward.
pub const POSE_WALK: usize = 0;
pub const POSE_BACKWARD: usize = 1;

/// Played in reverse. Measured on PL00.PLD: with Go's bone transforms, the
/// stance foot of EDD table 0 slides toward the model's +X (its facing, from
/// the toes and knee bend) and the EMR root speed is -X, so played forward the
/// model steps backward while moving forward ("moonwalk"). Reversed, the steps
/// match the forward motion. Table 1 already matches backward motion.
pub fn pose_plays_reversed(pose: Option<usize>) -> bool {
    pose == Some(POSE_WALK)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollisionEntity {
    pub index: usize,
    pub x: i32,
    pub z: i32,
    pub width: i32,
    pub density: i32,
    pub shape: u16,
    pub slope_height: f32,
    /// 0/1: ramp along X, 2/3: ramp along Z; `None` for non-ramps.
    pub slope_type: Option<u8>,
    pub ramp_bottom: f32,
    /// Bit n set: the entity exists on floor level n.
    pub floor_mask: u32,
}

impl CollisionEntity {
    fn on_floor(&self, floor: i32) -> bool {
        (0..32).contains(&floor) && (self.floor_mask >> floor) & 1 == 1
    }

    fn is_ramp(&self) -> bool {
        self.shape == SCA_TYPE_SLOPE || self.shape == SCA_TYPE_STAIRS
    }

    fn rectangle(&self) -> [[f32; 2]; 4] {
        let (x, z) = (self.x as f32, self.z as f32);
        let (w, d) = (self.width as f32, self.density as f32);
        [[x, z], [x, z + d], [x + w, z + d], [x + w, z]]
    }
}

/// Go `LoadRDT_SCA`.
pub fn parse_sca(rdt: &RdtFile) -> Result<Vec<CollisionEntity>, String> {
    let offset = section_offset(rdt, OFFSET_COLLISION, "SCA collision")?;
    let header = slice(&rdt.bytes, offset, SCA_HEADER_SIZE, "SCA header")?;
    let count = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    // Go reads Count - 1 elements; the first slot is the ceiling header.
    let element_count = count.saturating_sub(1);
    if element_count > MAX_SCA_ELEMENTS {
        return Err(format!(
            "SCA declares {count} elements; limit is {MAX_SCA_ELEMENTS}"
        ));
    }
    let elements = slice(
        &rdt.bytes,
        offset + SCA_HEADER_SIZE,
        element_count * SCA_ELEMENT_SIZE,
        "SCA elements",
    )?;
    let mut entities = Vec::with_capacity(element_count);
    for (index, record) in elements.chunks_exact(SCA_ELEMENT_SIZE).enumerate() {
        let x = i32::from(i16::from_le_bytes([record[0], record[1]]));
        let z = i32::from(i16::from_le_bytes([record[2], record[3]]));
        let width = i32::from(u16::from_le_bytes([record[4], record[5]]));
        let density = i32::from(u16::from_le_bytes([record[6], record[7]]));
        let flag = u16::from_le_bytes([record[8], record[9]]);
        let kind = u16::from_le_bytes([record[10], record[11]]);
        let floor_mask = u32::from_le_bytes([record[12], record[13], record[14], record[15]]);
        let shape = flag & 0x000f;
        let floor_multiplier = (kind >> 6) & 0x1f;

        let (slope_type, ramp_bottom) = if shape == SCA_TYPE_SLOPE || shape == SCA_TYPE_STAIRS {
            let slope = ((kind >> 4) & 3) as u8;
            let bottom = match slope {
                0 => x as f32,
                1 => (x + width) as f32,
                2 => z as f32,
                _ => (z + density) as f32,
            };
            (Some(slope), bottom)
        } else {
            (None, 0.0)
        };
        entities.push(CollisionEntity {
            index,
            x,
            z,
            width,
            density,
            shape,
            slope_height: f32::from(floor_multiplier) * FLOOR_HEIGHT_UNIT,
            slope_type,
            ramp_bottom,
            floor_mask,
        });
    }
    Ok(entities)
}

/// Go `RVDHeader`: crossing the quad switches from `cam0` to `cam1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraSwitch {
    pub flag: u8,
    pub floor: u8,
    pub cam0: u8,
    pub cam1: u8,
    pub corners: [[i16; 2]; 4],
}

/// Go `LoadRDT_RVD`: up to 100 records, terminated by 0xFF×4.
pub fn parse_rvd(rdt: &RdtFile) -> Result<Vec<CameraSwitch>, String> {
    let offset = section_offset(rdt, OFFSET_CAMERA_SWITCHES, "RVD camera switches")?;
    let mut switches = Vec::new();
    for index in 0..MAX_RVD_RECORDS {
        let record = slice(
            &rdt.bytes,
            offset + index * RVD_RECORD_SIZE,
            RVD_RECORD_SIZE,
            "RVD record",
        )?;
        if record[..4] == [0xff; 4] {
            break;
        }
        let value = |at: usize| i16::from_le_bytes([record[at], record[at + 1]]);
        switches.push(CameraSwitch {
            flag: record[0],
            floor: record[1],
            cam0: record[2],
            cam1: record[3],
            corners: [
                [value(4), value(6)],
                [value(8), value(10)],
                [value(12), value(14)],
                [value(16), value(18)],
            ],
        });
    }
    Ok(switches)
}

/// Go `CameraSwitchHandler`.
#[derive(Debug, Clone)]
pub struct CameraSwitches {
    switches: Vec<CameraSwitch>,
    /// For each camera, the switch indices checked while it is active.
    transitions: Vec<Vec<usize>>,
}

impl CameraSwitches {
    pub fn new(switches: Vec<CameraSwitch>, camera_count: usize) -> Self {
        let mut transitions = Vec::with_capacity(camera_count);
        for camera in 0..camera_count {
            let mut cam1_zero = Vec::new();
            let mut checked = Vec::new();
            for (index, switch) in switches.iter().enumerate() {
                if usize::from(switch.cam0) != camera {
                    continue;
                }
                // The first cam1 = 0 record describes the camera's own area;
                // the last one is the real switch back to camera 0.
                if switch.cam1 == 0 {
                    cam1_zero.push(index);
                } else {
                    checked.push(index);
                }
            }
            if cam1_zero.len() >= 2 {
                checked.push(cam1_zero[cam1_zero.len() - 1]);
            }
            transitions.push(checked);
        }
        Self {
            switches,
            transitions,
        }
    }

    /// Go `GetCameraSwitchNewRegion`: the switch the player is standing in.
    pub fn region(&self, position: [f32; 3], camera: usize) -> Option<&CameraSwitch> {
        let floor = floor_number(position[1]);
        self.transitions.get(camera)?.iter().find_map(|&index| {
            let region = &self.switches[index];
            if region.floor != 255 && i32::from(region.floor) != floor {
                return None;
            }
            let corners = region.corners.map(|[x, z]| [f32::from(x), f32::from(z)]);
            point_in_quad([position[0], position[2]], corners).then_some(region)
        })
    }
}

pub fn floor_number(y: f32) -> i32 {
    (y / FLOOR_HEIGHT_UNIT).round() as i32
}

/// Go `CheckCollision`: first entity on the player's floor containing the point.
pub fn check_collision(
    position: [f32; 3],
    entities: &[CollisionEntity],
) -> Option<&CollisionEntity> {
    let floor = floor_number(position[1]);
    let point = [position[0], position[2]];
    entities
        .iter()
        .filter(|entity| entity.on_floor(floor))
        .find(|entity| {
            let (x, z) = (entity.x as f32, entity.z as f32);
            let (w, d) = (entity.width as f32, entity.density as f32);
            match entity.shape {
                0 | SHAPE_CLIMB_UP | SHAPE_JUMP_DOWN | SCA_TYPE_SLOPE | SCA_TYPE_STAIRS => {
                    point_in_quad(point, entity.rectangle())
                }
                1 => point_in_triangle(point, [x, z + d], [x + w, z + d], [x + w, z]),
                2 => point_in_triangle(point, [x, z], [x, z + d], [x + w, z + d]),
                3 => point_in_triangle(point, [x, z], [x + w, z + d], [x + w, z]),
                6 => {
                    let radius = w / 2.0;
                    let center = [x + radius, z + radius];
                    ((point[0] - center[0]).powi(2) + (point[1] - center[1]).powi(2)).sqrt()
                        <= radius
                }
                7 => in_ellipse(point, [x + w / 2.0, z + d / 2.0], w / 2.0, d / 2.0),
                8 => in_ellipse(point, [x + w / 2.0, z + d / 2.0], w / 2.0, d / 2.0),
                _ => false,
            }
        })
}

/// Axis-aligned ellipse test with semi-axes along X and Z (Go cases 7 and 8
/// differ only in which axis they call "major").
fn in_ellipse(point: [f32; 2], center: [f32; 2], semi_x: f32, semi_z: f32) -> bool {
    if semi_x <= 0.0 || semi_z <= 0.0 {
        return false;
    }
    ((point[0] - center[0]) / semi_x).powi(2) + ((point[1] - center[1]) / semi_z).powi(2) <= 1.0
}

/// Go `isPointInRectangle`: inside when all edge cross products share a sign.
fn point_in_quad(point: [f32; 2], corners: [[f32; 2]; 4]) -> bool {
    let [x, z] = point;
    let mut signs = [0.0_f32; 4];
    for (index, sign) in signs.iter_mut().enumerate() {
        let [x1, z1] = corners[index];
        let [x2, z2] = corners[(index + 1) % 4];
        *sign = (x2 - x1) * (z - z1) - (z2 - z1) * (x - x1);
    }
    signs.iter().all(|&value| value > 0.0) || signs.iter().all(|&value| value < 0.0)
}

/// Go `isPointInTriangle` (area comparison with a 0.01 tolerance).
fn point_in_triangle(point: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let area = |p: [f32; 2], q: [f32; 2], r: [f32; 2]| {
        ((p[0] * (q[1] - r[1]) + q[0] * (r[1] - p[1]) + r[0] * (p[1] - q[1])) / 2.0).abs()
    };
    let total = area(a, b, c);
    let parts = area(point, b, c) + area(a, point, c) + area(a, b, point);
    (total - parts).abs() <= 0.01
}

fn section_offset(rdt: &RdtFile, index: usize, label: &str) -> Result<usize, String> {
    let offset = *rdt
        .offsets
        .get(index)
        .ok_or_else(|| format!("RDT has no {label} offset"))? as usize;
    if offset == 0 {
        return Err(format!("RDT has no {label} section"));
    }
    Ok(offset)
}

fn slice<'a>(
    bytes: &'a [u8],
    start: usize,
    length: usize,
    label: &str,
) -> Result<&'a [u8], String> {
    start
        .checked_add(length)
        .and_then(|end| bytes.get(start..end))
        .ok_or_else(|| format!("{label} at {start} needs {length} bytes beyond the RDT end"))
}

/// Go `game.Player`.
#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    pub position: [f32; 3],
    /// Degrees around +Y, as in Go.
    pub rotation_degrees: f32,
    /// Animation pose; `None` is idle (Go pose -1).
    pub pose: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerInput {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
}

impl Player {
    pub fn new(position: [f32; 3], rotation_degrees: f32) -> Self {
        Self {
            position,
            rotation_degrees,
            pose: None,
        }
    }

    /// One frame of tank controls with collision, matching Go's handlers.
    pub fn update(&mut self, input: PlayerInput, entities: &[CollisionEntity], seconds: f32) {
        self.pose = None;
        if input.left {
            self.rotation_degrees -= PLAYER_TURN_SPEED * seconds;
            if self.rotation_degrees < 0.0 {
                self.rotation_degrees += 360.0;
            }
        }
        if input.right {
            self.rotation_degrees += PLAYER_TURN_SPEED * seconds;
            if self.rotation_degrees > 360.0 {
                self.rotation_degrees -= 360.0;
            }
        }
        if input.forward {
            self.move_by(PLAYER_FORWARD_SPEED * seconds, POSE_WALK, true, entities);
        } else if input.backward {
            self.move_by(
                -PLAYER_BACKWARD_SPEED * seconds,
                POSE_BACKWARD,
                false,
                entities,
            );
        }
    }

    fn predict(&self, distance: f32) -> [f32; 3] {
        // Go rotates the vector (distance, 0, 0) by the model's Y rotation.
        let rotation = rotation_y(self.rotation_degrees.to_radians());
        [
            self.position[0] + rotation[0] * distance,
            self.position[1],
            self.position[2] + rotation[2] * distance,
        ]
    }

    fn move_by(&mut self, distance: f32, pose: usize, forward: bool, entities: &[CollisionEntity]) {
        let predicted = self.predict(distance);
        match check_collision(predicted, entities) {
            None => {
                self.position = predicted;
                self.pose = Some(pose);
            }
            Some(entity) if entity.is_ramp() => {
                self.position = slope_position(predicted, entity);
                self.pose = Some(pose);
            }
            Some(entity)
                if forward
                    && (entity.shape == SHAPE_CLIMB_UP || entity.shape == SHAPE_JUMP_DOWN) =>
            {
                // Go's ramp-shape check compares against 9/10 here: climb a box.
                self.position = self.climb_box();
            }
            Some(_) => {}
        }
    }

    fn climb_box(&self) -> [f32; 3] {
        let [x, _, z] = self.position;
        match floor_number(self.position[1]) {
            0 => [x, FLOOR_HEIGHT_UNIT, z],
            1 => [x, 0.0, z],
            _ => self.position,
        }
    }
}

/// Go `PredictPositionSlope`: height interpolated from the ramp bottom.
fn slope_position(flat: [f32; 3], ramp: &CollisionEntity) -> [f32; 3] {
    let progress = match ramp.slope_type {
        Some(0 | 1) if ramp.width != 0 => (flat[0] - ramp.ramp_bottom).abs() / ramp.width as f32,
        Some(2 | 3) if ramp.density != 0 => {
            (flat[2] - ramp.ramp_bottom).abs() / ramp.density as f32
        }
        _ => 0.0,
    };
    [flat[0], ramp.slope_height * progress, flat[2]]
}

#[cfg(test)]
mod tests {
    use super::{
        check_collision, point_in_quad, slope_position, CameraSwitch, CameraSwitches,
        CollisionEntity, Player, PlayerInput, POSE_WALK,
    };

    fn wall(x: i32, z: i32, width: i32, density: i32, shape: u16) -> CollisionEntity {
        CollisionEntity {
            index: 0,
            x,
            z,
            width,
            density,
            shape,
            slope_height: 0.0,
            slope_type: None,
            ramp_bottom: 0.0,
            floor_mask: 1,
        }
    }

    #[test]
    fn rectangles_triangles_and_circles_block_like_go() {
        let rect = wall(0, 0, 100, 100, 0);
        assert!(check_collision([50.0, 0.0, 50.0], std::slice::from_ref(&rect)).is_some());
        assert!(check_collision([150.0, 0.0, 50.0], std::slice::from_ref(&rect)).is_none());
        // Other floors are ignored.
        assert!(check_collision([50.0, -1800.0, 50.0], std::slice::from_ref(&rect)).is_none());

        let triangle = wall(0, 0, 100, 100, 3); // vertices (0,0) (100,100) (100,0)
        assert!(check_collision([90.0, 0.0, 10.0], std::slice::from_ref(&triangle)).is_some());
        assert!(check_collision([10.0, 0.0, 90.0], std::slice::from_ref(&triangle)).is_none());

        let circle = wall(0, 0, 100, 100, 6);
        assert!(check_collision([50.0, 0.0, 50.0], std::slice::from_ref(&circle)).is_some());
        assert!(check_collision([2.0, 0.0, 2.0], std::slice::from_ref(&circle)).is_none());
    }

    #[test]
    fn tank_controls_move_along_the_facing_direction() {
        let mut player = Player::new([0.0, 0.0, 0.0], 0.0);
        player.update(
            PlayerInput {
                forward: true,
                ..Default::default()
            },
            &[],
            0.5,
        );
        assert_eq!(player.pose, Some(POSE_WALK));
        assert!((player.position[0] - 2000.0).abs() < 1e-3);

        // Facing 90°: Go's Y rotation sends +X to -Z.
        let mut player = Player::new([0.0, 0.0, 0.0], 90.0);
        player.update(
            PlayerInput {
                forward: true,
                ..Default::default()
            },
            &[],
            0.5,
        );
        assert!(player.position[0].abs() < 1e-2 && (player.position[2] + 2000.0).abs() < 1e-2);

        // A wall in front stops the player and keeps it idle.
        let mut player = Player::new([0.0, 0.0, 50.0], 0.0);
        player.update(
            PlayerInput {
                forward: true,
                ..Default::default()
            },
            &[wall(100, 0, 500, 100, 0)],
            0.1,
        );
        assert_eq!(player.position, [0.0, 0.0, 50.0]);
        assert_eq!(player.pose, None);
    }

    #[test]
    fn turning_wraps_between_0_and_360_degrees() {
        let mut player = Player::new([0.0; 3], 5.0);
        player.update(
            PlayerInput {
                left: true,
                ..Default::default()
            },
            &[],
            0.1,
        );
        assert!((player.rotation_degrees - 355.0).abs() < 1e-3);
    }

    #[test]
    fn ramps_raise_the_player_proportionally() {
        let mut ramp = wall(0, 0, 1000, 500, 11);
        ramp.slope_type = Some(0);
        ramp.slope_height = -1800.0;
        let position = slope_position([250.0, 0.0, 100.0], &ramp);
        assert!((position[1] + 450.0).abs() < 1e-3);
    }

    #[test]
    fn camera_switch_regions_follow_go_transition_rules() {
        let square = |offset: i16| {
            [
                [offset, 0],
                [offset, 100],
                [offset + 100, 100],
                [offset + 100, 0],
            ]
        };
        let switches = vec![
            CameraSwitch {
                flag: 0,
                floor: 255,
                cam0: 1,
                cam1: 0,
                corners: square(0),
            },
            CameraSwitch {
                flag: 0,
                floor: 255,
                cam0: 1,
                cam1: 2,
                corners: square(200),
            },
            CameraSwitch {
                flag: 0,
                floor: 255,
                cam0: 1,
                cam1: 0,
                corners: square(400),
            },
        ];
        let handler = CameraSwitches::new(switches, 3);
        // First cam1=0 record is ignored, the last one switches back to 0.
        assert!(handler.region([50.0, 0.0, 50.0], 1).is_none());
        assert_eq!(
            handler.region([250.0, 0.0, 50.0], 1).map(|r| r.cam1),
            Some(2)
        );
        assert_eq!(
            handler.region([450.0, 0.0, 50.0], 1).map(|r| r.cam1),
            Some(0)
        );
        assert!(handler.region([250.0, 0.0, 50.0], 0).is_none());
        assert!(point_in_quad(
            [1.0, 1.0],
            [[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0]]
        ));
    }
}

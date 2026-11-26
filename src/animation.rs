//! Skeletal animation playback, migrated from Go `render/animation.go` and
//! `render/AnimatedEntity.go` (`buildComponentTransforms`).

use crate::fileio::pld::{EddFrame, EmrFrame, Skeleton};
use crate::math::{identity, multiply, rotation_x, rotation_y, rotation_z, translation, Mat4};

/// Go `ANIMATION_FRAME_TIME`: each EDD entry is shown for 30 ms.
pub const ANIMATION_FRAME_MS: f64 = 30.0;

#[derive(Debug, Clone, Default)]
pub struct Animation {
    total_ms: f64,
    frame_index: usize,
    frame_number: usize,
    pose: Option<usize>,
    reverse: bool,
}

impl Animation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mirrors Go `UpdateAnimationFrame`. `pose == None` is Go's pose `-1`:
    /// the neutral skeleton without rotations. A pose without a valid table
    /// is also treated as neutral instead of panicking. `reverse` plays the
    /// table from its last entry to its first.
    pub fn update(
        &mut self,
        pose: Option<usize>,
        reverse: bool,
        tables: &[Vec<EddFrame>],
        elapsed_seconds: f64,
    ) {
        let pose = pose.filter(|&index| tables.get(index).is_some_and(|table| !table.is_empty()));
        if self.pose.is_some() {
            self.total_ms += elapsed_seconds * 1000.0;
        } else {
            self.total_ms = 0.0;
        }

        if self.pose != pose || self.reverse != reverse {
            self.frame_index = 0;
            self.reverse = reverse;
            if let Some(index) = pose {
                self.frame_number = self.entry(&tables[index]).frame_id;
            }
            self.pose = pose;
        }

        if let Some(index) = self.pose {
            if self.total_ms >= ANIMATION_FRAME_MS {
                self.total_ms = 0.0;
                self.frame_index += 1;
                let table = &tables[index];
                if self.frame_index >= table.len() {
                    self.frame_index = 0;
                }
                self.frame_number = self.entry(table).frame_id;
            }
        }
    }

    fn entry<'a>(&self, table: &'a [EddFrame]) -> &'a EddFrame {
        if self.reverse {
            &table[table.len() - 1 - self.frame_index]
        } else {
            &table[self.frame_index]
        }
    }

    /// EMR frame to display, or `None` for the neutral pose.
    pub fn current_frame(&self) -> Option<usize> {
        self.pose.map(|_| self.frame_number)
    }
}

/// Builds one transform per bone: parent × translate(relative) × Rx × Ry × Rz.
///
/// Go only rebuilds these when the pose number changes, which freezes the
/// animation on its first frame; Rust rebuilds them every frame (15 bones is
/// negligible work) so the walk cycle actually plays.
pub fn bone_transforms(skeleton: &Skeleton, frame: Option<&EmrFrame>, output: &mut Vec<Mat4>) {
    output.clear();
    output.resize(skeleton.relative_positions.len(), identity());
    if !skeleton.relative_positions.is_empty() {
        visit(skeleton, frame, 0, identity(), output, 0);
    }
}

fn visit(
    skeleton: &Skeleton,
    frame: Option<&EmrFrame>,
    bone: usize,
    parent: Mat4,
    output: &mut [Mat4],
    depth: usize,
) {
    // The PLD loader already rejects cycles; the depth guard keeps this total.
    if bone >= output.len() || depth > output.len() {
        return;
    }
    let mut transform = multiply(parent, translation(skeleton.relative_positions[bone]));
    if let Some(angles) = frame.and_then(|frame| frame.rotations.get(bone)) {
        let rotation = multiply(
            rotation_x(angles[0]),
            multiply(rotation_y(angles[1]), rotation_z(angles[2])),
        );
        transform = multiply(transform, rotation);
    }
    output[bone] = transform;
    for &child in &skeleton.children[bone] {
        visit(skeleton, frame, child, transform, output, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::{bone_transforms, Animation};
    use crate::fileio::pld::{EddFrame, EmrFrame, Skeleton};
    use crate::math::transform_point;

    fn tables() -> Vec<Vec<EddFrame>> {
        vec![vec![
            EddFrame {
                frame_id: 4,
                flag: 0,
            },
            EddFrame {
                frame_id: 7,
                flag: 0,
            },
        ]]
    }

    #[test]
    fn advances_every_30ms_and_loops_like_go() {
        let tables = tables();
        let mut animation = Animation::new();
        animation.update(None, false, &tables, 0.016);
        assert_eq!(animation.current_frame(), None);
        animation.update(Some(0), false, &tables, 0.016);
        assert_eq!(animation.current_frame(), Some(4));
        animation.update(Some(0), false, &tables, 0.031);
        assert_eq!(animation.current_frame(), Some(7));
        animation.update(Some(0), false, &tables, 0.031);
        assert_eq!(animation.current_frame(), Some(4));
        // Unknown poses fall back to neutral instead of panicking.
        animation.update(Some(9), false, &tables, 0.031);
        assert_eq!(animation.current_frame(), None);
    }

    #[test]
    fn child_bones_inherit_parent_translation_and_rotation() {
        let skeleton = Skeleton {
            relative_positions: vec![[100.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
            children: vec![vec![1], vec![]],
        };
        let mut transforms = Vec::new();
        bone_transforms(&skeleton, None, &mut transforms);
        let child = transform_point(&transforms[1], [0.0, 0.0, 0.0]);
        assert_eq!(child, [100.0, 10.0, 0.0, 1.0]);

        // Rotate the root 90° about Z: the child's +Y offset turns into -X.
        let frame = EmrFrame {
            offset: [0; 3],
            speed: [0; 3],
            rotations: vec![[0.0, 0.0, std::f32::consts::FRAC_PI_2], [0.0; 3]],
        };
        bone_transforms(&skeleton, Some(&frame), &mut transforms);
        let child = transform_point(&transforms[1], [0.0, 0.0, 0.0]);
        assert!((child[0] - 90.0).abs() < 1e-3 && child[1].abs() < 1e-3);
    }
}

#[cfg(test)]
mod reverse_tests {
    use super::*;

    #[test]
    fn reverse_plays_the_table_backwards() {
        let frame = |frame_id| EddFrame { frame_id, flag: 0 };
        let tables = vec![vec![frame(4), frame(7), frame(9)]];
        let mut animation = Animation::new();
        animation.update(Some(0), true, &tables, 0.016);
        assert_eq!(animation.current_frame(), Some(9));
        animation.update(Some(0), true, &tables, 0.031);
        assert_eq!(animation.current_frame(), Some(7));
        animation.update(Some(0), true, &tables, 0.031);
        assert_eq!(animation.current_frame(), Some(4));
        animation.update(Some(0), true, &tables, 0.031);
        assert_eq!(animation.current_frame(), Some(9));
    }
}

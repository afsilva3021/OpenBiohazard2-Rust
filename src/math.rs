//! Column-major 4×4 matrix helpers shared by the renderer and game logic
//! (GLSL / mathgl conventions).

pub type Mat4 = [f32; 16];

pub fn identity() -> Mat4 {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

pub fn multiply(left: Mat4, right: Mat4) -> Mat4 {
    let mut result = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            for k in 0..4 {
                result[column * 4 + row] += left[k * 4 + row] * right[column * 4 + k];
            }
        }
    }
    result
}

pub fn translation(position: [f32; 3]) -> Mat4 {
    let mut result = identity();
    result[12] = position[0];
    result[13] = position[1];
    result[14] = position[2];
    result
}

pub fn rotation_y(angle: f32) -> Mat4 {
    let (sin, cos) = angle.sin_cos();
    [
        cos, 0.0, -sin, 0.0, 0.0, 1.0, 0.0, 0.0, sin, 0.0, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

pub fn scale(x: f32, y: f32, z: f32) -> Mat4 {
    [
        x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Result<Mat4, String> {
    let forward = normalize(subtract(target, eye), "RDT camera direction")?;
    let side = normalize(cross(forward, up), "RDT camera up direction")?;
    let corrected_up = cross(side, forward);
    Ok([
        side[0],
        corrected_up[0],
        -forward[0],
        0.0,
        side[1],
        corrected_up[1],
        -forward[1],
        0.0,
        side[2],
        corrected_up[2],
        -forward[2],
        0.0,
        -dot(side, eye),
        -dot(corrected_up, eye),
        dot(forward, eye),
        1.0,
    ])
}

pub fn perspective(fov_radians: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let focal = 1.0 / (fov_radians * 0.5).tan();
    let mut result = [0.0; 16];
    result[0] = focal / aspect;
    result[5] = focal;
    result[10] = (far + near) / (near - far);
    result[11] = -1.0;
    result[14] = (2.0 * far * near) / (near - far);
    result
}

pub fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

pub fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

pub fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

pub fn normalize(vector: [f32; 3], label: &str) -> Result<[f32; 3], String> {
    let length = dot(vector, vector).sqrt();
    if !length.is_finite() || length <= f32::EPSILON {
        return Err(format!("cannot build view matrix: {label} is degenerate"));
    }
    Ok([vector[0] / length, vector[1] / length, vector[2] / length])
}

pub fn rotation_x(angle: f32) -> Mat4 {
    let (sin, cos) = angle.sin_cos();
    [
        1.0, 0.0, 0.0, 0.0, 0.0, cos, sin, 0.0, 0.0, -sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

pub fn rotation_z(angle: f32) -> Mat4 {
    let (sin, cos) = angle.sin_cos();
    [
        cos, sin, 0.0, 0.0, -sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

/// Transforms a point (w = 1) by a column-major matrix.
pub fn transform_point(matrix: &Mat4, point: [f32; 3]) -> [f32; 4] {
    let mut result = [0.0; 4];
    for (row, value) in result.iter_mut().enumerate() {
        *value = matrix[row] * point[0]
            + matrix[4 + row] * point[1]
            + matrix[8 + row] * point[2]
            + matrix[12 + row];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{multiply, rotation_x, rotation_y, rotation_z, transform_point, translation};

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn rotations_follow_the_right_hand_rule() {
        let quarter = std::f32::consts::FRAC_PI_2;
        assert!(close(
            transform_point(&rotation_x(quarter), [0.0, 1.0, 0.0]),
            [0.0, 0.0, 1.0, 1.0]
        ));
        assert!(close(
            transform_point(&rotation_y(quarter), [0.0, 0.0, 1.0]),
            [1.0, 0.0, 0.0, 1.0]
        ));
        assert!(close(
            transform_point(&rotation_z(quarter), [1.0, 0.0, 0.0]),
            [0.0, 1.0, 0.0, 1.0]
        ));
    }

    #[test]
    fn multiply_applies_right_matrix_first() {
        let m = multiply(
            translation([10.0, 0.0, 0.0]),
            rotation_z(std::f32::consts::FRAC_PI_2),
        );
        assert!(close(
            transform_point(&m, [1.0, 0.0, 0.0]),
            [10.0, 1.0, 0.0, 1.0]
        ));
    }
}

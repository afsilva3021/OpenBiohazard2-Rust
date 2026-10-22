use std::{
    fs::File,
    io::{Read, Take},
    path::Path,
};

const PLD_HEADER_SIZE: usize = 8;
const PLD_DIRECTORY_FIELDS: usize = 4;
const MD1_HEADER_SIZE: usize = 12;
const MD1_OBJECT_HEADER_SIZE: usize = 56;
const EMR_HEADER_SIZE: usize = 8;
const MAX_PLD_BYTES: u64 = 32 * 1024 * 1024;
const MAX_COMPONENTS: usize = 256;
const MAX_STORED_VERTICES: usize = 1_000_000;
const MAX_DRAW_VERTICES: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Debug)]
pub struct StaticPldModel {
    /// Expanded triangle-list positions after the PLD's neutral skeleton offsets.
    pub positions: Vec<[f32; 3]>,
    pub component_count: usize,
    pub stored_vertex_count: usize,
    pub triangle_count: usize,
    pub quad_count: usize,
    pub bounds: ModelBounds,
}

impl StaticPldModel {
    pub fn draw_vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangulated_face_count(&self) -> usize {
        self.triangle_count + self.quad_count * 2
    }
}

pub fn load_static_pld(path: impl AsRef<Path>) -> Result<StaticPldModel, String> {
    let path = path.as_ref();
    let file = File::open(path)
        .map_err(|error| format!("failed to open PLD {}: {error}", path.display()))?;
    let file_length = file
        .metadata()
        .map_err(|error| format!("failed to inspect PLD {}: {error}", path.display()))?
        .len();
    if file_length > MAX_PLD_BYTES {
        return Err(format!(
            "PLD {} is {file_length} bytes; limit is {MAX_PLD_BYTES}",
            path.display()
        ));
    }

    let mut bytes = Vec::with_capacity(file_length as usize);
    let mut limited: Take<File> = file.take(MAX_PLD_BYTES + 1);
    limited
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read PLD {}: {error}", path.display()))?;
    if bytes.len() as u64 > MAX_PLD_BYTES {
        return Err(format!(
            "PLD {} exceeds the {MAX_PLD_BYTES}-byte input limit",
            path.display()
        ));
    }

    parse_static_pld(&bytes).map_err(|error| format!("invalid PLD {}: {error}", path.display()))
}

fn parse_static_pld(bytes: &[u8]) -> Result<StaticPldModel, String> {
    if bytes.len() < PLD_HEADER_SIZE {
        return Err("truncated 8-byte PLD header".to_owned());
    }
    let directory_offset = read_u32(bytes, 0, "PLD directory offset")? as usize;
    let directory_count = read_u32(bytes, 4, "PLD directory field count")? as usize;
    if directory_count != PLD_DIRECTORY_FIELDS {
        return Err(format!(
            "PLD declares {directory_count} directory fields; the Go PLD layout requires {PLD_DIRECTORY_FIELDS} (EDD, EMR, MD1, TIM)"
        ));
    }
    let directory_bytes = directory_count
        .checked_mul(4)
        .ok_or_else(|| "PLD directory size overflow".to_owned())?;
    let directory_end = directory_offset
        .checked_add(directory_bytes)
        .ok_or_else(|| "PLD directory end overflow".to_owned())?;
    if directory_offset < PLD_HEADER_SIZE || directory_end > bytes.len() {
        return Err(format!(
            "PLD directory range {directory_offset}..{directory_end} is outside the {}-byte file",
            bytes.len()
        ));
    }

    let mut offsets = [0_usize; PLD_DIRECTORY_FIELDS];
    for index in 0..offsets.len() {
        let at = directory_offset + index * 4;
        offsets[index] = read_u32(bytes, at, "PLD section offset")? as usize;
        if offsets[index] < PLD_HEADER_SIZE || offsets[index] >= directory_offset {
            return Err(format!(
                "PLD section {index} offset {} is outside the data region {}..{directory_offset}",
                offsets[index], PLD_HEADER_SIZE
            ));
        }
        if index > 0 && offsets[index] <= offsets[index - 1] {
            return Err(format!(
                "PLD section offsets are not strictly increasing at field {index}: {} follows {}",
                offsets[index],
                offsets[index - 1]
            ));
        }
    }

    let emr = &bytes[offsets[1]..offsets[2]];
    let md1 = &bytes[offsets[2]..offsets[3]];
    let skeleton = parse_neutral_skeleton(emr)?;
    parse_md1(md1, &skeleton)
}

#[derive(Debug)]
struct NeutralSkeleton {
    component_offsets: Vec<[f32; 3]>,
}

fn parse_neutral_skeleton(section: &[u8]) -> Result<NeutralSkeleton, String> {
    if section.len() < EMR_HEADER_SIZE {
        return Err("EMR section is shorter than its 8-byte header".to_owned());
    }
    let armatures_offset = read_u16(section, 0, "EMR armature offset")? as usize;
    let frames_offset = read_u16(section, 2, "EMR frame offset")? as usize;
    let component_count = read_u16(section, 4, "EMR bone count")? as usize;
    let frame_element_size = read_u16(section, 6, "EMR frame element size")? as usize;

    if component_count == 0 || component_count > MAX_COMPONENTS {
        return Err(format!(
            "EMR bone count {component_count} is outside 1..={MAX_COMPONENTS}"
        ));
    }
    let positions_size = component_count
        .checked_mul(6)
        .ok_or_else(|| "EMR relative-position table size overflow".to_owned())?;
    let positions_end = EMR_HEADER_SIZE
        .checked_add(positions_size)
        .ok_or_else(|| "EMR relative-position table end overflow".to_owned())?;
    if positions_end > section.len() || armatures_offset < positions_end {
        return Err(format!(
            "EMR relative-position table ends at {positions_end}, armature table starts at {armatures_offset}, section length is {}",
            section.len()
        ));
    }

    let armature_bytes = component_count
        .checked_mul(4)
        .ok_or_else(|| "EMR armature table size overflow".to_owned())?;
    let armature_end = armatures_offset
        .checked_add(armature_bytes)
        .ok_or_else(|| "EMR armature table end overflow".to_owned())?;
    if armature_end > section.len() || frames_offset < armature_end || frames_offset > section.len()
    {
        return Err(format!(
            "EMR armature table range {armatures_offset}..{armature_end} or frame offset {frames_offset} is invalid for {} bytes",
            section.len()
        ));
    }
    if frame_element_size != 0 && frame_element_size < 12 {
        return Err(format!(
            "EMR frame element size {frame_element_size} is smaller than its 12-byte header"
        ));
    }

    let mut relative_positions = Vec::with_capacity(component_count);
    for index in 0..component_count {
        let at = EMR_HEADER_SIZE + index * 6;
        relative_positions.push([
            read_i16(section, at, "EMR bone X offset")? as f32,
            read_i16(section, at + 2, "EMR bone Y offset")? as f32,
            read_i16(section, at + 4, "EMR bone Z offset")? as f32,
        ]);
    }

    let mut children = Vec::with_capacity(component_count);
    let mut armature_offsets = Vec::with_capacity(component_count);
    for index in 0..component_count {
        let at = armatures_offset + index * 4;
        let child_count = read_u16(section, at, "EMR child count")? as usize;
        let child_offset = read_u16(section, at + 2, "EMR child-list offset")? as usize;
        if child_count >= component_count {
            return Err(format!(
                "EMR bone {index} has invalid child count {child_count} for {component_count} bones"
            ));
        }
        if child_count == 0 {
            armature_offsets.push(child_offset);
            children.push(Vec::new());
            continue;
        }
        let child_start = armatures_offset
            .checked_add(child_offset)
            .ok_or_else(|| format!("EMR bone {index} child-list offset overflow"))?;
        let child_end = child_start
            .checked_add(child_count)
            .ok_or_else(|| format!("EMR bone {index} child-list end overflow"))?;
        if child_start < armature_end || child_end > frames_offset {
            return Err(format!(
                "EMR bone {index} child-list range {child_start}..{child_end} is outside the hierarchy region {armature_end}..{frames_offset}"
            ));
        }
        let mut bone_children = Vec::with_capacity(child_count);
        for &child in &section[child_start..child_end] {
            if usize::from(child) >= component_count {
                return Err(format!(
                    "EMR bone {index} references out-of-range child {child}"
                ));
            }
            bone_children.push(usize::from(child));
        }
        armature_offsets.push(child_offset);
        children.push(bone_children);
    }

    // Go's loader also exposes a component-id list beginning at the root's
    // child-list offset. Read and bound it, but preserve Go's render behavior:
    // MD1 component i receives neutral transform i.
    let root_child_offset = armature_offsets[0];
    let mesh_ids_start = armatures_offset
        .checked_add(root_child_offset)
        .ok_or_else(|| "EMR mesh-id-list offset overflow".to_owned())?;
    let mesh_ids_end = mesh_ids_start
        .checked_add(component_count)
        .ok_or_else(|| "EMR mesh-id-list end overflow".to_owned())?;
    if mesh_ids_end > frames_offset {
        return Err(format!(
            "EMR mesh-id-list range {mesh_ids_start}..{mesh_ids_end} exceeds hierarchy region ending at {frames_offset}"
        ));
    }
    let mesh_ids = section[mesh_ids_start..mesh_ids_end].to_vec();
    for &mesh_id in &mesh_ids {
        if usize::from(mesh_id) >= component_count {
            return Err(format!(
                "EMR mesh-id list references out-of-range component {mesh_id}"
            ));
        }
    }

    let mut component_offsets = vec![[0.0; 3]; component_count];
    let mut visit_state = vec![0_u8; component_count];
    let mut parent_counts = vec![0_u8; component_count];
    visit_skeleton(
        0,
        None,
        &relative_positions,
        &children,
        &mut component_offsets,
        &mut visit_state,
        &mut parent_counts,
    )?;
    if let Some(unvisited) = visit_state.iter().position(|&state| state == 0) {
        return Err(format!(
            "EMR hierarchy does not connect bone {unvisited} to root bone 0"
        ));
    }

    Ok(NeutralSkeleton { component_offsets })
}

#[allow(clippy::too_many_arguments)]
fn visit_skeleton(
    index: usize,
    parent_offset: Option<[f32; 3]>,
    relative_positions: &[[f32; 3]],
    children: &[Vec<usize>],
    component_offsets: &mut [[f32; 3]],
    visit_state: &mut [u8],
    parent_counts: &mut [u8],
) -> Result<(), String> {
    if visit_state[index] == 1 {
        return Err(format!(
            "EMR hierarchy contains a cycle through bone {index}"
        ));
    }
    if visit_state[index] == 2 {
        return Err(format!("EMR hierarchy reaches bone {index} more than once"));
    }
    visit_state[index] = 1;
    let local = relative_positions[index];
    let accumulated = parent_offset.map_or(local, |parent| {
        [
            parent[0] + local[0],
            parent[1] + local[1],
            parent[2] + local[2],
        ]
    });
    component_offsets[index] = accumulated;

    for &child in &children[index] {
        parent_counts[child] = parent_counts[child]
            .checked_add(1)
            .ok_or_else(|| format!("EMR bone {child} has too many parents"))?;
        if parent_counts[child] > 1 {
            return Err(format!(
                "EMR bone {child} has more than one parent in the hierarchy"
            ));
        }
        visit_skeleton(
            child,
            Some(accumulated),
            relative_positions,
            children,
            component_offsets,
            visit_state,
            parent_counts,
        )?;
    }
    visit_state[index] = 2;
    Ok(())
}

#[derive(Debug)]
struct Md1ObjectHeader {
    triangles: PrimitiveHeader,
    quads: PrimitiveHeader,
}

#[derive(Debug, Clone, Copy)]
struct PrimitiveHeader {
    vertex_offset: u32,
    vertex_count: u32,
    normal_offset: u32,
    normal_count: u32,
    index_offset: u32,
    index_count: u32,
    texture_offset: u32,
}

fn parse_md1(section: &[u8], skeleton: &NeutralSkeleton) -> Result<StaticPldModel, String> {
    if section.len() < MD1_HEADER_SIZE {
        return Err("MD1 section is shorter than its 12-byte header".to_owned());
    }
    let declared_data_length = read_u32(section, 0, "MD1 declared section length")? as usize;
    let object_count_raw = read_u32(section, 8, "MD1 object count")? as usize;
    if object_count_raw == 0 || !object_count_raw.is_multiple_of(2) {
        return Err(format!(
            "MD1 object count {object_count_raw} must be a nonzero even number"
        ));
    }
    let component_count = object_count_raw / 2;
    if component_count > MAX_COMPONENTS {
        return Err(format!(
            "MD1 has {component_count} components; limit is {MAX_COMPONENTS}"
        ));
    }
    if component_count != skeleton.component_offsets.len() {
        return Err(format!(
            "MD1 has {component_count} components but EMR has {} bones; Go's static renderer pairs them by index",
            skeleton.component_offsets.len()
        ));
    }
    if declared_data_length == 0 || declared_data_length > section.len() {
        return Err(format!(
            "MD1 declared data length {declared_data_length} is invalid for its {}-byte section",
            section.len()
        ));
    }

    let object_headers_size = component_count
        .checked_mul(MD1_OBJECT_HEADER_SIZE)
        .ok_or_else(|| "MD1 object-header table size overflow".to_owned())?;
    let table_end = MD1_HEADER_SIZE
        .checked_add(object_headers_size)
        .ok_or_else(|| "MD1 object-header table end overflow".to_owned())?;
    if table_end > section.len() {
        return Err(format!(
            "MD1 object-header table ends at {table_end}, beyond the {}-byte section",
            section.len()
        ));
    }

    let mut headers = Vec::with_capacity(component_count);
    for component in 0..component_count {
        let at = MD1_HEADER_SIZE + component * MD1_OBJECT_HEADER_SIZE;
        headers.push(Md1ObjectHeader {
            triangles: read_primitive_header(section, at, "MD1 triangle header")?,
            quads: read_primitive_header(section, at + 28, "MD1 quad header")?,
        });
    }

    let mut positions = Vec::new();
    let mut stored_vertex_count = 0_usize;
    let mut triangle_count = 0_usize;
    let mut quad_count = 0_usize;
    for (component, header) in headers.iter().enumerate() {
        let triangle_vertex_count =
            checked_count(header.triangles.vertex_count, "MD1 vertex count")?;
        let quad_vertex_count = checked_count(header.quads.vertex_count, "MD1 vertex count")?;
        stored_vertex_count = stored_vertex_count
            .checked_add(triangle_vertex_count)
            .and_then(|count| count.checked_add(quad_vertex_count))
            .ok_or_else(|| "MD1 stored vertex count overflow".to_owned())?;
        if stored_vertex_count > MAX_STORED_VERTICES {
            return Err(format!(
                "MD1 has {stored_vertex_count} stored vertices; limit is {MAX_STORED_VERTICES}"
            ));
        }
        let triangle_vertices = read_vertices(section, header.triangles, "triangle", component)?;
        let quad_vertices = read_vertices(section, header.quads, "quad", component)?;

        validate_region(
            section,
            header.triangles,
            header.triangles.normal_count,
            8,
            "triangle normals",
            component,
        )?;
        validate_region(
            section,
            header.quads,
            header.quads.normal_count,
            8,
            "quad normals",
            component,
        )?;
        validate_region(
            section,
            header.triangles,
            header.triangles.index_count,
            12,
            "triangle indices",
            component,
        )?;
        validate_region(
            section,
            header.triangles,
            header.triangles.index_count,
            12,
            "triangle texture records",
            component,
        )?;
        validate_region(
            section,
            header.quads,
            header.quads.index_count,
            16,
            "quad indices",
            component,
        )?;
        validate_region(
            section,
            header.quads,
            header.quads.index_count,
            16,
            "quad texture records",
            component,
        )?;

        let triangle_faces = checked_count(header.triangles.index_count, "MD1 triangle count")?;
        let quad_faces = checked_count(header.quads.index_count, "MD1 quad count")?;
        triangle_count = triangle_count
            .checked_add(triangle_faces)
            .ok_or_else(|| "MD1 triangle count overflow".to_owned())?;
        quad_count = quad_count
            .checked_add(quad_faces)
            .ok_or_else(|| "MD1 quad count overflow".to_owned())?;
        let added_vertices = triangle_faces
            .checked_mul(3)
            .and_then(|count| {
                quad_faces
                    .checked_mul(6)
                    .and_then(|quads| count.checked_add(quads))
            })
            .ok_or_else(|| "MD1 draw vertex count overflow".to_owned())?;
        let next_len = positions
            .len()
            .checked_add(added_vertices)
            .ok_or_else(|| "MD1 draw vertex count overflow".to_owned())?;
        if next_len > MAX_DRAW_VERTICES {
            return Err(format!(
                "MD1 needs {next_len} expanded vertices; limit is {MAX_DRAW_VERTICES}"
            ));
        }
        positions.reserve(added_vertices);

        append_triangles(
            section,
            header.triangles,
            component,
            &triangle_vertices,
            &skeleton.component_offsets[component],
            &mut positions,
        )?;
        append_quads(
            section,
            header.quads,
            component,
            &quad_vertices,
            &skeleton.component_offsets[component],
            &mut positions,
        )?;
    }

    let bounds = calculate_bounds(&positions)?;
    Ok(StaticPldModel {
        positions,
        component_count,
        stored_vertex_count,
        triangle_count,
        quad_count,
        bounds,
    })
}

fn read_primitive_header(
    section: &[u8],
    at: usize,
    label: &str,
) -> Result<PrimitiveHeader, String> {
    Ok(PrimitiveHeader {
        vertex_offset: read_u32(section, at, label)?,
        vertex_count: read_u32(section, at + 4, label)?,
        normal_offset: read_u32(section, at + 8, label)?,
        normal_count: read_u32(section, at + 12, label)?,
        index_offset: read_u32(section, at + 16, label)?,
        index_count: read_u32(section, at + 20, label)?,
        texture_offset: read_u32(section, at + 24, label)?,
    })
}

fn read_vertices(
    section: &[u8],
    header: PrimitiveHeader,
    label: &str,
    component: usize,
) -> Result<Vec<[i16; 3]>, String> {
    let range = relative_region(
        section,
        header.vertex_offset,
        header.vertex_count,
        8,
        &format!("{label} vertices for MD1 component {component}"),
    )?;
    let mut vertices = Vec::with_capacity(checked_count(header.vertex_count, "MD1 vertex count")?);
    for record in range.as_chunks::<8>().0 {
        vertices.push([
            i16::from_le_bytes([record[0], record[1]]),
            i16::from_le_bytes([record[2], record[3]]),
            i16::from_le_bytes([record[4], record[5]]),
        ]);
    }
    Ok(vertices)
}

fn validate_region(
    section: &[u8],
    header: PrimitiveHeader,
    count_raw: u32,
    stride: usize,
    label: &str,
    component: usize,
) -> Result<(), String> {
    relative_region(
        section,
        match label {
            "triangle normals" | "quad normals" => header.normal_offset,
            "triangle indices" | "quad indices" => header.index_offset,
            "triangle texture records" | "quad texture records" => header.texture_offset,
            _ => return Err(format!("unsupported MD1 region kind {label}")),
        },
        count_raw,
        stride,
        &format!("{label} for MD1 component {component}"),
    )?;
    Ok(())
}

fn append_triangles(
    section: &[u8],
    header: PrimitiveHeader,
    component: usize,
    vertices: &[[i16; 3]],
    bone_offset: &[f32; 3],
    output: &mut Vec<[f32; 3]>,
) -> Result<(), String> {
    let indices = relative_region(
        section,
        header.index_offset,
        header.index_count,
        12,
        &format!("triangle indices for MD1 component {component}"),
    )?;
    for (face, record) in indices.as_chunks::<12>().0.iter().enumerate() {
        for corner in 0..3 {
            let at = corner * 4;
            let normal_index = u16::from_le_bytes([record[at], record[at + 1]]) as usize;
            let vertex_index = u16::from_le_bytes([record[at + 2], record[at + 3]]) as usize;
            validate_index(normal_index, header.normal_count, "normal", component, face)?;
            let vertex = vertices.get(vertex_index).ok_or_else(|| {
                format!(
                    "MD1 component {component} triangle {face} references vertex {vertex_index}, but has {} vertices",
                    vertices.len()
                )
            })?;
            output.push(add_offset(vertex, bone_offset));
        }
    }
    Ok(())
}

fn append_quads(
    section: &[u8],
    header: PrimitiveHeader,
    component: usize,
    vertices: &[[i16; 3]],
    bone_offset: &[f32; 3],
    output: &mut Vec<[f32; 3]>,
) -> Result<(), String> {
    let indices = relative_region(
        section,
        header.index_offset,
        header.index_count,
        16,
        &format!("quad indices for MD1 component {component}"),
    )?;
    // Go's MD1 renderer expands each quad as (v0,v1,v3), then (v0,v2,v3).
    const TRIANGULATED_QUAD_CORNERS: [[usize; 3]; 2] = [[0, 1, 3], [0, 2, 3]];
    for (face, record) in indices.as_chunks::<16>().0.iter().enumerate() {
        let mut vertex_indices = [0_usize; 4];
        for (corner, vertex_index_out) in vertex_indices.iter_mut().enumerate() {
            let at = corner * 4;
            let normal_index = u16::from_le_bytes([record[at], record[at + 1]]) as usize;
            let vertex_index = u16::from_le_bytes([record[at + 2], record[at + 3]]) as usize;
            validate_index(normal_index, header.normal_count, "normal", component, face)?;
            if vertex_index >= vertices.len() {
                return Err(format!(
                    "MD1 component {component} quad {face} references vertex {vertex_index}, but has {} vertices",
                    vertices.len()
                ));
            }
            *vertex_index_out = vertex_index;
        }
        for triangle in TRIANGULATED_QUAD_CORNERS {
            for corner in triangle {
                output.push(add_offset(&vertices[vertex_indices[corner]], bone_offset));
            }
        }
    }
    Ok(())
}

fn validate_index(
    index: usize,
    count_raw: u32,
    kind: &str,
    component: usize,
    face: usize,
) -> Result<(), String> {
    let count = checked_count(count_raw, "MD1 index target count")?;
    if index >= count {
        return Err(format!(
            "MD1 component {component} face {face} references {kind} {index}, but has {count}"
        ));
    }
    Ok(())
}

fn add_offset(vertex: &[i16; 3], offset: &[f32; 3]) -> [f32; 3] {
    [
        f32::from(vertex[0]) + offset[0],
        f32::from(vertex[1]) + offset[1],
        f32::from(vertex[2]) + offset[2],
    ]
}

fn calculate_bounds(positions: &[[f32; 3]]) -> Result<ModelBounds, String> {
    let Some(first) = positions.first() else {
        return Err("MD1 contains no triangle or quad faces to render".to_owned());
    };
    let mut bounds = ModelBounds {
        min: *first,
        max: *first,
    };
    for position in &positions[1..] {
        for (axis, coordinate) in position.iter().enumerate() {
            bounds.min[axis] = bounds.min[axis].min(*coordinate);
            bounds.max[axis] = bounds.max[axis].max(*coordinate);
        }
    }
    Ok(bounds)
}

fn relative_region<'a>(
    section: &'a [u8],
    offset_raw: u32,
    count_raw: u32,
    stride: usize,
    label: &str,
) -> Result<&'a [u8], String> {
    let count = checked_count(count_raw, label)?;
    if count == 0 {
        return Ok(&[]);
    }
    let size = count
        .checked_mul(stride)
        .ok_or_else(|| format!("{label} byte length overflow"))?;
    let start = MD1_HEADER_SIZE
        .checked_add(offset_raw as usize)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let component_count = read_u32(section, 8, "MD1 object count")? as usize / 2;
    let header_end = MD1_HEADER_SIZE
        .checked_add(
            component_count
                .checked_mul(MD1_OBJECT_HEADER_SIZE)
                .ok_or_else(|| "MD1 object-header table size overflow".to_owned())?,
        )
        .ok_or_else(|| "MD1 object-header table end overflow".to_owned())?;
    if start < header_end {
        return Err(format!(
            "{label} starts at {start}, overlapping MD1 headers ending at {header_end}"
        ));
    }
    let end = start
        .checked_add(size)
        .ok_or_else(|| format!("{label} end offset overflow"))?;
    section.get(start..end).ok_or_else(|| {
        format!(
            "{label} range {start}..{end} is outside the {}-byte MD1 section",
            section.len()
        )
    })
}

fn checked_count(value: u32, label: &str) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| format!("{label} does not fit this platform"))
}

fn read_u16(bytes: &[u8], at: usize, label: &str) -> Result<u16, String> {
    let end = at
        .checked_add(2)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let field = bytes
        .get(at..end)
        .ok_or_else(|| format!("truncated {label} at byte {at}"))?;
    Ok(u16::from_le_bytes([field[0], field[1]]))
}

fn read_i16(bytes: &[u8], at: usize, label: &str) -> Result<i16, String> {
    let end = at
        .checked_add(2)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let field = bytes
        .get(at..end)
        .ok_or_else(|| format!("truncated {label} at byte {at}"))?;
    Ok(i16::from_le_bytes([field[0], field[1]]))
}

fn read_u32(bytes: &[u8], at: usize, label: &str) -> Result<u32, String> {
    let end = at
        .checked_add(4)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let field = bytes
        .get(at..end)
        .ok_or_else(|| format!("truncated {label} at byte {at}"))?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

#[cfg(test)]
mod tests {
    use super::{parse_neutral_skeleton, parse_static_pld, MD1_HEADER_SIZE, PLD_HEADER_SIZE};

    fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn minimal_pld() -> Vec<u8> {
        let emr_start = 12;
        let md1_start = 30;
        let tim_start = 170;
        let directory_start = 174;
        let mut bytes = vec![0_u8; directory_start + 16];
        put_u32(&mut bytes, 0, directory_start as u32);
        put_u32(&mut bytes, 4, 4);
        for (i, offset) in [8_u32, emr_start as u32, md1_start as u32, tim_start as u32]
            .iter()
            .enumerate()
        {
            put_u32(&mut bytes, directory_start + i * 4, *offset);
        }

        // One neutral skeleton bone with no children or animation frames.
        put_u16(&mut bytes, emr_start, 14);
        put_u16(&mut bytes, emr_start + 2, 18);
        put_u16(&mut bytes, emr_start + 4, 1);
        put_u16(&mut bytes, emr_start + 6, 12);
        put_u16(&mut bytes, emr_start + 14, 0);
        put_u16(&mut bytes, emr_start + 16, 0);

        // One MD1 triangle. Offsets are relative to the data base at header+12.
        put_u32(&mut bytes, md1_start, 138);
        put_u32(&mut bytes, md1_start + 8, 2);
        let header_at = md1_start + MD1_HEADER_SIZE;
        for (i, value) in [56_u32, 3, 80, 3, 104, 1, 116, 0, 0, 0, 0, 0, 0, 0]
            .iter()
            .enumerate()
        {
            put_u32(&mut bytes, header_at + i * 4, *value);
        }
        let data = md1_start + MD1_HEADER_SIZE + 56;
        let vertex_at = data;
        for (i, vertex) in [[0_i16, 0, 0], [10, 0, 0], [0, 10, 0]].iter().enumerate() {
            for (axis, value) in vertex.iter().enumerate() {
                put_u16(&mut bytes, vertex_at + i * 8 + axis * 2, *value as u16);
            }
        }
        let normal_at = data + 24;
        for i in 0..3 {
            put_u16(&mut bytes, normal_at + i * 8 + 4, 1000);
        }
        let indices_at = data + 48;
        for (corner, index) in [0_u16, 1, 2].iter().enumerate() {
            put_u16(&mut bytes, indices_at + corner * 4, *index);
            put_u16(&mut bytes, indices_at + corner * 4 + 2, *index);
        }
        bytes
    }

    #[test]
    fn expands_a_valid_triangle_with_neutral_bone_transform() {
        let model = parse_static_pld(&minimal_pld()).expect("valid minimal PLD");
        assert_eq!(model.component_count, 1);
        assert_eq!(model.stored_vertex_count, 3);
        assert_eq!(model.triangle_count, 1);
        assert_eq!(model.quad_count, 0);
        assert_eq!(
            model.positions,
            [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]]
        );
    }

    #[test]
    fn rejects_truncated_header_and_directory_offsets() {
        assert!(parse_static_pld(&[0; PLD_HEADER_SIZE - 1]).is_err());
        let mut bytes = minimal_pld();
        put_u32(&mut bytes, 0, u32::MAX);
        assert!(parse_static_pld(&bytes).is_err());
    }

    #[test]
    fn rejects_out_of_bounds_mesh_regions_and_bad_vertex_indices() {
        let mut bytes = minimal_pld();
        let md1_start = 30;
        put_u32(&mut bytes, md1_start + MD1_HEADER_SIZE, u32::MAX);
        assert!(parse_static_pld(&bytes).is_err());

        let mut bytes = minimal_pld();
        let index_record = md1_start + MD1_HEADER_SIZE + 56 + 48;
        put_u16(&mut bytes, index_record + 2, 99);
        assert!(parse_static_pld(&bytes)
            .expect_err("bad vertex index must fail")
            .contains("references vertex 99"));
    }

    #[test]
    fn rejects_truncated_emr_and_invalid_child_references() {
        let mut bytes = minimal_pld();
        put_u32(&mut bytes, 4, 4);
        // Move the MD1 section boundary into the EMR header.
        let directory_start = bytes.len() - 16;
        put_u32(&mut bytes, directory_start + 1 * 4, 29);
        assert!(parse_static_pld(&bytes).is_err());

        let mut emr = vec![0_u8; 29];
        // A two-bone skeleton whose root references child id 2.
        put_u16(&mut emr, 0, 20);
        put_u16(&mut emr, 2, 29);
        put_u16(&mut emr, 4, 2);
        put_u16(&mut emr, 6, 12);
        put_u16(&mut emr, 20, 1);
        put_u16(&mut emr, 22, 8);
        emr[28] = 2;
        assert!(parse_neutral_skeleton(&emr)
            .expect_err("invalid child must fail")
            .contains("out-of-range child"));
    }
}

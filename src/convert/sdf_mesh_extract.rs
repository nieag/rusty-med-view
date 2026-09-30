use crate::convert::{
    signed_distance_from_voxel_data, voxel_index_to_world_mm, SignedDistanceError,
};
use crate::model::{MeshData, MeshFace, MeshVertex, VoxelData};
use std::collections::HashMap;

// Local marching-cubes implementation. The compact case table encodes the
// standard 256 configurations; its representation is deliberately local so
// ROI topology does not depend on a third-party runtime dependency. The table
// encoding is derived from isosurface 0.0.4 (copyright 2018 Tristam MacDonald,
// Apache-2.0); the extraction, geometry, cache, and tests are local code.
const CORNERS: [[u32; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
];

const EDGE_CONNECTION: [[usize; 2]; 12] = [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 0],
    [4, 5],
    [5, 6],
    [6, 7],
    [7, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
];

// Each hexadecimal digit is one triangle-table entry; `f` means terminator.
const TRIANGLE_TABLE_HEX: &str = "ffffffffffffffff083fffffffffffff019fffffffffffff183981ffffffffff12afffffffffffff08312affffffffff92a029ffffffffff2832a8a98fffffff3b2fffffffffffff0b28b0ffffffffff19023bffffffffff1b219b98bfffffff3a1ba3ffffffffff0a108a8bafffffff3903b9ba9fffffff98aa8bffffffffff478fffffffffffff430734ffffffffff019847ffffffffff419471731fffffff12a847ffffffffff34730412afffffff92a902847fffffff2a9297273794ffff8473b2ffffffffffb47b24204fffffff90184723bfffffff47b94b9b2921ffff3a13ba784fffffff1ba14b1047b4ffff47890b9bab03ffff47b4b99bafffffff954fffffffffffff954083ffffffffff054150ffffffffff854835315fffffff12a954ffffffffff30812a495fffffff52a542402fffffff2a5325354348ffff95423bffffffffff0b208b495fffffff05401523bfffffff21525828b485ffffa3ba13954fffffff4950818a18baffff54050b5bab03ffff54858aa8bfffffff978579ffffffffff930953573fffffff078017157fffffff153357ffffffffff978957a12fffffffa12950530573ffff802825857a52ffff2a5253357fffffff7957893b2fffffff95797292027bffff23b018178157ffffb21b17715fffffff958857a13a3bffff5705097b010aba0fba0b03a50807570fba57b5ffffffffffa65fffffffffffff0835a6ffffffffff9015a6ffffffffff1831985a6fffffff165261ffffffffff165126308fffffff965906026fffffff598582526328ffff23ba65ffffffffffb08b20a65fffffff01923b5a6fffffff5a61929b298bffff63b653513fffffff08b0b50515b6ffff3b6036065059ffff65969bb98fffffff5a6478ffffffffff43047365afffffff1905a6847fffffffa65197173794ffff612651478fffffff125526304347ffff847905065026ffff739794329596269f3b2784a65fffffff5a647242027bffff01947823b5a6ffff9219b294b7b45a6f8473b53515b6ffff51b5b610b7b404bf059065036b63847f65969b4797b9ffffa4964affffffffff4a649a083fffffffa01a60640fffffff83181686461affff149124264fffffff308129249264ffff024426ffffffffff832824426fffffffa49a64b23fffffff08228b49a4a6ffff3b201606461affff64161a48121b8b1f964936913b63ffff8b1810b61914641f3b6360064fffffff648b68ffffffffff7a678a89afffffff0730a709a67affffa671a7178180ffffa67a71173fffffff126168189867ffff269291679093739f780706602fffffff732672ffffffffff23ba68a89867ffff20727b09767a9a7f1801781a767a23bfb21b17a61671ffff896867916b63136f091b67ffffffffff7807063b0b60ffff7b6fffffffffffff76bfffffffffffff308b76ffffffffff019b76ffffffffff819831b76fffffffa126b7ffffffffff12a3086b7fffffff2902a96b7fffffff6b72a3a83a98ffff723627ffffffffff708760620fffffff276237019fffffff162186198876ffffa76a17137fffffffa7617a187108ffff03707a0a96a7ffff76a7a88a9fffffff684b86ffffffffff36b306046fffffff86b846901fffffff946963931b36ffff6846b82a1fffffff12a30b06b046ffff4b846b0292a9ffffa93a32943b36463f823842462fffffff042462ffffffffff190234246438ffff194142246fffffff8138618466a1ffffa10a06604fffffff4634386a3039a93fa946a4ffffffffff49576bffffffffff083495b76fffffff50154076bfffffffb76834354315ffff954a1276bfffffff6b712a083495ffff76b54a42a402ffff348354325a52b76f723762549fffffff954086062687ffff362376150540ffff628687218485158f954a16176137ffff16a176107870954f40a4a503a6a737af76a7a854a48affff6956b9b89fffffff36b063056095ffff0b805b01556bffff6b3635531fffffff12a95b9b8b56ffff0b306b09656912afb85b56805a52025f6b36352a3a53ffff589528562382ffff956960062fffffff158180568382628f156216ffffffffff13616a386569896fa10a06950560ffff03856affffffffffa56fffffffffffffb5a75bffffffffffb5ab75830fffffff5b75ab190fffffffa75ab7981831ffffb12b71751fffffff08312717572bffff9759279022b7ffff75272b592328982f25a235375fffffff820852875a25ffff9015a35373a2ffff982921872a25752f135375ffffffffff087071175fffffff903935537fffffff987597ffffffffff5845a8ab8fffffff5045b05abb30ffff01984a8aba45ffffab4a45b34941314f2512852b8458ffff04b0b345b2b151bf0250592b5458b85f9452b3ffffffffff25a352345384ffff5a2524420fffffff3a235a385458019f5a2524192942ffff845853351fffffff045105ffffffffff845853905035ffff945fffffffffffff4b749b9abfffffff0834979b79abffff1ab1b414074bffff3143481a474bab4f4b79b492b912ffff9749b791b2b1083fb74b42240fffffffb74b42834324ffff29a279237749ffff9a7974a27870207f37a3a274a1a040af1a2874ffffffffff491417713fffffff491417081871ffff403743ffffffffff487fffffffffffff9a8ab8ffffffffff30939bb9afffffff01a0a88abfffffff31ab3affffffffff12b1b99b8fffffff30939b1292b9ffff02b80bffffffffff32bfffffffffffff23828aa89fffffff9a2092ffffffffff23828a0181a8ffff1a2fffffffffffff138918ffffffffff091fffffffffffff038fffffffffffffffffffffffffffff";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmoothMeshExtractionError {
    SignedDistance(SignedDistanceError),
    InvalidRawDataLength { expected: usize, actual: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SmoothMeshField {
    /// Padded-grid index of this field's first sample; zero for a whole-volume field.
    origin: [u32; 3],
    /// Dimensions of the sampled block (the padded volume's for a whole-volume field).
    padded_dimensions: [u32; 3],
    values: Vec<f32>,
}

impl SmoothMeshField {
    /// A field with no samples, for a rebuild that has nothing to extract.
    pub fn empty() -> Self {
        Self {
            origin: [0; 3],
            padded_dimensions: [0; 3],
            values: Vec::new(),
        }
    }
}

pub fn extract_smooth_mesh_from_voxel_data(
    voxel_data: &VoxelData,
) -> Result<MeshData, SmoothMeshExtractionError> {
    let dimensions = voxel_data.geometry.dimensions;
    let expected = voxel_count(dimensions);
    if voxel_data.raw_data.len() != expected {
        return Err(SmoothMeshExtractionError::InvalidRawDataLength {
            expected,
            actual: voxel_data.raw_data.len(),
        });
    }
    if dimensions.contains(&0) {
        return Ok(MeshData {
            vertices: Vec::new(),
            faces: Vec::new(),
        });
    }

    let field = build_smooth_mesh_field(voxel_data)?;
    extract_smooth_mesh_chunk_from_field(voxel_data, &field, [0; 3], dimensions)
}

pub fn build_smooth_mesh_field(
    voxel_data: &VoxelData,
) -> Result<SmoothMeshField, SmoothMeshExtractionError> {
    let dimensions = voxel_data.geometry.dimensions;
    let expected = voxel_count(dimensions);
    if voxel_data.raw_data.len() != expected {
        return Err(SmoothMeshExtractionError::InvalidRawDataLength {
            expected,
            actual: voxel_data.raw_data.len(),
        });
    }
    let padded = padded_voxel_data(voxel_data);
    let values = signed_distance_from_voxel_data(&padded)
        .map_err(SmoothMeshExtractionError::SignedDistance)?;
    Ok(SmoothMeshField {
        origin: [0; 3],
        padded_dimensions: padded.geometry.dimensions,
        values,
    })
}

/// The field of just the block of the padded grid from `min_inclusive` to `max_inclusive`
/// (padded indices, where the volume occupies `1..=dimension` and everything else is empty).
///
/// Samples whose nearest opposite voxel lies inside the block equal the whole-volume field
/// exactly, which covers every sample a marching-cubes cell that crosses the surface reads, as
/// long as the block extends past the cells wanted by the largest surface-crossing distance.
pub fn build_smooth_mesh_field_block(
    voxel_data: &VoxelData,
    min_inclusive: [u32; 3],
    max_inclusive: [u32; 3],
) -> Result<SmoothMeshField, SmoothMeshExtractionError> {
    let dimensions = voxel_data.geometry.dimensions;
    let expected = voxel_count(dimensions);
    if voxel_data.raw_data.len() != expected {
        return Err(SmoothMeshExtractionError::InvalidRawDataLength {
            expected,
            actual: voxel_data.raw_data.len(),
        });
    }
    let block_dimensions: [u32; 3] =
        std::array::from_fn(|axis| max_inclusive[axis] - min_inclusive[axis] + 1);
    let mut raw_data = vec![0; voxel_count(block_dimensions)];
    for z in 0..block_dimensions[2] {
        for y in 0..block_dimensions[1] {
            for x in 0..block_dimensions[0] {
                let padded = [
                    min_inclusive[0] + x,
                    min_inclusive[1] + y,
                    min_inclusive[2] + z,
                ];
                let inside_volume =
                    (0..3).all(|axis| padded[axis] >= 1 && padded[axis] <= dimensions[axis]);
                if inside_volume {
                    raw_data[linear_index([x, y, z], block_dimensions)] = voxel_data.raw_data
                        [linear_index(padded.map(|value| value - 1), dimensions)];
                }
            }
        }
    }
    let mut geometry = voxel_data.geometry;
    geometry.dimensions = block_dimensions;
    let values = signed_distance_from_voxel_data(&VoxelData { geometry, raw_data })
        .map_err(SmoothMeshExtractionError::SignedDistance)?;
    Ok(SmoothMeshField {
        origin: min_inclusive,
        padded_dimensions: block_dimensions,
        values,
    })
}

/// The range of cells (padded indices) a chunk covering voxels `min..max` owns.
pub fn smooth_mesh_cell_ranges(
    dimensions: [u32; 3],
    min_inclusive: [u32; 3],
    max_exclusive: [u32; 3],
) -> [std::ops::Range<u32>; 3] {
    std::array::from_fn(|axis| {
        let start = if min_inclusive[axis] == 0 {
            0
        } else {
            min_inclusive[axis] + 1
        };
        let end = if max_exclusive[axis] >= dimensions[axis] {
            dimensions[axis] + 1
        } else {
            max_exclusive[axis] + 1
        };
        start..end
    })
}

pub fn extract_smooth_mesh_chunk_from_field(
    voxel_data: &VoxelData,
    field: &SmoothMeshField,
    min_inclusive: [u32; 3],
    max_exclusive: [u32; 3],
) -> Result<MeshData, SmoothMeshExtractionError> {
    let dimensions = voxel_data.geometry.dimensions;
    let expected = voxel_count(dimensions);
    if voxel_data.raw_data.len() != expected {
        return Err(SmoothMeshExtractionError::InvalidRawDataLength {
            expected,
            actual: voxel_data.raw_data.len(),
        });
    }
    if dimensions.contains(&0) {
        return Ok(MeshData {
            vertices: Vec::new(),
            faces: Vec::new(),
        });
    }
    let mut mesh = MeshData {
        vertices: Vec::new(),
        faces: Vec::new(),
    };
    let mut edge_vertices = HashMap::new();
    let cell_ranges = smooth_mesh_cell_ranges(dimensions, min_inclusive, max_exclusive);
    for z in cell_ranges[2].clone() {
        for y in cell_ranges[1].clone() {
            for x in cell_ranges[0].clone() {
                append_cell(&mut mesh, &mut edge_vertices, [x, y, z], field, voxel_data);
            }
        }
    }
    Ok(mesh)
}

fn padded_voxel_data(voxel_data: &VoxelData) -> VoxelData {
    let dimensions = voxel_data.geometry.dimensions;
    let padded_dimensions = dimensions.map(|value| value + 2);
    let mut raw_data = vec![0; voxel_count(padded_dimensions)];
    for z in 0..dimensions[2] {
        for y in 0..dimensions[1] {
            for x in 0..dimensions[0] {
                raw_data[linear_index([x + 1, y + 1, z + 1], padded_dimensions)] =
                    voxel_data.raw_data[linear_index([x, y, z], dimensions)];
            }
        }
    }
    let mut geometry = voxel_data.geometry;
    geometry.dimensions = padded_dimensions;
    VoxelData { geometry, raw_data }
}

fn append_cell(
    mesh: &mut MeshData,
    edge_vertices: &mut HashMap<EdgeKey, u32>,
    cell: [u32; 3],
    field: &SmoothMeshField,
    original: &VoxelData,
) {
    let corner_indices = CORNERS.map(|offset| add(cell, offset));
    let values = corner_indices.map(|index| {
        let local = std::array::from_fn(|axis| index[axis] - field.origin[axis]);
        field.values[linear_index(local, field.padded_dimensions)]
    });
    let mut case_index = 0usize;
    for (index, value) in values.iter().enumerate() {
        if *value <= 0.0 {
            case_index |= 1 << index;
        }
    }
    for triangle in 0..5 {
        let first = table_edge(case_index, triangle * 3);
        if first < 0 {
            return;
        }
        let edges = [
            first as usize,
            table_edge(case_index, triangle * 3 + 1) as usize,
            table_edge(case_index, triangle * 3 + 2) as usize,
        ];
        let indices = edges.map(|edge| {
            let key = EdgeKey::from_cell_edge(cell, edge);
            *edge_vertices.entry(key).or_insert_with(|| {
                // Adjacent cells may enumerate the same edge in opposite
                // directions. Canonical order keeps chunk seams bit-identical.
                let [mut a, mut b] = EDGE_CONNECTION[edge];
                if corner_indices[a] > corner_indices[b] {
                    std::mem::swap(&mut a, &mut b);
                }
                let offset = interpolation_offset(values[a], values[b]);
                let index = lerp_index(corner_indices[a], corner_indices[b], offset);
                let original_index = index.map(|value| value - 1.0);
                let vertex = MeshVertex {
                    world_mm: voxel_index_to_world_mm(original_index, original.geometry),
                };
                let result = mesh.vertices.len() as u32;
                mesh.vertices.push(vertex);
                result
            })
        });
        mesh.faces.push(MeshFace {
            vertex_indices: [indices[0], indices[2], indices[1]],
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct EdgeKey {
    start: [u32; 3],
    axis: u8,
}

impl EdgeKey {
    fn from_cell_edge(cell: [u32; 3], edge: usize) -> Self {
        let [a, b] = EDGE_CONNECTION[edge];
        let a = add(cell, CORNERS[a]);
        let b = add(cell, CORNERS[b]);
        let start = std::array::from_fn(|axis| a[axis].min(b[axis]));
        let axis = (0..3).find(|axis| a[*axis] != b[*axis]).unwrap() as u8;
        Self { start, axis }
    }
}

fn table_edge(case_index: usize, entry: usize) -> i8 {
    let encoded = TRIANGLE_TABLE_HEX.as_bytes()[case_index * 16 + entry];
    match encoded {
        b'0'..=b'9' => (encoded - b'0') as i8,
        b'a'..=b'e' => (encoded - b'a' + 10) as i8,
        b'f' => -1,
        _ => unreachable!("marching-cubes table only contains hexadecimal digits"),
    }
}

fn interpolation_offset(a: f32, b: f32) -> f32 {
    let delta = b - a;
    if delta.abs() <= f32::EPSILON {
        0.5
    } else {
        (-a / delta).clamp(0.0, 1.0)
    }
}

fn add(a: [u32; 3], b: [u32; 3]) -> [u32; 3] {
    std::array::from_fn(|axis| a[axis] + b[axis])
}

fn lerp_index(a: [u32; 3], b: [u32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] as f32 + (b[axis] as f32 - a[axis] as f32) * t)
}

fn voxel_count(dimensions: [u32; 3]) -> usize {
    dimensions[0] as usize * dimensions[1] as usize * dimensions[2] as usize
}

fn linear_index(index: [u32; 3], dimensions: [u32; 3]) -> usize {
    (index[2] as usize * dimensions[1] as usize + index[1] as usize) * dimensions[0] as usize
        + index[0] as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::voxelize_mesh_to_voxel_data;
    use crate::model::VoxelGeometry;

    fn voxel_data(raw_data: Vec<u8>) -> VoxelData {
        VoxelData {
            geometry: VoxelGeometry::new(
                [3, 3, 3],
                [1.0, 2.0, 3.0],
                [10.0, 20.0, 30.0],
                [0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
            raw_data,
        }
    }

    #[test]
    fn test_smooth_mesh_is_indexed_and_non_empty_for_one_voxel() {
        let mut raw = vec![0; 27];
        raw[linear_index([1, 1, 1], [3, 3, 3])] = 1;
        let mesh = extract_smooth_mesh_from_voxel_data(&voxel_data(raw)).unwrap();

        assert!(!mesh.faces.is_empty());
        assert!(mesh.vertices.len() < mesh.faces.len() * 3);
    }

    #[test]
    fn test_smooth_mesh_roundtrips_a_centered_voxel_label() {
        let mut raw = vec![0; 27];
        raw[linear_index([1, 1, 1], [3, 3, 3])] = 1;
        let source = voxel_data(raw.clone());
        let mesh = extract_smooth_mesh_from_voxel_data(&source).unwrap();
        let rebuilt = voxelize_mesh_to_voxel_data(&mesh, source.geometry).unwrap();

        assert_eq!(rebuilt.raw_data, raw);
    }

    #[test]
    fn test_smooth_mesh_rejects_invalid_raw_data_length() {
        let error = extract_smooth_mesh_from_voxel_data(&voxel_data(vec![1])).unwrap_err();
        assert_eq!(
            error,
            SmoothMeshExtractionError::InvalidRawDataLength {
                expected: 27,
                actual: 1,
            }
        );
    }

    #[test]
    fn test_all_binary_cell_patterns_produce_closed_roundtrippable_meshes() {
        for spacing in [[1.0; 3], [0.7, 1.3, 2.1]] {
            for pattern in 1_u16..256 {
                let source = VoxelData {
                    geometry: VoxelGeometry::new([2; 3], spacing, [0.0; 3], [0.0, 0.0, 0.0, 1.0])
                        .unwrap(),
                    raw_data: (0..8)
                        .map(|bit| u8::from(pattern & (1 << bit) != 0))
                        .collect(),
                };
                let mesh = extract_smooth_mesh_from_voxel_data(&source).unwrap();
                let rebuilt = voxelize_mesh_to_voxel_data(&mesh, source.geometry)
                    .unwrap_or_else(|error| panic!("pattern {pattern:08b}: {error:?}"));
                assert_eq!(rebuilt.raw_data, source.raw_data, "pattern {pattern:08b}");
            }
        }
    }

    #[test]
    fn test_larger_binary_patterns_preserve_closed_mesh_and_voxel_occupancy() {
        // Fixed seed exercises adjacent ambiguous cells without flaky randomness.
        let mut seed = 0x5eed_u32;
        for case in 0..256 {
            let mut source = voxel_data(vec![0; 27]);
            for voxel in &mut source.raw_data {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *voxel = u8::from(seed & 1 != 0);
            }
            let mesh = extract_smooth_mesh_from_voxel_data(&source).unwrap();
            let rebuilt = voxelize_mesh_to_voxel_data(&mesh, source.geometry)
                .unwrap_or_else(|error| panic!("case {case}: {error:?}"));
            assert_eq!(rebuilt.raw_data, source.raw_data, "case {case}");
        }
    }

    #[test]
    #[ignore = "exhaustive topology QA"]
    fn test_adjacent_binary_cells_preserve_closed_mesh_and_voxel_occupancy() {
        let dimensions = [3, 2, 2];
        for pattern in 1_u16..(1 << 12) {
            let source = VoxelData {
                geometry: VoxelGeometry::new(
                    dimensions,
                    [0.7, 1.3, 2.1],
                    [10.0, 20.0, 30.0],
                    [0.0, 0.0, 0.0, 1.0],
                )
                .unwrap(),
                raw_data: (0..12)
                    .map(|bit| u8::from(pattern & (1 << bit) != 0))
                    .collect(),
            };
            let mesh = extract_smooth_mesh_from_voxel_data(&source).unwrap();
            let rebuilt = voxelize_mesh_to_voxel_data(&mesh, source.geometry)
                .unwrap_or_else(|error| panic!("pattern {pattern:012b}: {error:?}"));
            assert_eq!(rebuilt.raw_data, source.raw_data, "pattern {pattern:012b}");
        }
    }

    #[test]
    fn test_marching_cubes_table_contains_all_cases() {
        assert_eq!(TRIANGLE_TABLE_HEX.len(), 256 * 16);
    }
}

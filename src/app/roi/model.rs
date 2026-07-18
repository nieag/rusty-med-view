use crate::convert::{PlaneDefinition, PlaneFamily};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoiId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryRepresentation {
    Voxel,
    Contour,
    Mesh,
}

#[derive(Debug, Clone)]
pub struct RoiMetadata {
    pub roi_id: RoiId,
    pub name: String,
    pub is_visible: bool,
    pub is_locked: bool,
    pub color: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoxelData {
    pub geometry: VoxelGeometry,
    pub raw_data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelGeometry {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub origin: [f32; 3],
    pub orientation: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourPoint {
    pub local_mm: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourLoop {
    pub points: Vec<ContourPoint>,
    pub is_closed: bool,
}

impl ContourLoop {
    pub fn is_valid_closed_loop(&self) -> bool {
        self.is_closed && self.points.len() >= 3
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourSlice {
    pub plane: PlaneDefinition,
    pub loops: Vec<ContourLoop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourData {
    pub active_plane_family: PlaneFamily,
    pub slices: Vec<ContourSlice>,
}

impl ContourData {
    pub fn is_empty(&self) -> bool {
        self.slices.is_empty()
    }

    pub fn has_loops(&self) -> bool {
        self.slices.iter().any(|slice| !slice.loops.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshVertex {
    pub world_mm: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshFace {
    pub vertex_indices: [u32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshData {
    pub vertices: Vec<MeshVertex>,
    pub faces: Vec<MeshFace>,
}

pub enum RoiAuthoritativeData {
    Voxel(VoxelData),
    Contour(ContourData),
    Mesh(MeshData),
}

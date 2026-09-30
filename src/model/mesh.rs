//! Triangle surface data in world millimetres.

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

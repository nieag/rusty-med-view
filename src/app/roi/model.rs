pub use crate::model::{
    ContourData, ContourLoop, ContourPoint, ContourSlice, MeshData, MeshFace, MeshVertex,
    VoxelData, VoxelGeometry, VoxelGeometryError,
};

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

/// Spawns a ROI with the components every listed ROI carries.
pub fn spawn_roi_layer(
    world: &mut hecs::World,
    (roi, metadata): (crate::app::components::Roi, RoiMetadata),
    opacity: f32,
) -> hecs::Entity {
    world.spawn((
        roi,
        metadata,
        crate::app::components::LayerSettings { opacity },
        crate::app::components::RoiTag,
    ))
}

/// Whether a ROI is shown; a ROI without metadata is not.
pub fn is_roi_visible(world: &hecs::World, entity: hecs::Entity) -> bool {
    world
        .get::<&RoiMetadata>(entity)
        .is_ok_and(|metadata| metadata.is_visible)
}

/// Whether a ROI is locked against edits.
pub fn is_roi_locked(world: &hecs::World, entity: hecs::Entity) -> bool {
    world
        .get::<&RoiMetadata>(entity)
        .is_ok_and(|metadata| metadata.is_locked)
}

impl RoiMetadata {
    pub fn new(roi_id: RoiId, name: String) -> Self {
        Self {
            roi_id,
            name,
            is_visible: true,
            is_locked: false,
            color: [1.0, 0.2, 0.2, 1.0],
        }
    }
}

/// Preview of a contour point drag that has not been committed.
#[derive(Debug, Clone, PartialEq)]
pub struct ContourMovePreview {
    pub contour_data: ContourData,
}

/// Preview of a mesh deformation that has not been committed.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshEditPreview {
    pub mesh_data: MeshData,
    /// Welded topology of the mesh being deformed, built once when the drag starts and reused by
    /// every preview update of that drag.
    pub deform_base: Option<std::sync::Arc<crate::convert::MeshDeformBase>>,
}

/// A voxel ROI: a read-only source such as a model prediction. It has no edit operations and
/// therefore no preview.
#[derive(Debug, Clone, PartialEq)]
pub struct VoxelBody {
    pub data: VoxelData,
}

/// A contour ROI and its in-flight point drag, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct ContourBody {
    pub data: ContourData,
    pub preview: Option<ContourMovePreview>,
}

impl ContourBody {
    pub fn new(data: ContourData) -> Self {
        Self {
            data,
            preview: None,
        }
    }
}

/// A mesh ROI and its in-flight deformation, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshBody {
    pub data: MeshData,
    pub preview: Option<MeshEditPreview>,
}

impl MeshBody {
    pub fn new(data: MeshData) -> Self {
        Self {
            data,
            preview: None,
        }
    }
}

/// The authoritative representation of an ROI, with the edit state that only that
/// representation has: a contour drag cannot exist on a mesh ROI, and a voxel ROI cannot be
/// edited at all.
// `VoxelGeometry` is a `Copy` affine plus its cached inverse, stored inline on purpose: there are
// only a handful of ROIs, and recomputing the inverse in the mapping hot paths would cost more
// than the size imbalance between variants.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum RoiBody {
    Voxel(VoxelBody),
    Contour(ContourBody),
    Mesh(MeshBody),
}

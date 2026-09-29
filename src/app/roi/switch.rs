//! Automatic primary-representation switching (ADR 0004).
//!
//! Editing tools call [`ensure_editable`] with what they are about to edit. If the ROI is not
//! already authoritative in that form, the ROI is converted (one undo step) or, when the derived
//! data needed for the conversion is still being built, the switch is left pending and completed
//! by [`complete_pending_switches`] once that data is current. Users never choose the switch.

use crate::app::components::{
    PrimaryRepresentation, Revision, Roi, RoiBody, RoiCacheKind, RoiEditSnapshot, RoiJobKind,
};
use crate::app::roi::authority::{
    promote_current_mesh_cache_to_authority, promote_roi_to_contour_authority,
    request_mesh_voxel_cache_rebuild, MeshAuthorityPromotionError, VoxelContourPromotionError,
};
use crate::app::roi::history::record_authority_change;
use crate::convert::PlaneFamily;
use hecs::World;

/// What an editing tool needs the ROI to be authoritative in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTarget {
    /// Contours of one orthogonal plane family.
    Contour(PlaneFamily),
    Mesh,
}

/// A switch waiting for derived data. Discarded if the ROI's shape changes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingSwitch {
    pub target: EditTarget,
    pub source: Revision,
}

/// What a conversion did, for the status area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversionReport {
    pub from: PrimaryRepresentation,
    pub to: PrimaryRepresentation,
    pub from_family: Option<PlaneFamily>,
    pub to_family: Option<PlaneFamily>,
    /// Whether the new form represents exactly the same shape (a switch between contour
    /// families, or voxel to contour). Conversions that resample a surface are not lossless.
    pub lossless: bool,
}

impl ConversionReport {
    pub fn message(&self) -> String {
        let form = |kind, family: Option<PlaneFamily>| match (kind, family) {
            (PrimaryRepresentation::Contour, Some(family)) => format!("{family:?} contours"),
            (PrimaryRepresentation::Contour, None) => "contours".to_string(),
            (PrimaryRepresentation::Voxel, _) => "voxels".to_string(),
            (PrimaryRepresentation::Mesh, _) => "mesh".to_string(),
        };
        let loss = if self.lossless {
            ""
        } else {
            " (resampled: fine detail may change; Undo restores the original)"
        };
        format!(
            "Converted {} to {}{loss}.",
            form(self.from, self.from_family),
            form(self.to, self.to_family)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// The ROI is already authoritative in the requested form.
    Ready,
    /// The ROI was converted just now, as one undo step.
    Switched(ConversionReport),
    /// Derived data is still being built; the switch completes on its own. The gesture that
    /// asked for it is ignored.
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchError {
    MissingRoi,
    Locked,
    /// Oblique views are derived per-slice views and cannot be edited.
    UnsupportedTarget,
    SourceUnavailable,
    Contour(VoxelContourPromotionError),
    Mesh(MeshAuthorityPromotionError),
    MeshInvalid,
}

impl SwitchError {
    pub fn message(&self) -> String {
        match self {
            Self::MissingRoi => "The ROI no longer exists.".to_string(),
            Self::Locked => "The ROI is locked.".to_string(),
            Self::UnsupportedTarget => "Oblique views cannot be edited.".to_string(),
            Self::SourceUnavailable => "The ROI could not be prepared for editing.".to_string(),
            Self::Contour(error) => format!("Contour conversion failed: {error:?}."),
            Self::Mesh(error) => format!("Mesh conversion failed: {error:?}."),
            Self::MeshInvalid => "The mesh is not a closed surface.".to_string(),
        }
    }
}

/// Makes the ROI authoritative in `target`, converting or scheduling the conversion if needed.
pub fn ensure_editable(
    world: &mut World,
    roi_entity: hecs::Entity,
    target: EditTarget,
) -> Result<Readiness, SwitchError> {
    if target == EditTarget::Contour(PlaneFamily::Oblique) {
        return Err(SwitchError::UnsupportedTarget);
    }
    let (locked, already, pending, source_revision) = {
        let roi = world
            .get::<&Roi>(roi_entity)
            .map_err(|_| SwitchError::MissingRoi)?;
        let already = match (&roi.body, target) {
            (RoiBody::Contour(body), EditTarget::Contour(family)) => {
                body.data.active_plane_family == family
            }
            (RoiBody::Mesh(_), EditTarget::Mesh) => true,
            _ => false,
        };
        (
            roi.metadata.is_locked,
            already,
            roi.job_state.pending_switch,
            roi.dirty_state.authoritative,
        )
    };
    if already {
        set_pending(world, roi_entity, None);
        return Ok(Readiness::Ready);
    }
    if locked {
        return Err(SwitchError::Locked);
    }
    let wanted = PendingSwitch {
        target,
        source: source_revision,
    };
    if pending == Some(wanted) {
        return Ok(Readiness::Pending);
    }

    let ready = match target {
        EditTarget::Contour(_) => voxel_source_is_ready(world, roi_entity),
        EditTarget::Mesh => mesh_source_is_ready(world, roi_entity),
    };
    if !ready {
        request_source(world, roi_entity, target)?;
        set_pending(world, roi_entity, Some(wanted));
        return Ok(Readiness::Pending);
    }

    let report = convert(world, roi_entity, target)?;
    set_pending(world, roi_entity, None);
    Ok(Readiness::Switched(report))
}

/// Finishes every pending switch whose derived data has become current. Returns the outcome per
/// ROI so the caller can report it. A pending switch whose ROI changed in the meantime, or whose
/// data can no longer arrive, is dropped.
pub fn complete_pending_switches(
    world: &mut World,
) -> Vec<(hecs::Entity, Result<ConversionReport, SwitchError>)> {
    let pending = world
        .query::<&Roi>()
        .iter()
        .filter_map(|(entity, roi)| roi.job_state.pending_switch.map(|p| (entity, p)))
        .collect::<Vec<_>>();
    let mut outcomes = Vec::new();
    for (entity, switch) in pending {
        let (stale, has_work) = match world.get::<&Roi>(entity) {
            Ok(roi) => (
                roi.dirty_state.authoritative.shape != switch.source.shape,
                roi.running_job_kind().is_some() || !roi.job_state.pending.is_empty(),
            ),
            Err(_) => continue,
        };
        if stale {
            set_pending(world, entity, None);
            continue;
        }
        let ready = match switch.target {
            EditTarget::Contour(_) => voxel_source_is_ready(world, entity),
            EditTarget::Mesh => mesh_source_is_ready(world, entity),
        };
        if ready {
            set_pending(world, entity, None);
            outcomes.push((entity, convert(world, entity, switch.target)));
        } else if !has_work {
            set_pending(world, entity, None);
            outcomes.push((entity, Err(SwitchError::SourceUnavailable)));
        }
    }
    outcomes
}

fn set_pending(world: &mut World, roi_entity: hecs::Entity, pending: Option<PendingSwitch>) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.job_state.pending_switch = pending;
    }
}

/// A voxel form of the ROI's current shape exists: the body itself for a voxel ROI, else a
/// current voxel cache.
fn voxel_source_is_ready(world: &World, roi_entity: hecs::Entity) -> bool {
    world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
        matches!(roi.body, RoiBody::Voxel(_))
            || (roi.voxel_cache().is_some() && roi.is_cache_current(RoiCacheKind::Voxel))
    })
}

fn mesh_source_is_ready(world: &World, roi_entity: hecs::Entity) -> bool {
    world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
        roi.mesh_cache()
            .is_some_and(|mesh| !mesh.data.vertices.is_empty() && !mesh.data.faces.is_empty())
            && roi.is_cache_current(RoiCacheKind::Mesh)
    })
}

/// Queues whatever builds the derived data the conversion needs.
fn request_source(
    world: &mut World,
    roi_entity: hecs::Entity,
    target: EditTarget,
) -> Result<(), SwitchError> {
    let is_mesh_body = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| SwitchError::MissingRoi)?
        .mesh_data()
        .is_some();
    match target {
        EditTarget::Contour(_) if is_mesh_body => {
            request_mesh_voxel_cache_rebuild(world, roi_entity)
                .map_err(|_| SwitchError::MeshInvalid)
        }
        EditTarget::Contour(_) => {
            let mut roi = world
                .get::<&mut Roi>(roi_entity)
                .map_err(|_| SwitchError::MissingRoi)?;
            roi.mark_cache_dirty(RoiCacheKind::Voxel);
            roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
            Ok(())
        }
        EditTarget::Mesh => {
            if !voxel_source_is_ready(world, roi_entity) {
                return request_source(world, roi_entity, EditTarget::Contour(PlaneFamily::Axial));
            }
            let mut roi = world
                .get::<&mut Roi>(roi_entity)
                .map_err(|_| SwitchError::MissingRoi)?;
            roi.mark_cache_dirty(RoiCacheKind::Mesh);
            roi.enqueue_rebuild(RoiJobKind::RebuildMeshCache);
            Ok(())
        }
    }
}

/// The body as an undo snapshot, or `None` for an ROI that no longer exists.
fn snapshot_of_body(world: &World, roi_entity: hecs::Entity) -> Option<RoiEditSnapshot> {
    let roi = world.get::<&Roi>(roi_entity).ok()?;
    Some(match &roi.body {
        RoiBody::Voxel(body) => RoiEditSnapshot::Voxel(Box::new(body.data.clone())),
        RoiBody::Contour(body) => RoiEditSnapshot::Contour(body.data.clone()),
        RoiBody::Mesh(body) => RoiEditSnapshot::Mesh(body.data.clone()),
    })
}

/// Runs the conversion once its source is ready and records it as one undo step.
fn convert(
    world: &mut World,
    roi_entity: hecs::Entity,
    target: EditTarget,
) -> Result<ConversionReport, SwitchError> {
    let previous = snapshot_of_body(world, roi_entity).ok_or(SwitchError::MissingRoi)?;
    let (from, from_family) = {
        let roi = world
            .get::<&Roi>(roi_entity)
            .map_err(|_| SwitchError::MissingRoi)?;
        (
            roi.primary_representation(),
            roi.contour_data().map(|data| data.active_plane_family),
        )
    };
    let report = match target {
        EditTarget::Contour(family) => {
            promote_roi_to_contour_authority(world, roi_entity, family).map_err(
                |error| match error {
                    VoxelContourPromotionError::Locked => SwitchError::Locked,
                    other => SwitchError::Contour(other),
                },
            )?;
            ConversionReport {
                from,
                to: PrimaryRepresentation::Contour,
                from_family,
                to_family: Some(family),
                lossless: from != PrimaryRepresentation::Mesh,
            }
        }
        EditTarget::Mesh => {
            promote_current_mesh_cache_to_authority(world, roi_entity).map_err(
                |error| match error {
                    MeshAuthorityPromotionError::Locked => SwitchError::Locked,
                    other => SwitchError::Mesh(other),
                },
            )?;
            ConversionReport {
                from,
                to: PrimaryRepresentation::Mesh,
                from_family,
                to_family: None,
                lossless: false,
            }
        }
    };
    record_authority_change(world, roi_entity, previous);
    Ok(report)
}

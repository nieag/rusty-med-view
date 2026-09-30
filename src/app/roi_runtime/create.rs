use super::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelRoiImportSpec {
    pub geometry: VoxelGeometry,
    pub start_visible: bool,
    pub geometry_matches_main: bool,
}

pub fn prepare_voxel_roi_import(
    world: &World,
    loaded_label: &LoadedLabel,
) -> Result<VoxelRoiImportSpec, String> {
    // The loader already validated the geometry (finite, non-singular affine), so the ROI
    // constructor cannot fail on it.
    let geometry = loaded_label.geometry;
    let geometry_matches_main = if let Some(main_geometry) = main_volume_geometry(world) {
        let differs = main_geometry.identity() != geometry.identity();
        if differs {
            log::warn!(
                "Loaded label geometry differs from main volume geometry; preserving label-owned geometry. label dims={:?} spacing={:?} origin={:?}, main dims={:?} spacing={:?} origin={:?}",
                geometry.dimensions,
                geometry.spacing(),
                geometry.origin(),
                main_geometry.dimensions,
                main_geometry.spacing(),
                main_geometry.origin(),
            );
        }
        !differs
    } else {
        true
    };

    Ok(VoxelRoiImportSpec {
        geometry,
        start_visible: visible_voxel_overlay_count(world) < MAX_SIMULTANEOUS_ROI_OVERLAYS,
        geometry_matches_main,
    })
}

/// Imports a labelmap as one voxel ROI per non-zero label, so structures such as a liver and its
/// tumor stay separate through every later conversion. A map with no labels becomes one empty ROI.
///
/// The first `MAX_SIMULTANEOUS_ROI_OVERLAYS` ROIs start visible; the rest are created hidden.
pub fn create_voxel_rois_from_label(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded_label: &LoadedLabel,
) -> Result<Vec<hecs::Entity>, String> {
    let import_spec = prepare_voxel_roi_import(world, loaded_label)?;
    let masks = label_masks_for_import(&loaded_label.data)?;

    let placeholder_bg = world
        .query::<&GpuVolumeResources>()
        .with::<&MainVolumeTag>()
        .iter()
        .next()
        .map(|(_, res)| res.bind_group.clone())
        .ok_or_else(|| {
            "Cannot create a label ROI without an initialized main volume resource".to_string()
        })?;
    let dimensions = import_spec.geometry.dimensions();

    spawn_label_rois(
        world,
        import_spec.geometry,
        &loaded_label.filename,
        masks,
        |mask_bytes| {
            let (texture, view, sampler) = crate::io::volume::create_r8_texture_from_label_bytes(
                device,
                queue,
                dimensions,
                mask_bytes,
                "NIfTI Labelmap",
            )?;
            Ok(Some(GpuVolumeResources {
                texture,
                view,
                sampler,
                bind_group: placeholder_bg.clone(),
            }))
        },
    )
}

/// One mask per non-zero label, or a single all-zero mask for a map without labels. Fails when
/// the split would exceed the import memory budget.
pub(super) fn label_masks_for_import(data: &[u8]) -> Result<Vec<LabelMask>, String> {
    let labels = present_label_ids(data);
    check_label_import_budget(data.len(), labels.len())?;
    if labels.is_empty() {
        return Ok(vec![LabelMask {
            label: 0,
            data: data.to_vec(),
        }]);
    }
    Ok(split_labelmap(data, &labels))
}

/// Spawns one ROI entity per mask. `gpu_for_mask` builds the GPU mirror for a mask (`None` when
/// there is no GPU), which keeps the world-building logic testable without a device.
pub(super) fn spawn_label_rois(
    world: &mut World,
    geometry: VoxelGeometry,
    filename: &str,
    masks: Vec<LabelMask>,
    mut gpu_for_mask: impl FnMut(&[u8]) -> Result<Option<GpuVolumeResources>, String>,
) -> Result<Vec<hecs::Entity>, String> {
    let already_visible = visible_voxel_overlay_count(world);
    let label_count = masks.len();
    let mut entities = Vec::with_capacity(label_count);
    for (index, mask) in masks.into_iter().enumerate() {
        let gpu_resources = gpu_for_mask(&mask.data)?;
        let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
        let (roi, mut metadata) = Roi::new_voxel_with_cache(
            RoiId(next_roi_id),
            label_roi_name(filename, mask.label, label_count),
            geometry,
            mask.data,
            gpu_resources,
        );
        metadata.is_visible = already_visible + index < MAX_SIMULTANEOUS_ROI_OVERLAYS;
        if mask.label != 0 {
            metadata.color = label_color(mask.label);
        }
        entities.push(crate::app::roi::spawn_roi_layer(
            world,
            (roi, metadata),
            0.5,
        ));
    }
    Ok(entities)
}

/// Creates an empty contour ROI in `active_plane_family` on the main volume's grid and makes it
/// the active ROI.
pub fn create_empty_contour_roi(
    world: &mut World,
    editor: &mut EditorState,
    active_plane_family: OrthogonalFamily,
) -> Result<hecs::Entity, String> {
    let reference_voxel_geometry = main_volume_geometry(world)
        .ok_or_else(|| "Missing main volume geometry; contour ROI was not created.".to_string())?;
    let reference_geometry = reference_voxel_geometry;
    let voxel_count = reference_voxel_geometry
        .dimensions
        .into_iter()
        .try_fold(1usize, |count, dimension| {
            count.checked_mul(dimension as usize)
        })
        .ok_or_else(|| "Contour ROI reference grid is too large.".to_string())?;

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let roi_name = format!("Contour ROI {}", next_roi_id);
    let entity = crate::app::roi::spawn_roi_layer(
        world,
        Roi::new_contour_with_geometry(
            RoiId(next_roi_id),
            roi_name,
            reference_geometry,
            ContourData {
                active_plane_family,
                slices: Vec::new(),
            },
        ),
        0.5,
    );

    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // The empty cache is valid for the initial empty contour authority. Its generation lets
        // the first changed-slice commit use the incremental slab rasterizer.
        let generation = roi.dirty_state.authoritative.shape;
        roi.install_voxel_cache_result(
            VoxelCache {
                data: VoxelData {
                    geometry: reference_voxel_geometry,
                    raw_data: vec![0; voxel_count],
                },
                gpu_resources: None,
            },
            generation,
        )
        .expect("new contour ROI cache must match its reference geometry");
    }

    editor.active_roi = Some(entity);
    Ok(entity)
}

#[cfg(test)]
pub fn create_contour_roi_from_voxel_roi(
    world: &mut World,
    source_roi: hecs::Entity,
    family: OrthogonalFamily,
) -> Result<hecs::Entity, VoxelContourCreationError> {
    let source_voxel = {
        let roi = world
            .get::<&Roi>(source_roi)
            .map_err(|_| VoxelContourCreationError::MissingRoi)?;
        match &roi.body {
            RoiBody::Voxel(VoxelBody { data: voxel }) => voxel.clone(),
            RoiBody::Contour(_) | RoiBody::Mesh(_) => {
                return Err(VoxelContourCreationError::NotVoxelRoi);
            }
        }
    };

    let extracted = extract_contours_from_voxel_data(&source_voxel, family)
        .map_err(VoxelContourCreationError::ExtractionFailed)?;

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let source_name = world
        .get::<&RoiMetadata>(source_roi)
        .ok()
        .map(|metadata| metadata.name.clone())
        .unwrap_or_else(|| "Voxel ROI".to_string());
    let new_name = format!(
        "{source_name} ({} Contour)",
        plane_family_label(family.into())
    );
    let reference_geometry = source_voxel.geometry;

    let entity = crate::app::roi::spawn_roi_layer(
        world,
        Roi::new_contour_with_geometry(RoiId(next_roi_id), new_name, reference_geometry, extracted),
        0.5,
    );
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // Preserve source voxel geometry as the initial contour reference frame.
        // This keeps extracted contour projection/edit mapping aligned before any
        // contour->voxel rebuild retargets caches to main-volume geometry.
        let generation = roi.dirty_state.authoritative.shape;
        let _ = roi.install_voxel_cache_result(
            VoxelCache {
                data: source_voxel,
                gpu_resources: None,
            },
            generation,
        );
    }
    Ok(entity)
}

#[cfg(test)]
pub fn create_mesh_roi_from_voxel_roi(
    world: &mut World,
    source_roi: hecs::Entity,
) -> Result<hecs::Entity, VoxelMeshCreationError> {
    let source_voxel =
        voxel_data_for_display_surface_extraction(world, source_roi).map_err(|err| match err {
            DisplayVoxelSourceError::MissingRoi => VoxelMeshCreationError::MissingRoi,
            DisplayVoxelSourceError::NotVoxelRoi => VoxelMeshCreationError::NotVoxelRoi,
        })?;

    let source_has_occupancy = source_voxel.raw_data.iter().any(|value| *value != 0);
    let extracted = extract_mesh_from_voxel_data(&source_voxel)
        .map_err(VoxelMeshCreationError::ExtractionFailed)?;
    if source_has_occupancy && (extracted.vertices.is_empty() || extracted.faces.is_empty()) {
        return Err(VoxelMeshCreationError::EmptyMeshFromNonEmptySource);
    }

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let source_name = world
        .get::<&RoiMetadata>(source_roi)
        .ok()
        .map(|metadata| metadata.name.clone())
        .unwrap_or_else(|| "Voxel ROI".to_string());
    let entity = crate::app::roi::spawn_roi_layer(
        world,
        Roi::new_mesh_with_geometry(
            RoiId(next_roi_id),
            format!("{source_name} (Mesh)"),
            source_voxel.geometry,
            extracted,
        ),
        0.5,
    );
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // Preserve source voxel geometry as extraction/provenance context.
        // Rendering projects mesh world-mm vertices through main display volume geometry.
        roi.store_stale_voxel_cache(VoxelCache {
            data: source_voxel,
            gpu_resources: None,
        });
    }
    Ok(entity)
}

#[cfg(test)]
pub fn voxel_data_for_display_surface_extraction(
    world: &World,
    source_roi: hecs::Entity,
) -> Result<VoxelData, DisplayVoxelSourceError> {
    let source_voxel = {
        let roi = world
            .get::<&Roi>(source_roi)
            .map_err(|_| DisplayVoxelSourceError::MissingRoi)?;
        match &roi.body {
            RoiBody::Voxel(VoxelBody { data: voxel }) => voxel.clone(),
            RoiBody::Contour(_) | RoiBody::Mesh(_) => {
                return Err(DisplayVoxelSourceError::NotVoxelRoi);
            }
        }
    };

    Ok(source_voxel)
}

#[cfg(test)]
pub fn create_mesh_roi_from_contour_roi(
    world: &mut World,
    source_roi: hecs::Entity,
) -> Result<hecs::Entity, ContourMeshCreationError> {
    let source_voxel = {
        let roi = world
            .get::<&Roi>(source_roi)
            .map_err(|_| ContourMeshCreationError::MissingRoi)?;
        if !matches!(&roi.body, RoiBody::Contour(_)) {
            return Err(ContourMeshCreationError::NotContourRoi);
        }
        let Some(cache) = roi.voxel_cache() else {
            return Err(ContourMeshCreationError::MissingCurrentVoxelCache);
        };
        if !roi.is_cache_current(RoiCacheKind::Voxel) {
            return Err(ContourMeshCreationError::MissingCurrentVoxelCache);
        }
        cache.data.clone()
    };

    let extracted = extract_mesh_from_voxel_data(&source_voxel)
        .map_err(ContourMeshCreationError::ExtractionFailed)?;

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let source_name = world
        .get::<&RoiMetadata>(source_roi)
        .ok()
        .map(|metadata| metadata.name.clone())
        .unwrap_or_else(|| "Contour ROI".to_string());
    let entity = crate::app::roi::spawn_roi_layer(
        world,
        Roi::new_mesh_with_geometry(
            RoiId(next_roi_id),
            format!("{source_name} (Mesh)"),
            source_voxel.geometry,
            extracted,
        ),
        0.5,
    );
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // Preserve contour-derived voxel geometry as extraction/provenance context.
        // Rendering projects mesh world-mm vertices through main display volume geometry.
        roi.store_stale_voxel_cache(VoxelCache {
            data: source_voxel,
            gpu_resources: None,
        });
    }
    Ok(entity)
}

use crate::components::*;
use crate::convert::{world_mm_to_volume_uv, PlaneFamily};
use crate::AppEvent;
use hecs::World;
use winit::event_loop::EventLoopProxy;

use crate::app::{roi, roi_runtime};
use crate::io::handlers;

fn activate_promoted_edit_tool(world: &mut World, entities: &AppEntities, tool: EditorTool) {
    let active_roi = world
        .get::<&EditorState>(entities.editor)
        .ok()
        .and_then(|editor| editor.active_roi);
    if let Some(roi_entity) = active_roi {
        roi::clear_roi_edit_history_for_roi(world, roi_entity);
    }
    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        input.contour_move_pending_commit = false;
        input.mesh_move_pending_commit = false;
    }
    roi::cancel_roi_edit_preview(world, entities.editor);
    if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
        editor.active_tool = tool;
        editor.contour_draft = None;
        editor.contour_selection = None;
        editor.mesh_selection = None;
    }
}

fn active_viewport_contour_view_key(
    world: &World,
    entities: &AppEntities,
    roi_entity: hecs::Entity,
) -> Option<ContourViewKey> {
    let active_viewport = world
        .get::<&InputState>(entities.input)
        .ok()
        .and_then(|input| input.active_viewport)?;
    let viewport = world.get::<&Viewport>(active_viewport).ok()?;
    let viewport_state = world.get::<&ViewportState>(active_viewport).ok()?;
    let cursor_uv = world
        .get::<&Transform>(entities.cursor)
        .map(|cursor| cursor.position)
        .unwrap_or([0.5, 0.5, 0.5]);
    let geometry = roi_runtime::main_volume_geometry(world).or_else(|| {
        world
            .get::<&Roi>(roi_entity)
            .ok()
            .and_then(|roi| roi.voxel_cache().map(|cache| cache.data.geometry))
    })?;
    let plane = crate::render::roi_views::displayed_plane_for_viewport(
        viewport.mode,
        cursor_uv,
        viewport_state.user_rotation,
        geometry,
    )?;
    Some(ContourViewKey::from_plane(plane))
}

fn promote_contour_family_for_active_view(
    world: &mut World,
    entities: &AppEntities,
    roi_entity: hecs::Entity,
    family: PlaneFamily,
) -> Result<(), String> {
    if family != PlaneFamily::Oblique {
        return roi::promote_roi_to_contour_authority(world, roi_entity, family)
            .map_err(|error| format!("{error:?}"));
    }

    let view_key = active_viewport_contour_view_key(world, entities, roi_entity)
        .filter(|key| key.family == PlaneFamily::Oblique)
        .ok_or_else(|| "activate an oblique 2D viewport before promotion".to_string())?;
    roi::promote_contour_view_to_authoritative(world, roi_entity, &view_key)
        .map_err(|error| format!("{error:?}"))
}

fn focus_cursor_on_first_extracted_slice(
    world: &mut World,
    entities: &AppEntities,
    roi_entity: hecs::Entity,
) {
    let Some((family, origin_mm, roi_geometry)) = (|| {
        let roi = world.get::<&Roi>(roi_entity).ok()?;
        let contour = roi.contour_data()?;
        let slice = contour.slices.first()?;
        let geometry = roi.voxel_cache().map(|cache| cache.data.geometry)?;
        Some((contour.active_plane_family, slice.plane.origin_mm, geometry))
    })() else {
        let Some((family, origin_mm, fallback_geometry)) = (|| {
            let roi = world.get::<&Roi>(roi_entity).ok()?;
            let contour = roi.contour_data()?;
            let slice = contour.slices.first()?;
            let mut query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
            let (_, volume) = query.iter().next()?;
            Some((
                contour.active_plane_family,
                slice.plane.origin_mm,
                volume.geometry?,
            ))
        })() else {
            return;
        };
        let slice_uv = world_mm_to_volume_uv(origin_mm, fallback_geometry);
        if let Ok(mut cursor) = world.get::<&mut Transform>(entities.cursor) {
            match family {
                PlaneFamily::Axial => cursor.position[2] = slice_uv[2],
                PlaneFamily::Coronal => cursor.position[1] = slice_uv[1],
                PlaneFamily::Sagittal => cursor.position[0] = slice_uv[0],
                PlaneFamily::Oblique => {}
            }
        }
        return;
    };

    let slice_uv = world_mm_to_volume_uv(origin_mm, roi_geometry);
    if let Ok(mut cursor) = world.get::<&mut Transform>(entities.cursor) {
        match family {
            PlaneFamily::Axial => cursor.position[2] = slice_uv[2],
            PlaneFamily::Coronal => cursor.position[1] = slice_uv[1],
            PlaneFamily::Sagittal => cursor.position[0] = slice_uv[0],
            PlaneFamily::Oblique => {}
        }
    }
}

pub fn draw_sidebar(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    world: &mut World,
    entities: &AppEntities,
    event_proxy: &EventLoopProxy<AppEvent>,
    volume_info: Option<[u32; 3]>,
) {
    ui.add_space(8.0);

    ui.collapsing("📁 Protocol", |ui| {
        if let Ok(proto) = world.get::<&ProtocolState>(entities.protocol) {
            let mut selected = proto.active_protocol.clone();
            let registry = crate::render::protocols::get_protocol_registry();
            egui::ComboBox::from_label("Active Protocol")
                .selected_text(&selected)
                .show_ui(ui, |ui| {
                    for p in registry {
                        if ui
                            .selectable_value(&mut selected, p.name.clone(), &p.name)
                            .clicked()
                        {
                            ctx.request_repaint();
                        }
                    }
                });

            if selected != proto.active_protocol {
                let _ = event_proxy.send_event(AppEvent::SwitchProtocol(selected));
                ctx.request_repaint();
            }
        }
    });

    ui.separator();

    // --- Volume Info ---
    ui.collapsing("📊 Volume Info", |ui| {
        if let Some(dims) = volume_info {
            ui.label(format!("Dimensions: {}×{}×{}", dims[0], dims[1], dims[2]));
            if let Some((_, vd)) = world
                .query::<&VolumeData>()
                .with::<&MainVolumeTag>()
                .iter()
                .next()
            {
                ui.label(format!(
                    "Spacing: {:.2}×{:.2}×{:.2} mm",
                    vd.spacing()[0],
                    vd.spacing()[1],
                    vd.spacing()[2]
                ));
                ui.label(format!(
                    "Range: {:.0} to {:.0} HU",
                    vd.intensity_range[0], vd.intensity_range[1]
                ));
            }
        } else {
            ui.label("No volume loaded");
        }
    });

    ui.separator();

    // --- Windowing / Contrast (Detailed) ---
    ui.collapsing("🌓 Windowing", |ui| {
        if let Ok(mut windowing) = world.get::<&mut VolumeWindowing>(entities.volume_windowing) {
            ui.label("Center (HU)");
            ui.add(egui::Slider::new(&mut windowing.center, -1024.0..=3071.0).show_value(true));

            ui.label("Width (HU)");
            ui.add(egui::Slider::new(&mut windowing.width, 1.0..=4096.0).show_value(true));

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Brain").clicked() {
                    windowing.center = 40.0;
                    windowing.width = 80.0;
                }
                if ui.button("Default").clicked() {
                    windowing.center = 40.0;
                    windowing.width = 400.0;
                }
            });
        }
    });

    ui.separator();

    // --- Layer Control ---
    ui.collapsing("📚 Layers", |ui| {
        let mut layers: Vec<(
            hecs::Entity,
            String,
            bool,
            f32,
            Option<roi_runtime::VoxelRoiStats>,
        )> = Vec::new();
        for (e, (roi, settings)) in world.query::<(&Roi, &LayerSettings)>().iter() {
            layers.push((
                e,
                roi.metadata.name.clone(),
                roi.metadata.is_visible,
                settings.opacity,
                roi_runtime::roi_voxel_stats(world, e),
            ));
        }

        let active_roi = world
            .get::<&EditorState>(entities.editor)
            .ok()
            .and_then(|e| e.active_roi);
        let mut new_active_roi = active_roi;

        ui.horizontal(|ui| {
            ui.label("Create contour ROI:");
            for (label, family) in [
                ("Axial", PlaneFamily::Axial),
                ("Coronal", PlaneFamily::Coronal),
                ("Sagittal", PlaneFamily::Sagittal),
                ("Oblique", PlaneFamily::Oblique),
            ] {
                if ui.small_button(label).clicked() {
                    match roi_runtime::create_empty_contour_roi(world, entities.editor, family) {
                        Ok(entity) => {
                            new_active_roi = Some(entity);
                            handlers::set_status_message(
                                world,
                                entities,
                                format!("Created contour ROI ({family:?})"),
                            );
                            let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                        }
                        Err(err) => {
                            handlers::set_status_message(world, entities, err);
                        }
                    }
                }
            }
        });

        let active_primary = new_active_roi.and_then(|entity| {
            world
                .get::<&Roi>(entity)
                .ok()
                .map(|roi| roi.primary_representation())
        });
        if let (Some(roi_entity), Some(primary)) = (new_active_roi, active_primary) {
            let voxel_status = roi::request_voxel_overlay_state(world, roi_entity);
            let voxel_ready = primary == PrimaryRepresentation::Voxel
                || voxel_status.state == roi::RepresentationRequestState::Current;
            let mesh_status = roi::request_mesh_cache_state(world, roi_entity);
            let mesh_ready = primary == PrimaryRepresentation::Mesh
                || mesh_status.state == roi::RepresentationRequestState::Current;
            let contour_family = world
                .get::<&Roi>(roi_entity)
                .ok()
                .and_then(|roi| {
                    roi.contour_data()
                        .map(|contour| contour.active_plane_family)
                });
            let target_contour_family = active_viewport_contour_view_key(world, entities, roi_entity)
                .map(|key| key.family)
                .filter(|family| *family != PlaneFamily::Oblique)
                .or(contour_family)
                .unwrap_or(PlaneFamily::Axial);

            ui.horizontal(|ui| {
                ui.label("Authority:");
                if ui
                    .add_enabled(
                        voxel_ready,
                        egui::Button::new("Voxel")
                            .small()
                            .selected(primary == PrimaryRepresentation::Voxel),
                    )
                    .on_hover_text(if voxel_ready {
                        "Use the current voxel representation as authority."
                    } else {
                        "Voxel representation is not current yet."
                    })
                    .clicked()
                    && primary != PrimaryRepresentation::Voxel
                {
                    match roi::promote_current_voxel_cache_to_authority(world, roi_entity) {
                        Ok(()) => {
                            activate_promoted_edit_tool(
                                world,
                                entities,
                                EditorTool::Navigation,
                            );
                            handlers::set_status_message(
                                world,
                                entities,
                                "Switched ROI to voxel authority.".to_string(),
                            );
                            let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                        }
                        Err(error) => handlers::set_status_message(
                            world,
                            entities,
                            format!("Voxel promotion failed: {error:?}."),
                        ),
                    }
                }
                let voxel_rebuildable = primary == PrimaryRepresentation::Mesh
                    && matches!(
                        voxel_status.state,
                        roi::RepresentationRequestState::Stale
                            | roi::RepresentationRequestState::Blocked
                    );
                if ui
                    .add_enabled(
                        voxel_rebuildable,
                        egui::Button::new("Build voxels").small(),
                    )
                    .on_hover_text(
                        "Resample the mesh onto its reference voxel grid for the voxel overlay or voxel authority. This can be slow for large meshes.",
                    )
                    .clicked()
                {
                    match roi::request_mesh_voxel_cache_rebuild(world, roi_entity) {
                        Ok(()) => {
                            handlers::set_status_message(
                                world,
                                entities,
                                "Voxel-cache rebuild queued; mesh contours remain direct.".to_string(),
                            );
                            let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                        }
                        Err(error) => handlers::set_status_message(
                            world,
                            entities,
                            format!("Voxel-cache rebuild request failed: {error:?}."),
                        ),
                    }
                }
                if ui
                    .add_enabled(
                        voxel_ready || primary == PrimaryRepresentation::Contour,
                        egui::Button::new("Contour")
                            .small()
                            .selected(primary == PrimaryRepresentation::Contour),
                    )
                    .on_hover_text(if voxel_ready || primary == PrimaryRepresentation::Contour {
                        "Use contours from the active 2D plane family as authority."
                    } else {
                        "Voxel representation must be current before contour conversion."
                    })
                    .clicked()
                    && primary != PrimaryRepresentation::Contour
                {
                    match roi::promote_roi_to_contour_authority(
                        world,
                        roi_entity,
                        target_contour_family,
                    ) {
                        Ok(()) => {
                            focus_cursor_on_first_extracted_slice(world, entities, roi_entity);
                            activate_promoted_edit_tool(
                                world,
                                entities,
                                EditorTool::ContourSelect,
                            );
                            handlers::set_status_message(
                                world,
                                entities,
                                format!(
                                    "Switched ROI to {target_contour_family:?} contour authority."
                                ),
                            );
                            let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                        }
                        Err(error) => handlers::set_status_message(
                            world,
                            entities,
                            format!("Contour promotion failed: {error:?}."),
                        ),
                    }
                }
                let mesh_hover = if mesh_ready {
                    "Use the current mesh representation as authority.".to_string()
                } else {
                    format!("Mesh representation is {}.", mesh_status.state.as_str())
                };
                if ui
                    .add_enabled(
                        mesh_ready,
                        egui::Button::new("Mesh")
                            .small()
                            .selected(primary == PrimaryRepresentation::Mesh),
                    )
                    .on_hover_text(mesh_hover)
                    .clicked()
                    && primary != PrimaryRepresentation::Mesh
                {
                    match roi::promote_current_mesh_cache_to_authority(world, roi_entity) {
                        Ok(()) => {
                            activate_promoted_edit_tool(world, entities, EditorTool::MeshDeform);
                            handlers::set_status_message(
                                world,
                                entities,
                                "Switched ROI to mesh authority.".to_string(),
                            );
                            let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                        }
                        Err(error) => handlers::set_status_message(
                            world,
                            entities,
                            format!("Mesh promotion failed: {error:?}."),
                        ),
                    }
                }
            });
        }

        let active_mesh_roi = new_active_roi.filter(|entity| {
            world.get::<&Roi>(*entity).is_ok_and(|roi| {
                roi.primary_representation() == PrimaryRepresentation::Mesh
            })
        });
        if let Some(mesh_entity) = active_mesh_roi {
            let (has_mesh_preview, mut brush_radius_mm, mut brush_strength) = world
                .get::<&EditorState>(entities.editor)
                .map(|editor| {
                    (
                        editor
                            .mesh_edit_preview()
                            .as_ref()
                            .is_some_and(|preview| preview.roi_entity == mesh_entity),
                        editor.mesh_brush_radius_mm,
                        editor.mesh_brush_strength,
                    )
                })
                .unwrap_or((false, 12.0, 1.0));
            let radius_changed = ui
                .add(egui::Slider::new(&mut brush_radius_mm, 1.0..=50.0).text("Minimum radius mm"))
                .on_hover_text("The affected surface area grows during long drags. This is the minimum radius; disconnected surfaces remain unaffected.")
                .changed();
            let strength_changed = ui
                .add(egui::Slider::new(&mut brush_strength, 0.1..=2.0).text("Strength"))
                .changed();
            if radius_changed || strength_changed {
                if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
                    editor.mesh_brush_radius_mm = brush_radius_mm;
                    editor.mesh_brush_strength = brush_strength;
                }
            }
            ui.horizontal(|ui| {
                if has_mesh_preview {
                    if ui.small_button("Commit").clicked() {
                        if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                            input.mesh_move_pending_commit = false;
                        }
                        match roi::commit_mesh_edit_preview(world, entities.editor) {
                            Ok(()) => {
                                handlers::set_status_message(
                                    world,
                                    entities,
                                    "Committed mesh edit; build voxels explicitly when needed."
                                        .to_string(),
                                );
                                ctx.request_repaint();
                            }
                            Err(error) => handlers::set_status_message(
                                world,
                                entities,
                                format!("Mesh commit failed: {error:?}."),
                            ),
                        }
                    }
                    if ui.small_button("Cancel").clicked() {
                        if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                            input.mesh_move_pending_commit = false;
                        }
                        match roi::cancel_mesh_edit_preview(world, entities.editor) {
                            Ok(()) => {
                                handlers::set_status_message(
                                    world,
                                    entities,
                                    "Cancelled mesh edit preview.".to_string(),
                                );
                                ctx.request_repaint();
                            }
                            Err(error) => handlers::set_status_message(
                                world,
                                entities,
                                format!("Mesh preview cancel failed: {error:?}."),
                            ),
                        }
                    }
                }
            });
        }

        let mut active_contour_plane_family = new_active_roi.and_then(|entity| {
            world
                .get::<&Roi>(entity)
                .ok()
                .and_then(|roi| roi.contour_data().map(|contour| contour.active_plane_family))
        });

        if let (Some(entity), Some(current_family)) = (new_active_roi, active_contour_plane_family) {
            ui.horizontal(|ui| {
                ui.label("Contour plane family:");
                egui::ComboBox::from_id_salt("contour_plane_family")
                    .selected_text(format!("{current_family:?}"))
                    .show_ui(ui, |ui| {
                        for family in [
                            PlaneFamily::Axial,
                            PlaneFamily::Coronal,
                            PlaneFamily::Sagittal,
                            PlaneFamily::Oblique,
                        ] {
                            ui.selectable_value(
                                &mut active_contour_plane_family,
                                Some(family),
                                format!("{family:?}"),
                            );
                        }
                    });
            });

            if active_contour_plane_family != Some(current_family) {
                if let Some(new_family) = active_contour_plane_family {
                    match roi::set_active_contour_plane_family(world, entity, new_family) {
                        Ok(()) => {}
                        Err(roi::ContourPlaneFamilySwitchError::RequiresConversion) => {
                            match promote_contour_family_for_active_view(
                                world, entities, entity, new_family,
                            ) {
                                Ok(()) => {
                                    roi::clear_roi_edit_history_for_roi(
                                        world,
                                        entity,);
                                    handlers::set_status_message(
                                        world,
                                        entities,
                                        format!("Switched contour authority to {new_family:?}."),
                                    )
                                }
                                Err(error) => handlers::set_status_message(
                                    world,
                                    entities,
                                    format!("Contour family switch failed: {error}."),
                                ),
                            }
                        }
                        Err(roi::ContourPlaneFamilySwitchError::MissingRoi) => {
                            handlers::set_status_message(
                                world,
                                entities,
                                "Cannot set contour plane family: active ROI is missing."
                                    .to_string(),
                            )
                        }
                        Err(roi::ContourPlaneFamilySwitchError::NotContourRoi) => {
                            handlers::set_status_message(
                                world,
                                entities,
                                "Cannot set contour plane family: active ROI is not contour-primary."
                                    .to_string(),
                            )
                        }
                    }
                }
            }

            if let Some(view_key) = active_viewport_contour_view_key(world, entities, entity) {
                let contour_status = roi::request_contour_view_state(world, entity, &view_key);
                ui.label(format!(
                    "Displayed view: {:?} ({})",
                    view_key.family,
                    contour_status.request.state.as_str()
                ));
                if let Some(reason) = contour_status.request.reason.as_deref() {
                    ui.small(reason);
                }
                if view_key.family != current_family
                    && ui.small_button("Make displayed view editable").clicked()
                {
                    if contour_status.promotable {
                        match roi::promote_contour_view_to_authoritative(
                            world, entity, &view_key,
                        ) {
                            Ok(()) => {
                                roi::clear_roi_edit_history_for_roi(
                                    world,
                                    entity,);
                                handlers::set_status_message(
                                    world,
                                    entities,
                                    format!(
                                        "Promoted {:?} contour view to authoritative editing.",
                                        view_key.family
                                    ),
                                );
                                ctx.request_repaint();
                            }
                            Err(err) => handlers::set_status_message(
                                world,
                                entities,
                                format!("Contour view promotion failed: {err:?}."),
                            ),
                        }
                    } else {
                        let message = match contour_status.request.state {
                            roi::RepresentationRequestState::Current => {
                                "Displayed contour view is already authoritative.".to_string()
                            }
                            roi::RepresentationRequestState::Stale => {
                                "Displayed contour view is stale; rebuild the voxel cache before promotion."
                                    .to_string()
                            }
                            roi::RepresentationRequestState::Preview => {
                                "Displayed contour view is a live preview; commit and wait for the current cache before promotion."
                                    .to_string()
                            }
                            roi::RepresentationRequestState::Queued => {
                                "Displayed contour view rebuild is queued; wait for current data before promotion."
                                    .to_string()
                            }
                            roi::RepresentationRequestState::Rebuilding => {
                                "Displayed contour view is rebuilding; wait for the voxel cache rebuild to finish."
                                    .to_string()
                            }
                            roi::RepresentationRequestState::Blocked
                            | roi::RepresentationRequestState::Unsupported => format!(
                                "Displayed contour view cannot be promoted: {}.",
                                contour_status
                                    .request
                                    .reason
                                    .as_deref()
                                    .unwrap_or("no promotable derived view is available")
                            ),
                        };
                        handlers::set_status_message(world, entities, message);
                    }
                }
            }
        }

        for (entity, name, mut visible, mut opacity, stats) in layers {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut new_active_roi, Some(entity), "");
                    if ui.checkbox(&mut visible, "").changed() {
                        let allow_visibility = if visible {
                            roi_runtime::can_enable_roi_visibility(world, entity)
                        } else {
                            true
                        };

                        if allow_visibility {
                            if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
                                roi.metadata.is_visible = visible;
                            }
                            let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                        } else {
                            visible = false;
                            if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
                                roi.metadata.is_visible = false;
                            }
                            handlers::set_status_message(
                                world,
                                entities,
                                "Only two ROI overlays can be visible at once in the current renderer."
                                    .to_string(),
                            );
                        }
                    }
                    ui.label(name);
                });
                if let Some(stats) = stats {
                    ui.label(format!(
                        "{} voxels, {:.2} mm^3",
                        stats.occupied_voxels, stats.volume_mm3
                    ));
                }
                if visible
                    && ui
                        .add(egui::Slider::new(&mut opacity, 0.0..=1.0).text("Opacity"))
                        .changed()
                {
                    if let Ok(mut set) = world.get::<&mut LayerSettings>(entity) {
                        set.opacity = opacity;
                    }
                }
            });

            let show_contour_point_controls = Some(entity) == new_active_roi
                && world.get::<&Roi>(entity).is_ok_and(|roi| {
                    roi.primary_representation() == PrimaryRepresentation::Contour
                })
                && world
                    .get::<&EditorState>(entities.editor)
                    .is_ok_and(|editor| editor.active_tool == EditorTool::ContourSelect);
            if show_contour_point_controls {
                ui.horizontal(|ui| {
                if ui
                    .small_button("Insert Point")
                    .on_hover_text("Insert a point in the selected loop.")
                    .clicked()
                {
                    let mouse_uv = world
                        .get::<&InputState>(entities.input)
                        .map(|input| input.mouse_uv)
                        .unwrap_or([0.5, 0.5]);
                    match crate::systems::insert_point_into_selected_loop(world, entities, mouse_uv)
                    {
                        Ok(()) => {
                            handlers::set_status_message(
                                world,
                                entities,
                                "Inserted contour point.".to_string(),
                            );
                            ctx.request_repaint();
                        }
                        Err(err) => handlers::set_status_message(
                            world,
                            entities,
                            format!("Insert point failed: {err:?}"),
                        ),
                    }
                }

                if ui
                    .small_button("Delete Selected")
                    .on_hover_text("Delete selected point or loop.")
                    .clicked()
                {
                    match crate::systems::delete_selected_contour_element(world, entities) {
                        Ok(()) => {
                            handlers::set_status_message(
                                world,
                                entities,
                                "Deleted selected contour element.".to_string(),
                            );
                            ctx.request_repaint();
                        }
                        Err(err) => handlers::set_status_message(
                            world,
                            entities,
                            format!("Delete failed: {err:?}"),
                        ),
                    }
                }
                });
            }
        }

        if new_active_roi != active_roi {
            if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                input.contour_move_pending_commit = false;
                input.mesh_move_pending_commit = false;
            }
            crate::systems::clear_contour_draft_for_roi_change(
                world,
                entities.editor,
                new_active_roi,
            );
            crate::systems::clear_contour_selection_for_roi_change(
                world,
                entities.editor,
                new_active_roi,
            );
            if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
                editor.active_roi = new_active_roi;
                let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
            }
        }
    });

    ui.separator();

    // --- Annotations ---
    ui.collapsing("📍 Annotations", |ui| {
        if ui.button("➕ Add at Cursor").clicked() {
            let mut current_pos = glam::Vec3::ZERO;
            if let Ok(t) = world.get::<&Transform>(entities.cursor) {
                current_pos = glam::Vec3::from(t.position);
            }

            if let Ok(mut state) = world.get::<&mut AnnotationState>(entities.annotations) {
                let next_idx = state.annotations.len() + 1;
                let new_id = uuid::Uuid::new_v4();
                state.annotations.push(Annotation {
                    id: new_id,
                    world_pos: current_pos,
                    label: format!("Note {}", next_idx),
                    note: String::new(),
                    comments: vec![],
                });
                state.focused_id = Some(new_id);
                state.show_right_sidebar = true;
            }
        }

        if ui.button("📁 View All Notes").clicked() {
            if let Ok(mut state) = world.get::<&mut AnnotationState>(entities.annotations) {
                state.focused_id = None;
                state.show_right_sidebar = true;
            }
        }

        ui.separator();

        if let Ok(mut state) = world.get::<&mut AnnotationState>(entities.annotations) {
            if !state.annotations.is_empty() {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        let mut to_focus = None;
                        for ann in &state.annotations {
                            let is_focused = state.focused_id == Some(ann.id);
                            if ui
                                .selectable_label(is_focused, format!("📍 {}", ann.label))
                                .clicked()
                            {
                                to_focus = Some(ann.id);
                            }
                        }
                        if let Some(id) = to_focus {
                            state.focused_id = Some(id);
                            state.show_right_sidebar = true;
                        }
                    });
            } else {
                ui.label(
                    egui::RichText::new("No notes yet.")
                        .size(10.0)
                        .color(egui::Color32::GRAY),
                );
            }
        }
    });

    ui.separator();
    ui.collapsing("⌨ Controls", |ui| {
        ui.label("LMB: Set crosshair");
        ui.label("MMB: Pan");
        ui.label("RMB: Rotate (3D / Oblique)");
        ui.label("Scroll: Zoom / Slice");
        ui.label("Ctrl+Scroll: 2D Zoom");
    });

    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
        ui.separator();
        if let Some(hu) = crate::systems::get_hu_at_mouse(world, entities) {
            ui.label(format!("HU at cursor: {:.0}", hu));
        } else {
            ui.label("HU at cursor: --");
        }
    });
}

use crate::components::*;
use hecs::{Entity, World};

pub struct HangingProtocol {
    pub name: String,
    pub viewports: Vec<(ViewMode, [f32; 4])>, // ViewMode and relative_rect [x, y, w, h]
}

pub fn get_protocol_registry() -> Vec<HangingProtocol> {
    vec![
        HangingProtocol {
            name: "Standard 2x2".to_string(),
            viewports: vec![
                (ViewMode::ThreeD, [0.0, 0.0, 0.5, 0.5]),
                (ViewMode::Axial, [0.5, 0.0, 0.5, 0.5]),
                (ViewMode::Coronal, [0.0, 0.5, 0.5, 0.5]),
                (ViewMode::Sagittal, [0.5, 0.5, 0.5, 0.5]),
            ],
        },
        HangingProtocol {
            name: "Single Axial".to_string(),
            viewports: vec![(ViewMode::Axial, [0.0, 0.0, 1.0, 1.0])],
        },
        HangingProtocol {
            name: "Single 3D".to_string(),
            viewports: vec![(ViewMode::ThreeD, [0.0, 0.0, 1.0, 1.0])],
        },
        HangingProtocol {
            name: "Single Coronal".to_string(),
            viewports: vec![(ViewMode::Coronal, [0.0, 0.0, 1.0, 1.0])],
        },
        HangingProtocol {
            name: "Single Sagittal".to_string(),
            viewports: vec![(ViewMode::Sagittal, [0.0, 0.0, 1.0, 1.0])],
        },
        HangingProtocol {
            name: "Single Oblique".to_string(),
            viewports: vec![(ViewMode::Oblique, [0.0, 0.0, 1.0, 1.0])],
        },
        HangingProtocol {
            name: "Clinical Triple".to_string(),
            viewports: vec![
                (ViewMode::ThreeD, [0.0, 0.0, 0.5, 1.0]),
                (ViewMode::Axial, [0.5, 0.0, 0.5, 0.5]),
                (ViewMode::Coronal, [0.5, 0.5, 0.5, 0.5]),
            ],
        },
    ]
}

pub fn apply_protocol(world: &mut World, entities: &AppEntities, protocol_name: &str) {
    let registry = get_protocol_registry();
    let protocol = registry
        .iter()
        .find(|p| p.name == protocol_name)
        .unwrap_or(&registry[0]);

    // If we are maximizing (going to a Single view), remember what we had before
    if let Ok(mut proto_state) = world.get::<&mut ProtocolState>(entities.protocol) {
        // If we are maximizing (going to a Single view), remember what we had before
        if protocol_name.starts_with("Single") && !proto_state.active_protocol.starts_with("Single")
        {
            proto_state.last_protocol = Some(proto_state.active_protocol.clone());
        }
        proto_state.active_protocol = protocol_name.to_string();
    }

    // 1. Remove existing viewports
    let to_remove: Vec<Entity> = world.query::<&Viewport>().iter().map(|(e, _)| e).collect();
    for e in to_remove {
        let _ = world.despawn(e);
    }

    // 2. Spawn new viewports based on the protocol
    let mut first_vp = None;
    for (i, (mode, rect)) in protocol.viewports.iter().enumerate() {
        let e = world.spawn((
            Viewport {
                mode: *mode,
                rect: [0.0, 0.0, 0.0, 0.0], // Will be updated by GUI
                uniform_index: i as u32,
            },
            ViewportState::default(),
            ViewportLayout {
                relative_rect: *rect,
            },
        ));
        if i == 0 {
            first_vp = Some(e);
        }
    }

    // 3. Update active viewport
    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        if let Some(e) = first_vp {
            input.active_viewport = Some(e);
        }
    }
}

pub fn toggle_maximize(world: &mut World, entities: &AppEntities, vp_entity: Entity) {
    let (mode, is_already_maximized) = {
        let Ok(proto_state) = world.get::<&ProtocolState>(entities.protocol) else {
            return;
        };
        let Ok(vp) = world.get::<&Viewport>(vp_entity) else {
            return;
        };
        (vp.mode, proto_state.active_protocol.starts_with("Single"))
    };

    if is_already_maximized {
        // Restore
        let restore_name = {
            let Ok(mut proto_state) = world.get::<&mut ProtocolState>(entities.protocol) else {
                return;
            };
            proto_state
                .last_protocol
                .take()
                .unwrap_or_else(|| "Standard 2x2".to_string())
        };
        apply_protocol(world, entities, &restore_name);
    } else {
        // Maximize
        let protocol_name = match mode {
            ViewMode::Axial => "Single Axial",
            ViewMode::Coronal => "Single Coronal",
            ViewMode::Sagittal => "Single Sagittal",
            ViewMode::Oblique => "Single Oblique",
            ViewMode::ThreeD => "Single 3D", // We should probably add this to the registry
        };

        // Ensure the "Single 3D" or similar exists or we fallback to one that fills the screen
        apply_protocol(world, entities, protocol_name);
    }
}

pub fn swap_viewports(
    world: &mut World,
    _entities: &AppEntities,
    entity_a: Entity,
    entity_b: Entity,
) {
    if entity_a == entity_b {
        return;
    }

    let (mode_a, state_a) = {
        let Ok(vp) = world.get::<&Viewport>(entity_a) else {
            return;
        };
        let Ok(vs) = world.get::<&ViewportState>(entity_a) else {
            return;
        };
        (vp.mode, *vs)
    };
    let (mode_b, state_b) = {
        let Ok(vp) = world.get::<&Viewport>(entity_b) else {
            return;
        };
        let Ok(vs) = world.get::<&ViewportState>(entity_b) else {
            return;
        };
        (vp.mode, *vs)
    };

    {
        let Ok(mut vp_a) = world.get::<&mut Viewport>(entity_a) else {
            return;
        };
        let Ok(mut vs_a) = world.get::<&mut ViewportState>(entity_a) else {
            return;
        };
        vp_a.mode = mode_b;
        *vs_a = state_b;
    }
    {
        let Ok(mut vp_b) = world.get::<&mut Viewport>(entity_b) else {
            return;
        };
        let Ok(mut vs_b) = world.get::<&mut ViewportState>(entity_b) else {
            return;
        };
        vp_b.mode = mode_a;
        *vs_b = state_a;
    }
}

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

    // 3. Update active viewport and protocol name
    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        if let Some(e) = first_vp {
            input.active_viewport = Some(e);
        }
    }

    if let Ok(mut proto_state) = world.get::<&mut ProtocolState>(entities.protocol) {
        proto_state.active_protocol = protocol_name.to_string();
    }
}

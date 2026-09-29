use crate::app::components::LoadResult;
use crate::io::nifti::LoadError;

#[derive(Debug)]
// Events are rare and short-lived; the loaded volume/label payloads dominate the size.
#[allow(clippy::large_enum_variant)]
pub enum AppEvent {
    VolumeLoaded(Result<LoadResult, LoadError>),
    QaStartSampleLoad,
    RebuildBindGroups,
    SwitchProtocol(String),
    ToggleMaximize(hecs::Entity),
    SwapViewports(hecs::Entity, hecs::Entity),
    FocusAnnotation(uuid::Uuid),
    AddComment(uuid::Uuid, String),
    DeleteAnnotation(uuid::Uuid),
}

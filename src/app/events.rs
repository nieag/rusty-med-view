use crate::app::components::LoadResult;
use crate::io::nifti::LoadError;

#[derive(Debug)]
pub enum AppEvent {
    VolumeLoaded(Result<LoadResult, LoadError>),
    RebuildBindGroups,
    SwitchProtocol(String),
    ToggleMaximize(hecs::Entity),
    SwapViewports(hecs::Entity, hecs::Entity),
    FocusAnnotation(uuid::Uuid),
    AddComment(uuid::Uuid, String),
    DeleteAnnotation(uuid::Uuid),
}

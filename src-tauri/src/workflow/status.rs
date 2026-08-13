use super::*;

#[derive(Debug, Clone, Serialize)]
pub(super) struct AudioLevelSample {
    pub(super) level: f32,
    pub(super) recording_revision: u64,
}

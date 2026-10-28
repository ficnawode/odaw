use thiserror::Error;

use odaw_core::{AssetIdError, TempoError, ValidationError};

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("unsupported project format version {found} (this build supports {supported})")]
    UnsupportedVersion { found: u16, supported: u16 },
    #[error("invalid tempo map: {0}")]
    Tempo(#[from] TempoError),
    #[error("invalid asset id {value:?}: {source}")]
    InvalidAssetId {
        value: String,
        #[source]
        source: AssetIdError,
    },
    #[error("invalid project: {0}")]
    Validation(#[from] ValidationError),
}

pub const FORMAT_VERSION: u16 = 4;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProjectFile {
    pub format_version: u16,
    pub project: crate::dto::ProjectDto,
}

impl ProjectFile {
    pub fn from_project(project: &odaw_core::Project) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            project: project.into(),
        }
    }

    pub fn into_project(self) -> Result<odaw_core::Project, FormatError> {
        odaw_core::Project::try_from(self.migrate()?.project)
    }

    pub fn migrate(self) -> Result<Self, FormatError> {
        match self.format_version {
            FORMAT_VERSION => Ok(self),
            found => Err(FormatError::UnsupportedVersion {
                found,
                supported: FORMAT_VERSION,
            }),
        }
    }
}

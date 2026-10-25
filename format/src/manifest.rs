use serde::{Deserialize, Serialize};

use crate::project_file::FORMAT_VERSION;

pub const FORMAT_NAME: &str = "odaw";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const PROJECT_FILE: &str = "project.postcard";
pub const ASSETS_DIR: &str = "assets";
pub const CACHE_DIR: &str = "cache";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u16,
    pub project: String,
}

impl Manifest {
    pub fn new() -> Self {
        Self {
            format: FORMAT_NAME.to_owned(),
            format_version: FORMAT_VERSION,
            project: PROJECT_FILE.to_owned(),
        }
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_manifest_names_the_current_layout() {
        let manifest = Manifest::new();
        assert_eq!(manifest.format, "odaw");
        assert_eq!(manifest.format_version, FORMAT_VERSION);
        assert_eq!(manifest.project, "project.postcard");
    }
}

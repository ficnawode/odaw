use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const KIND_PREFIX: &str = "vst3:";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEntry {
    pub class_id: String,
    pub name: String,
    pub vendor: String,
    pub category: String,
    pub sub_categories: Vec<String>,
    pub path: PathBuf,
    pub modified_secs: u64,
    pub has_gui: bool,
    pub has_midi_input: bool,
}

impl PluginEntry {
    pub fn kind(&self) -> String {
        format!("{KIND_PREFIX}{}", self.class_id)
    }

    pub fn is_instrument(&self) -> bool {
        self.category.contains("Instrument")
            || self
                .sub_categories
                .iter()
                .any(|category| category.contains("Instrument"))
    }

    pub fn is_effect(&self) -> bool {
        !self.is_instrument()
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return true;
        }
        self.name.to_ascii_lowercase().contains(&query)
            || self.vendor.to_ascii_lowercase().contains(&query)
            || self.category.to_ascii_lowercase().contains(&query)
            || self
                .sub_categories
                .iter()
                .any(|category| category.to_ascii_lowercase().contains(&query))
            || self.class_id.to_ascii_lowercase().contains(&query)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub entries: Vec<PluginEntry>,
}

impl Catalog {
    pub fn get(&self, class_id: &str) -> Option<&PluginEntry> {
        self.entries
            .iter()
            .find(|entry| entry.class_id.eq_ignore_ascii_case(class_id))
    }

    pub fn get_kind(&self, kind: &str) -> Option<&PluginEntry> {
        kind.strip_prefix(KIND_PREFIX)
            .and_then(|class_id| self.get(class_id))
    }

    pub fn save(&self) -> Result<(), CatalogError> {
        let path = cache_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    pub fn load() -> Result<Self, CatalogError> {
        let path = cache_path()?;
        let bytes = std::fs::read(path)?;
        let catalog: Self = serde_json::from_slice(&bytes)?;
        Ok(catalog.without_stale_entries())
    }

    pub fn load_or_empty() -> Self {
        Self::load().unwrap_or_default()
    }

    /// Drop entries whose bundle vanished or changed since the cache was written.
    pub fn without_stale_entries(self) -> Self {
        Self {
            entries: self
                .entries
                .into_iter()
                .filter(|entry| entry.modified_secs == modified_secs(&entry.path))
                .collect(),
        }
    }

    /// Walk the standard VST3 locations and read bundle metadata. Bundles with a
    /// `moduleinfo.json` are inspected without loading code; older bundles fall
    /// back to the host's in-process introspection.
    pub fn scan() -> Self {
        let mut paths = vst3_host::discovery::scan_standard_paths();
        if let Some(extra) = std::env::var_os("VST3_PATH") {
            paths.extend(std::env::split_paths(&extra));
        }
        Self::scan_paths(&paths)
    }

    pub fn scan_paths(paths: &[PathBuf]) -> Self {
        let bundle_paths =
            vst3_host::discovery::scan_directories(paths).unwrap_or_else(|_| Vec::new());
        let mut seen = BTreeSet::new();
        let mut entries = Vec::new();
        for path in bundle_paths {
            let mut found = scan_bundle(&path);
            if found.is_empty() {
                if let Ok(info) = vst3_host::discovery::get_plugin_info(&path) {
                    found.push(PluginEntry {
                        class_id: info.uid,
                        name: info.name,
                        vendor: info.vendor,
                        category: info.category,
                        sub_categories: Vec::new(),
                        path: path.clone(),
                        modified_secs: modified_secs(&path),
                        has_gui: info.has_gui,
                        has_midi_input: info.has_midi_input,
                    });
                }
            }
            for entry in found {
                let key = (entry.class_id.to_ascii_uppercase(), entry.path.clone());
                if seen.insert(key) {
                    entries.push(entry);
                }
            }
        }
        entries.sort_by(|left, right| {
            left.vendor
                .to_ascii_lowercase()
                .cmp(&right.vendor.to_ascii_lowercase())
                .then_with(|| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                })
                .then_with(|| left.class_id.cmp(&right.class_id))
        });
        Self { entries }
    }
}

fn scan_bundle(path: &Path) -> Vec<PluginEntry> {
    let Ok(Some(module_info)) = vst3_host::discovery::read_module_info(path) else {
        return Vec::new();
    };
    let modified = modified_secs(path);
    let has_controller = module_info.classes.iter().any(|class| {
        class.category.contains("Component Controller") || class.category.contains("Controller")
    });
    module_info
        .classes
        .iter()
        .filter(|class| class.category.contains("Audio Module Class"))
        .map(|class| PluginEntry {
            class_id: class.class_id.clone(),
            name: class.name.clone(),
            vendor: if class.vendor.is_empty() {
                module_info.factory.vendor.clone()
            } else {
                class.vendor.clone()
            },
            category: class.category.clone(),
            sub_categories: class.sub_categories.clone(),
            path: path.to_path_buf(),
            modified_secs: modified,
            has_gui: has_controller,
            has_midi_input: class
                .sub_categories
                .iter()
                .any(|category| category.contains("Instrument")),
        })
        .collect()
}

fn modified_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs())
}

pub fn cache_path() -> Result<PathBuf, CatalogError> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .ok_or(CatalogError::NoCacheDirectory)?;
    Ok(base.join("odaw").join("vst3-plugins.json"))
}

pub fn class_id_from_kind(kind: &str) -> Option<&str> {
    kind.strip_prefix(KIND_PREFIX)
        .filter(|class_id| !class_id.is_empty())
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("no cache directory is available")]
    NoCacheDirectory,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> PluginEntry {
        PluginEntry {
            class_id: "ABCDEF019182FAEB4447534244657864".to_owned(),
            name: "Dexed".to_owned(),
            vendor: "Digital Suburban".to_owned(),
            category: "Audio Module Class".to_owned(),
            sub_categories: vec!["Instrument".to_owned(), "Synth".to_owned()],
            path: PathBuf::from("/tmp/Dexed.vst3"),
            modified_secs: 1,
            has_gui: true,
            has_midi_input: true,
        }
    }

    #[test]
    fn kind_round_trips_a_class_id() {
        let entry = entry();
        assert!(entry.is_instrument());
        assert_eq!(
            class_id_from_kind(&entry.kind()),
            Some(entry.class_id.as_str())
        );
    }

    #[test]
    fn search_matches_name_vendor_and_category() {
        let entry = entry();
        assert!(entry.matches("dexed"));
        assert!(entry.matches("digital"));
        assert!(entry.matches("synth"));
        assert!(!entry.matches("reverb"));
    }
}

//! VST3 backend for odaw.
//!
//! The core document, compiler, and engine never depend on VST3. This crate
//! implements [`odaw_engine::DeviceFactory`] and exposes a small catalogue plus
//! a live-plugin registry for state capture and editor windows.

pub mod backend;
pub mod catalog;

use std::sync::{Arc, Mutex};

use odaw_core::NodeId;
use vst3_host::Plugin;
use vst3_host::window::PluginWindow;

pub use backend::{MissingPlugin, PARAM_PREFIX, Vst3Backend, Vst3Factory, Vst3Processor};
pub use catalog::{Catalog, KIND_PREFIX, PluginEntry, class_id_from_kind};

/// Standalone plugin editor window. Editors are floating native windows; odaw
/// does not embed plugin UIs into its egui surface.
pub struct EditorWindow {
    plugin: Arc<Mutex<Plugin>>,
    window: PluginWindow,
}

impl EditorWindow {
    pub fn open(backend: &Vst3Backend, node: NodeId) -> Result<Self, String> {
        let plugin = backend
            .plugin(node)
            .ok_or_else(|| "the plugin instance is not live".to_owned())?;
        let mut window = PluginWindow::new(Arc::clone(&plugin));
        window.open().map_err(|error| error.to_string())?;
        Ok(Self { plugin, window })
    }

    /// Drive Linux IRunLoop timers, editor resize requests, and window events.
    /// Call once per UI frame. Uses non-blocking locks so a busy audio block
    /// never stalls the UI.
    pub fn service(&self) {
        if let Ok(mut plugin) = self.plugin.try_lock() {
            plugin.service_run_loop();
        }
        let _ = self.window.service_platform_events();
    }

    pub fn is_open(&self) -> bool {
        self.window.is_open() && !self.window.closed_by_user()
    }

    /// Detach the editor and destroy the native window while the X server is
    /// still available. `Drop` does this too, but the app closes windows
    /// explicitly on shutdown so plugin UI code never sees a dead window.
    pub fn close(&mut self) {
        self.window.close();
    }

    /// True while this window still points at the live instance for its node.
    /// A structural edit creates a new instance, so the old window must be
    /// closed rather than left editing a retired plugin.
    pub fn is_for(&self, plugin: &Arc<Mutex<Plugin>>) -> bool {
        Arc::ptr_eq(&self.plugin, plugin)
    }
}

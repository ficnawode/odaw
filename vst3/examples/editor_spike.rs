//! Open the first installed VST3 plugin's editor for a few seconds.
//!
//! This is the manual spike for Linux/XWayland editor support:
//! `cargo run -p odaw-vst3 --example editor_spike`.

use std::time::{Duration, Instant};

use odaw_core::{Node, NodeId};
use odaw_engine::{DeviceContext, DeviceFactory};
use odaw_vst3::{Catalog, EditorWindow, Vst3Backend};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = Catalog::scan();
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.has_gui)
        .ok_or("no VST3 plugin with an editor found")?
        .clone();
    println!("using {} ({})", entry.name, entry.path.display());
    let backend = Vst3Backend::new(catalog);
    let node = Node::device(odaw_core::NodeKind::new(entry.kind()));
    let context = DeviceContext::new(NodeId::from_raw(1), 48_000, 512, 2, 120.0);
    let _processor = backend.factory().build(&node, &context)?;
    let window = EditorWindow::open(&backend, NodeId::from_raw(1))?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) && window.is_open() {
        window.service();
        std::thread::sleep(Duration::from_millis(16));
    }
    println!(
        "editor {}",
        if window.is_open() {
            "stayed open"
        } else {
            "closed"
        }
    );
    Ok(())
}

use std::collections::HashMap;

use eframe::egui;
use odaw_core::{NodeId, ParamAddress, ParamId};

use crate::editor::Editor;

pub const GAIN_DB: &str = odaw_dsp::STRIP_GAIN_PARAM;
pub const PAN: &str = odaw_dsp::STRIP_PAN_PARAM;
pub const MIN_GAIN_DB: f64 = -60.0;
pub const MAX_GAIN_DB: f64 = 12.0;

/// Fader gestures preview straight into the realtime parameter buffer and
/// commit one document command when the drag ends. Previews are kept here so
/// the slider does not snap back to the document value mid-drag.
#[derive(Debug, Default)]
pub struct MixerState {
    previews: HashMap<ParamAddress, f64>,
}

impl MixerState {
    pub fn clear(&mut self) {
        self.previews.clear();
    }
    pub fn gain(&mut self, ui: &mut egui::Ui, editor: &mut Editor, node: NodeId, compact: bool) {
        self.fader(
            ui,
            editor,
            node,
            GAIN_DB,
            "Gain",
            MIN_GAIN_DB..=MAX_GAIN_DB,
            0.0,
            " dB",
            compact,
        );
    }

    pub fn pan(&mut self, ui: &mut egui::Ui, editor: &mut Editor, node: NodeId, compact: bool) {
        self.fader(ui, editor, node, PAN, "Pan", -1.0..=1.0, 0.0, "", compact);
    }

    #[allow(clippy::too_many_arguments)]
    fn fader(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        node: NodeId,
        param: &str,
        label: &str,
        range: std::ops::RangeInclusive<f64>,
        default: f64,
        suffix: &str,
        compact: bool,
    ) {
        let address = ParamAddress {
            node,
            param: ParamId::new(param),
        };
        let mut value = self
            .previews
            .get(&address)
            .copied()
            .or_else(|| editor.param_value(node, param))
            .unwrap_or(default);

        let response = if compact {
            let short = if param == PAN { "P" } else { "G" };
            ui.label(
                egui::RichText::new(short)
                    .size(9.0)
                    .color(egui::Color32::from_rgb(145, 151, 164)),
            );
            let width = if param == PAN { 52.0 } else { 74.0 };
            let slider = ui.add_sized(
                egui::vec2(width, 12.0),
                egui::Slider::new(&mut value, range).show_value(false),
            );
            let text = if param == PAN {
                pan_label(value)
            } else {
                format!("{value:+.1}")
            };
            ui.add_sized(
                if param == PAN {
                    egui::vec2(18.0, 12.0)
                } else {
                    egui::vec2(24.0, 12.0)
                },
                egui::Label::new(egui::RichText::new(text).size(9.0)).selectable(false),
            );
            slider
        } else if param == PAN {
            ui.add(
                egui::Slider::new(&mut value, range)
                    .text(label)
                    .custom_formatter(|value, _| pan_label(value)),
            )
        } else {
            ui.add(
                egui::Slider::new(&mut value, range)
                    .text(label)
                    .suffix(suffix)
                    .custom_formatter(|value, _| format!("{value:.1}")),
            )
        };
        let response = if compact {
            response
                .on_hover_text(label)
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        } else {
            response
        };

        if response.changed() {
            self.previews.insert(address.clone(), value);
            editor.preview_param(node, param, value);
        }
        if (response.drag_stopped() || response.lost_focus())
            && self.previews.remove(&address).is_some()
        {
            editor.commit_param(node, param, value);
        }
    }
}

fn pan_label(value: f64) -> String {
    if value.abs() < 0.005 {
        "C".to_owned()
    } else if value < 0.0 {
        format!("L{:.0}", -value * 100.0)
    } else {
        format!("R{:.0}", value * 100.0)
    }
}

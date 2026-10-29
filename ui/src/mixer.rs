use std::collections::HashMap;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, pos2, vec2};
use odaw_core::{NodeId, ParamAddress, ParamId};

use crate::editor::Editor;

pub const GAIN_DB: &str = odaw_dsp::STRIP_GAIN_PARAM;
pub const PAN: &str = odaw_dsp::STRIP_PAN_PARAM;
pub const MIN_GAIN_DB: f64 = -60.0;
pub const MAX_GAIN_DB: f64 = 12.0;

const METER_WIDTH: f32 = 32.0;
const METER_HEIGHT: f32 = 18.0;

/// Fader gestures preview straight into the realtime parameter buffer and
/// commit one document command when the drag ends. Previews are kept here so
/// the slider does not snap back to the document value mid-drag. The same
/// state owns the decaying peak meters shown next to each gain fader.
#[derive(Debug, Default)]
pub struct MixerState {
    previews: HashMap<ParamAddress, f64>,
    levels: HashMap<NodeId, (f32, f32)>,
}

impl MixerState {
    pub fn clear(&mut self) {
        self.previews.clear();
    }

    /// True while any displayed meter is still decaying and needs repaints.
    pub fn active(&self) -> bool {
        self.levels
            .values()
            .any(|(left, right)| *left > 0.0005 || *right > 0.0005)
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
        let lane_value = if editor.playing && !editor.automation_armed(&address) {
            editor
                .automation_value(&address, editor.playhead_tick())
                .map(|value| value.as_f64())
        } else {
            None
        };
        let mut value = self
            .previews
            .get(&address)
            .copied()
            .or(lane_value)
            .or_else(|| editor.param_value(node, param))
            .unwrap_or(default);

        let has_lane = editor.automation_lane(node, &ParamId::new(param)).is_some();
        let label = if has_lane {
            format!("{label}  A")
        } else {
            label.to_owned()
        };
        let is_gain = param == GAIN_DB;
        let response = if compact {
            let short = if param == PAN { "P" } else { "G" };
            ui.label(
                egui::RichText::new(if has_lane {
                    format!("{short}A")
                } else {
                    short.to_owned()
                })
                .size(9.0)
                .color(Color32::from_rgb(145, 151, 164)),
            );
            let width = if is_gain { 52.0 } else { 36.0 };
            let slider = ui.add_sized(
                vec2(width, 12.0),
                egui::Slider::new(&mut value, range).show_value(false),
            );
            let text = if param == PAN {
                pan_label(value)
            } else {
                format!("{value:+.1}")
            };
            ui.add_sized(
                if is_gain {
                    vec2(22.0, 12.0)
                } else {
                    vec2(16.0, 12.0)
                },
                egui::Label::new(egui::RichText::new(text).size(9.0)).selectable(false),
            );
            slider
        } else if is_gain {
            let slider_width = (ui.available_width() - METER_WIDTH - 8.0).max(80.0);
            ui.add_sized(
                vec2(slider_width, METER_HEIGHT),
                egui::Slider::new(&mut value, range)
                    .text(&label)
                    .suffix(suffix)
                    .custom_formatter(|value, _| format!("{value:.1}")),
            )
        } else {
            ui.add(
                egui::Slider::new(&mut value, range)
                    .text(&label)
                    .custom_formatter(|value, _| pan_label(value)),
            )
        };

        let response = if compact {
            response
                .on_hover_text(&label)
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

        let menu_address = address.clone();
        response.context_menu(|ui| {
            if ui.button("Show automation").clicked() {
                editor.request_automation(menu_address.clone());
                ui.close();
            }
            if editor
                .automation_lane(menu_address.node, &menu_address.param)
                .is_none()
            {
                if ui.button("Add automation lane").clicked() {
                    editor.add_automation_lane(&menu_address);
                    editor.request_automation(menu_address.clone());
                    ui.close();
                }
            } else if ui.button("Clear automation").clicked() {
                editor.set_automation(menu_address.node, menu_address.param.clone(), None);
                ui.close();
            }
        });
    }

    pub fn meter(&mut self, ui: &mut egui::Ui, editor: &Editor, node: NodeId) {
        let levels = self.smoothed_levels(editor, node);
        level_meter(ui, levels);
    }

    fn smoothed_levels(&mut self, editor: &Editor, node: NodeId) -> (f32, f32) {
        let measured = editor.meter_levels(node);
        let level = self.levels.entry(node).or_insert((0.0, 0.0));
        let decay = 0.86;
        level.0 = (level.0 * decay).max(measured.0);
        level.1 = (level.1 * decay).max(measured.1);
        if level.0 < 0.0005 {
            level.0 = 0.0;
        }
        if level.1 < 0.0005 {
            level.1 = 0.0;
        }
        *level
    }
}

fn level_meter(ui: &mut egui::Ui, levels: (f32, f32)) {
    let (rect, response) = ui.allocate_exact_size(vec2(METER_WIDTH, METER_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, Color32::from_rgb(17, 19, 23));
    for (index, (name, value)) in [("L", levels.0), ("R", levels.1)].into_iter().enumerate() {
        let y = rect.min.y + 2.0 + index as f32 * 7.5;
        painter.text(
            pos2(rect.min.x + 2.0, y),
            Align2::LEFT_TOP,
            name,
            FontId::monospace(7.5),
            Color32::from_rgb(112, 118, 130),
        );
        let bar = Rect::from_min_size(
            pos2(rect.min.x + 10.0, y + 0.5),
            vec2(rect.width() - 13.0, 5.0),
        );
        painter.rect_filled(bar, 1.0, Color32::from_rgb(28, 31, 37));
        let ratio = meter_ratio(value);
        if ratio > 0.0 {
            let fill = Rect::from_min_size(bar.min, vec2(bar.width() * ratio, bar.height()));
            painter.rect_filled(fill, 1.0, meter_color(value));
        }
    }
    response.on_hover_text(format!(
        "L {} · R {}",
        db_label(levels.0),
        db_label(levels.1)
    ));
}

fn meter_ratio(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * value.max(1.0e-6).log10();
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn meter_color(value: f32) -> Color32 {
    let db = 20.0 * value.max(1.0e-6).log10();
    if db > -3.0 {
        Color32::from_rgb(240, 92, 92)
    } else if db > -12.0 {
        Color32::from_rgb(226, 190, 76)
    } else {
        Color32::from_rgb(84, 196, 132)
    }
}

fn db_label(value: f32) -> String {
    if value <= 0.0005 {
        "-inf dB".to_owned()
    } else {
        format!("{:.1} dB", 20.0 * value.log10())
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

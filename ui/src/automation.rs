use std::collections::BTreeSet;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, Sense, Stroke, pos2, vec2,
};
use odaw_core::{
    AutomationLane, AutomationMode, Curve, Keyframe, NodeId, PPQ, ParamAddress, ParamValue,
    Project, Tick,
};
use odaw_engine::DeviceParameter;

use crate::editor::Editor;
use crate::piano_roll::SnapSettings;

const AXIS_WIDTH: f32 = 54.0;
const RULER_HEIGHT: f32 = 20.0;
const LANE_HEIGHT: f32 = 220.0;
const POINT_RADIUS: f32 = 4.0;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 4096.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Select,
    Draw,
}

#[derive(Clone, Debug)]
struct DragState {
    originals: Vec<Keyframe>,
    previews: Vec<Keyframe>,
    start_pointer: Pos2,
    start_value: f64,
    indices: BTreeSet<usize>,
}

#[derive(Clone, Copy, Debug)]
struct Marquee {
    start: Pos2,
    current: Pos2,
    moved: bool,
}

#[derive(Clone, Debug)]
struct Target {
    address: ParamAddress,
    group: String,
    label: String,
    has_lane: bool,
}

#[derive(Clone, Debug)]
struct PointClipboard {
    points: Vec<Keyframe>,
    anchor: Tick,
    mode: AutomationMode,
}

#[derive(Debug)]
pub struct AutomationState {
    target: Option<ParamAddress>,
    selection: BTreeSet<usize>,
    pixels_per_beat: f32,
    snap: SnapSettings,
    fit_requested: bool,
    tool: Tool,
    drag: Option<DragState>,
    marquee: Option<Marquee>,
    drawing: Option<AutomationLane>,
    context_point: Option<usize>,
    context_tick: Option<Tick>,
    context_value: Option<f64>,
    clipboard: Option<PointClipboard>,
}

impl Default for AutomationState {
    fn default() -> Self {
        Self {
            target: None,
            selection: BTreeSet::new(),
            pixels_per_beat: 90.0,
            snap: SnapSettings::at_zoom(90.0),
            fit_requested: false,
            tool: Tool::Select,
            drag: None,
            marquee: None,
            drawing: None,
            context_point: None,
            context_tick: None,
            context_value: None,
            clipboard: None,
        }
    }
}

impl AutomationState {
    pub fn clear(&mut self) {
        self.selection.clear();
        self.drag = None;
        self.marquee = None;
        self.drawing = None;
        self.context_point = None;
        self.context_tick = None;
        self.context_value = None;
    }

    pub fn has_selection(&self) -> bool {
        !self.selection.is_empty()
    }

    pub fn focus(&mut self, address: ParamAddress) {
        self.target = Some(address);
        self.selection.clear();
        self.drag = None;
        self.marquee = None;
        self.drawing = None;
    }

    pub fn copy(&mut self, editor: &Editor) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let Some(lane) = editor.automation_lane(target.node, &target.param) else {
            return;
        };
        let points: Vec<Keyframe> = if self.selection.is_empty() {
            lane.keyframes.iter().cloned().collect()
        } else {
            self.selection
                .iter()
                .filter_map(|index| lane.keyframes.get(*index).cloned())
                .collect()
        };
        let Some(anchor) = points.first().map(|point| point.tick) else {
            return;
        };
        self.clipboard = Some(PointClipboard {
            points,
            anchor,
            mode: lane.mode,
        });
    }

    pub fn paste(&mut self, editor: &mut Editor) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let Some(clipboard) = self.clipboard.clone() else {
            return;
        };
        if clipboard.points.is_empty() {
            return;
        }
        let cursor = editor.cursor_tick();
        let offset = cursor.raw() - clipboard.anchor.raw();
        let mut lane = editor
            .automation_lane(target.node, &target.param)
            .unwrap_or(AutomationLane {
                mode: clipboard.mode,
                enabled: true,
                keyframes: [].into_iter().collect(),
            });
        for point in clipboard.points {
            lane.keyframes.push_back(Keyframe {
                tick: Tick::from_raw((point.tick.raw() + offset).max(0)),
                ..point
            });
        }
        lane = normalize(lane);
        self.selection.clear();
        self.commit(editor, &target, lane);
    }

    pub fn select_all(&mut self, editor: &Editor) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let Some(lane) = editor.automation_lane(target.node, &target.param) else {
            return;
        };
        self.selection = (0..lane.keyframes.len()).collect();
    }

    pub fn delete_selected(&mut self, editor: &mut Editor) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let Some(lane) = editor.automation_lane(target.node, &target.param) else {
            return;
        };
        let mut lane = lane;
        for index in self.selection.iter().rev() {
            if *index < lane.keyframes.len() {
                lane.keyframes.remove(*index);
            }
        }
        self.selection.clear();
        self.commit(editor, &target, lane);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        editor.ensure_selection();
        let Some(track) = editor.selected_track() else {
            ui.add_space(20.0);
            ui.label("Select a track to edit automation.");
            return;
        };
        let targets = collect_targets(editor, track);
        if targets.is_empty() {
            ui.add_space(20.0);
            ui.label("This track has no automatable parameters.");
            return;
        }
        self.ensure_target(&targets);

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                vec2(218.0, ui.available_height().max(LANE_HEIGHT + 48.0)),
                Layout::top_down(Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("automation_targets")
                        .show(ui, |ui| {
                            self.sidebar(ui, editor, &targets);
                        });
                },
            );
            ui.separator();
            ui.vertical(|ui| {
                let Some(target) = self.target.clone() else {
                    return;
                };
                let Some(lane) = editor.automation_lane(target.node, &target.param) else {
                    ui.add_space(24.0);
                    ui.label("This parameter has no automation lane yet.");
                    if ui.button("Add automation lane").clicked() {
                        editor.add_automation_lane(&target);
                    }
                    return;
                };
                self.lane_editor(ui, editor, target, lane, &targets);
            });
        });
    }

    fn ensure_target(&mut self, targets: &[Target]) {
        let valid = self
            .target
            .as_ref()
            .is_some_and(|target| targets.iter().any(|entry| entry.address == *target));
        if valid {
            return;
        }
        self.target = targets
            .iter()
            .find(|target| target.has_lane)
            .or_else(|| targets.first())
            .map(|target| target.address.clone());
        self.selection.clear();
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, editor: &mut Editor, targets: &[Target]) {
        ui.strong("Parameters");
        ui.add_space(2.0);
        let mut group = String::new();
        for target in targets {
            if target.group != group {
                group = target.group.clone();
                ui.add_space(4.0);
                ui.small(&group);
            }
            ui.horizontal(|ui| {
                let selected = self.target.as_ref() == Some(&target.address);
                let lane_mark = if target.has_lane { "A" } else { " " };
                if ui
                    .selectable_label(selected, format!("{lane_mark}  {}", target.label))
                    .clicked()
                {
                    self.target = Some(target.address.clone());
                    self.selection.clear();
                }
                if !target.has_lane
                    && ui
                        .small_button("+")
                        .on_hover_text("Add automation lane")
                        .clicked()
                {
                    editor.add_automation_lane(&target.address);
                    self.target = Some(target.address.clone());
                    self.selection.clear();
                }
            });
        }
    }

    fn lane_editor(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        target: ParamAddress,
        lane: AutomationLane,
        targets: &[Target],
    ) {
        let info = editor.parameter_info(&target).cloned();
        let base = editor.base_param_value(&target);
        let cursor = editor.cursor_tick();
        let mut working = lane.clone();

        ui.horizontal_wrapped(|ui| {
            let name = targets
                .iter()
                .find(|entry| entry.address == target)
                .map_or_else(|| target.param.to_string(), |entry| entry.label.clone());
            ui.strong(name);
            ui.separator();
            let discrete = !is_continuous(base);
            ui.add_enabled_ui(!discrete, |ui| {
                let mut mode = working.mode;
                egui::ComboBox::from_id_salt("automation_mode")
                    .selected_text(match working.mode {
                        AutomationMode::Absolute => "Absolute",
                        AutomationMode::Trim => "Trim",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut mode, AutomationMode::Absolute, "Absolute");
                        ui.selectable_value(&mut mode, AutomationMode::Trim, "Trim");
                    });
                if mode != working.mode {
                    working = convert_mode(&working, mode, base);
                    self.commit(editor, &target, working.clone());
                }
            });
            if ui.checkbox(&mut working.enabled, "Enabled").changed() {
                self.commit(editor, &target, working.clone());
            }
            let mut armed = editor.automation_armed(&target);
            if ui
                .checkbox(&mut armed, "Arm")
                .on_hover_text("Write this lane while Rec and transport are running")
                .changed()
            {
                editor.set_automation_armed(target.clone(), armed);
            }
            if ui.button("Add point at cursor").clicked() {
                let value = working
                    .value_at(cursor)
                    .unwrap_or_else(|| value_from_f64(base, 0.5));
                working.keyframes.push_back(Keyframe {
                    tick: cursor,
                    value,
                    curve: Curve::Linear,
                });
                working = normalize(working.clone());
                self.commit(editor, &target, working.clone());
            }
            if ui
                .button("Clear")
                .on_hover_text("Remove every point but keep the lane")
                .clicked()
            {
                working.keyframes.clear();
                self.commit(editor, &target, working.clone());
            }
            if ui
                .button("Delete lane")
                .on_hover_text("Remove the lane and its points")
                .clicked()
            {
                editor.set_automation(target.node, target.param.clone(), None);
                self.selection.clear();
            }
            if ui
                .button("Copy")
                .on_hover_text("Copy selected points, or the whole lane")
                .clicked()
            {
                self.copy(editor);
            }
            if ui
                .add_enabled(
                    self.clipboard.is_some(),
                    egui::Button::new("Paste @ cursor"),
                )
                .on_hover_text("Paste points at the edit cursor")
                .clicked()
            {
                self.paste(editor);
            }
            ui.separator();
            ui.label("Zoom");
            ui.add(
                egui::Slider::new(&mut self.pixels_per_beat, MIN_ZOOM..=MAX_ZOOM)
                    .logarithmic(true)
                    .show_value(false),
            );
            if ui.button("Fit").on_hover_text("Fit the song").clicked() {
                self.fit_requested = true;
            }
            ui.checkbox(&mut self.snap.enabled, "Snap");
            ui.checkbox(&mut self.snap.triplets, "1/3");
            ui.small(self.snap.label());
            ui.separator();
            if ui
                .selectable_label(self.tool == Tool::Select, "Select")
                .clicked()
            {
                self.tool = Tool::Select;
            }
            if ui
                .selectable_label(self.tool == Tool::Draw, "Draw")
                .clicked()
            {
                self.tool = Tool::Draw;
            }
            if let Some(value) = working.value_at(cursor) {
                ui.separator();
                ui.monospace(format!(
                    "@ {}  {}",
                    bar_beat(cursor, beats_per_bar(editor.session.project())),
                    format_value(info.as_ref(), value, working.mode)
                ));
            }
        });

        if working.keyframes.is_empty() {
            ui.add_space(18.0);
            ui.label("No points. Tap the grid to add one.");
        }
        self.canvas(ui, editor, &target, working, info, base);
    }

    fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        target: &ParamAddress,
        lane: AutomationLane,
        info: Option<DeviceParameter>,
        base: ParamValue,
    ) {
        let project = editor.session.project().clone();
        let lane_end = lane.keyframes.last().map_or(0, |point| {
            (point.tick.raw().max(0) as f64 / PPQ as f64).ceil() as i64
        });
        let total_beats = total_beats(&project).max(lane_end + 4);
        let beats_per_bar = beats_per_bar(&project);
        let viewport_width = (ui.available_width() - AXIS_WIDTH - 8.0).max(180.0);
        if self.fit_requested {
            self.pixels_per_beat =
                (viewport_width / total_beats.max(1) as f32).clamp(MIN_ZOOM, MAX_ZOOM);
            self.snap.update_zoom(self.pixels_per_beat);
            self.fit_requested = false;
        }
        let pixels_per_beat = self.pixels_per_beat;
        self.snap.update_zoom(pixels_per_beat);
        if ui.rect_contains_pointer(ui.max_rect()) {
            let factor = zoom_factor(ui);
            if (factor - 1.0).abs() > 0.001 {
                self.pixels_per_beat = (self.pixels_per_beat * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                ui.ctx().request_repaint();
            }
        }

        let content_width = (total_beats as f32 * pixels_per_beat).max(viewport_width) + AXIS_WIDTH;
        let (range_min, range_max) = value_range(info.as_ref(), &lane, base);
        let playhead = editor.playhead_tick();
        let cursor = editor.cursor_tick();
        let previews = self
            .drag
            .as_ref()
            .map(|drag| drag.previews.clone())
            .or_else(|| {
                self.drawing
                    .as_ref()
                    .map(|lane| lane.keyframes.iter().cloned().collect())
            })
            .unwrap_or_else(|| lane.keyframes.iter().cloned().collect());
        let selection = self.selection.clone();

        egui::ScrollArea::horizontal()
            .id_salt("automation_canvas")
            .show(ui, |ui| {
                let (response, painter) =
                    ui.allocate_painter(vec2(content_width, LANE_HEIGHT), Sense::click_and_drag());
                let rect = response.rect;
                let plot = Rect::from_min_max(
                    pos2(rect.min.x + AXIS_WIDTH, rect.min.y + RULER_HEIGHT),
                    rect.max,
                );
                draw_background(
                    &painter,
                    rect,
                    plot,
                    pixels_per_beat,
                    total_beats,
                    beats_per_bar,
                    self.snap,
                    range_min,
                    range_max,
                    info.as_ref(),
                    base,
                );
                draw_curve(
                    &painter,
                    plot,
                    &previews,
                    pixels_per_beat,
                    range_min,
                    range_max,
                    selection.clone(),
                    lane.enabled,
                    info.as_ref(),
                );
                draw_playhead(&painter, plot, pixels_per_beat, playhead, cursor);
                self.interact(
                    ui,
                    editor,
                    &response,
                    plot,
                    target,
                    &lane,
                    pixels_per_beat,
                    range_min,
                    range_max,
                    base,
                );
                if let Some(marquee) = self.marquee.filter(|marquee| marquee.moved) {
                    let marquee_rect = Rect::from_two_pos(marquee.start, marquee.current);
                    painter.rect_filled(
                        marquee_rect,
                        0.0,
                        Color32::from_rgba_unmultiplied(90, 160, 255, 28),
                    );
                    painter.rect_stroke(
                        marquee_rect,
                        0.0,
                        Stroke::new(1.0_f32, Color32::from_rgb(112, 178, 255)),
                        egui::StrokeKind::Inside,
                    );
                }
                if let Some(position) = ui.input(|input| input.pointer.hover_pos()) {
                    if plot.contains(position) {
                        let tick = x_to_tick(position.x, plot, pixels_per_beat);
                        let value = y_to_value(position.y, plot, range_min, range_max);
                        painter.text(
                            pos2(plot.max.x - 6.0, plot.min.y + 4.0),
                            Align2::RIGHT_TOP,
                            format!(
                                "{}  {}",
                                bar_beat(Tick::from_raw(tick), beats_per_bar),
                                format_value(info.as_ref(), value_from_f64(base, value), lane.mode)
                            ),
                            FontId::monospace(9.0),
                            Color32::from_rgb(180, 188, 200),
                        );
                    }
                }
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn interact(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        response: &egui::Response,
        plot: Rect,
        target: &ParamAddress,
        lane: &AutomationLane,
        pixels_per_beat: f32,
        range_min: f64,
        range_max: f64,
        base: ParamValue,
    ) {
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let primary_pressed = ui.input(|input| input.pointer.primary_pressed());
        let primary_down = ui.input(|input| input.pointer.primary_down());
        let primary_released = ui.input(|input| input.pointer.primary_released());
        let secondary_pressed = ui.input(|input| input.pointer.secondary_pressed());
        let shift = ui.input(|input| input.modifiers.shift);
        let points: Vec<Keyframe> = lane.keyframes.iter().cloned().collect();

        if primary_pressed && response.hovered() {
            if let Some(position) = pointer {
                if plot.contains(position) {
                    self.context_point = None;
                    self.context_tick =
                        Some(Tick::from_raw(x_to_tick(position.x, plot, pixels_per_beat)));
                    self.context_value = Some(y_to_value(position.y, plot, range_min, range_max));
                    if self.tool == Tool::Draw {
                        let mut drawing = lane.clone();
                        add_draw_point(
                            &mut drawing,
                            position,
                            plot,
                            pixels_per_beat,
                            range_min,
                            range_max,
                            base,
                            self.snap,
                        );
                        self.drawing = Some(drawing);
                        self.drag = None;
                        self.marquee = None;
                    } else if let Some(index) = hit_point(
                        &points,
                        position,
                        plot,
                        pixels_per_beat,
                        range_min,
                        range_max,
                    ) {
                        if shift {
                            if !self.selection.remove(&index) {
                                self.selection.insert(index);
                            }
                        } else if !self.selection.contains(&index) {
                            self.selection.clear();
                            self.selection.insert(index);
                        }
                        let originals = points.clone();
                        self.drag = Some(DragState {
                            originals: originals.clone(),
                            previews: originals,
                            start_pointer: position,
                            start_value: y_to_value(position.y, plot, range_min, range_max),
                            indices: self.selection.clone(),
                        });
                        self.marquee = None;
                    } else {
                        self.drag = None;
                        self.marquee = Some(Marquee {
                            start: position,
                            current: position,
                            moved: false,
                        });
                    }
                }
            }
        }

        if primary_down {
            if let Some(position) = pointer {
                let delta_ticks = x_to_tick(position.x, plot, pixels_per_beat)
                    - x_to_tick(
                        self.drag
                            .as_ref()
                            .map_or(position.x, |drag| drag.start_pointer.x),
                        plot,
                        pixels_per_beat,
                    );
                if let Some(drag) = self.drag.clone() {
                    let snapped = if self.snap.enabled {
                        self.snap.snap(delta_ticks)
                    } else {
                        delta_ticks
                    };
                    let delta_value =
                        y_to_value(position.y, plot, range_min, range_max) - drag.start_value;
                    let mut previews = drag.originals.clone();
                    for index in &drag.indices {
                        if let Some(point) = previews.get_mut(*index) {
                            let tick = (point.tick.raw() + snapped).max(0);
                            point.tick = Tick::from_raw(tick);
                            point.value = clamp_value(
                                add_value(point.value, delta_value),
                                range_min,
                                range_max,
                            );
                        }
                    }
                    self.drag = Some(DragState { previews, ..drag });
                }
                if let Some(marquee) = self.marquee {
                    let moved = marquee.moved || marquee.start.distance(position) > 3.0;
                    self.marquee = Some(Marquee {
                        current: position,
                        moved,
                        ..marquee
                    });
                }
                if self.drawing.is_some() {
                    if let Some(drawing) = &mut self.drawing {
                        add_draw_point(
                            drawing,
                            position,
                            plot,
                            pixels_per_beat,
                            range_min,
                            range_max,
                            base,
                            self.snap,
                        );
                    }
                }
            }
        }

        if primary_released {
            if let Some(marquee) = self.marquee.take() {
                if marquee.moved {
                    let marquee_rect = Rect::from_two_pos(marquee.start, marquee.current);
                    let selected: BTreeSet<usize> = points
                        .iter()
                        .enumerate()
                        .filter(|(_, point)| {
                            point_rect(
                                (**point).clone(),
                                plot,
                                pixels_per_beat,
                                range_min,
                                range_max,
                            )
                            .intersects(marquee_rect)
                        })
                        .map(|(index, _)| index)
                        .collect();
                    if shift {
                        self.selection.extend(selected);
                    } else {
                        self.selection = selected;
                    }
                } else {
                    let tick = Tick::from_raw(self.snap.snap(x_to_tick(
                        marquee.start.x,
                        plot,
                        pixels_per_beat,
                    )));
                    let value = value_from_f64(
                        base,
                        y_to_value(marquee.start.y, plot, range_min, range_max),
                    );
                    let mut lane = lane.clone();
                    lane.keyframes.push_back(Keyframe {
                        tick: tick.max(Tick::ZERO),
                        value,
                        curve: Curve::Linear,
                    });
                    lane = normalize(lane);
                    self.selection.clear();
                    self.commit(editor, target, lane);
                }
            }
            if let Some(drag) = self.drag.take() {
                let mut lane = lane.clone();
                if drag.previews != drag.originals {
                    lane.keyframes = normalize_lane(
                        AutomationLane {
                            keyframes: drag.previews.into_iter().collect(),
                            ..lane
                        },
                        lane.mode,
                    )
                    .keyframes;
                    self.selection.clear();
                    self.commit(editor, target, lane);
                }
            }
            if let Some(drawing) = self.drawing.take() {
                self.selection.clear();
                self.commit(editor, target, normalize(drawing));
            }
        }

        if secondary_pressed && response.hovered() {
            if let Some(position) = pointer {
                if plot.contains(position) {
                    self.context_point = hit_point(
                        &points,
                        position,
                        plot,
                        pixels_per_beat,
                        range_min,
                        range_max,
                    );
                    self.context_tick =
                        Some(Tick::from_raw(x_to_tick(position.x, plot, pixels_per_beat)));
                    self.context_value = Some(y_to_value(position.y, plot, range_min, range_max));
                }
            }
        }

        let context_point = self.context_point;
        let context_tick = self.context_tick;
        let context_value = self.context_value;
        let snap = self.snap;
        response.context_menu(|ui| {
            if let Some(index) = context_point {
                if ui.button("Delete point").clicked() {
                    let mut lane = lane.clone();
                    if index < lane.keyframes.len() {
                        lane.keyframes.remove(index);
                        self.selection.clear();
                        self.commit(editor, target, lane);
                    }
                    ui.close();
                }
                ui.menu_button("Curve", |ui| {
                    for (label, curve) in [("Hold", Curve::Hold), ("Linear", Curve::Linear)] {
                        if ui.button(label).clicked() {
                            let mut lane = lane.clone();
                            if let Some(point) = lane.keyframes.get_mut(index) {
                                point.curve = curve;
                            }
                            self.commit(editor, target, lane);
                            ui.close();
                        }
                    }
                });
            } else if let (Some(tick), Some(value)) = (context_tick, context_value) {
                if ui.button("Add point here").clicked() {
                    let mut lane = lane.clone();
                    lane.keyframes.push_back(Keyframe {
                        tick: Tick::from_raw(snap.snap(tick.raw()).max(0)),
                        value: value_from_f64(base, value),
                        curve: Curve::Linear,
                    });
                    self.commit(editor, target, normalize(lane));
                    ui.close();
                }
            }
            if ui.button("Clear selection").clicked() {
                self.selection.clear();
                ui.close();
            }
            if !self.selection.is_empty() && ui.button("Delete selection").clicked() {
                let mut lane = lane.clone();
                for index in self.selection.iter().rev() {
                    if *index < lane.keyframes.len() {
                        lane.keyframes.remove(*index);
                    }
                }
                self.selection.clear();
                self.commit(editor, target, lane);
                ui.close();
            }
        });

        if primary_down && (self.drag.is_some() || self.marquee.is_some() || self.drawing.is_some())
        {
            ui.ctx().request_repaint();
        }
    }

    fn commit(&self, editor: &mut Editor, target: &ParamAddress, lane: AutomationLane) {
        editor.set_automation(target.node, target.param.clone(), Some(lane));
    }
}

fn collect_targets(editor: &Editor, track: NodeId) -> Vec<Target> {
    let project = editor.session.project();
    let mut targets = Vec::new();
    if let Some(node) = project.nodes.get(&track) {
        for (address, parameter) in editor.parameter_entries(track) {
            if parameter.read_only {
                continue;
            }
            targets.push(Target {
                address,
                group: "Track".to_owned(),
                label: parameter.label,
                has_lane: node.automation.contains_key(&parameter.id),
            });
        }
    }
    if let Some(node) = project.nodes.get(&track) {
        for device in &node.rack {
            let Some(device_node) = project.nodes.get(device) else {
                continue;
            };
            let group = device_node
                .name
                .clone()
                .unwrap_or_else(|| "Device".to_owned());
            let mut entries = editor.parameter_entries(*device);
            if entries.is_empty() {
                entries = device_node
                    .params
                    .iter()
                    .map(|(id, value)| {
                        (
                            ParamAddress {
                                node: *device,
                                param: id.clone(),
                            },
                            DeviceParameter {
                                id: id.clone(),
                                label: id.to_string(),
                                default: *value,
                                min: 0.0,
                                max: 1.0,
                                step_count: 0,
                                unit: String::new(),
                                read_only: false,
                            },
                        )
                    })
                    .collect();
            }
            for (address, parameter) in entries {
                if parameter.read_only {
                    continue;
                }
                targets.push(Target {
                    address,
                    group: group.clone(),
                    label: parameter.label,
                    has_lane: device_node.automation.contains_key(&parameter.id),
                });
            }
        }
    }
    if let Some(master) = editor.master_node() {
        if let Some(node) = project.nodes.get(&master) {
            for (address, parameter) in editor.parameter_entries(master) {
                if parameter.read_only {
                    continue;
                }
                targets.push(Target {
                    address,
                    group: "Master".to_owned(),
                    label: parameter.label,
                    has_lane: node.automation.contains_key(&parameter.id),
                });
            }
        }
    }
    targets
}

fn normalize(mut lane: AutomationLane) -> AutomationLane {
    let mode = lane.mode;
    let keyframes = normalize_lane(lane.clone(), mode).keyframes;
    lane.keyframes = keyframes;
    lane
}

fn normalize_lane(mut lane: AutomationLane, mode: AutomationMode) -> AutomationLane {
    lane.mode = mode;
    let mut points: Vec<Keyframe> = lane.keyframes.iter().cloned().collect();
    points.sort_by_key(|point| point.tick);
    let mut deduped: Vec<Keyframe> = Vec::with_capacity(points.len());
    for point in points {
        if deduped
            .last()
            .is_some_and(|previous| previous.tick == point.tick)
        {
            deduped.pop();
        }
        deduped.push(point);
    }
    lane.keyframes = deduped.into_iter().collect();
    lane
}

fn convert_mode(lane: &AutomationLane, mode: AutomationMode, base: ParamValue) -> AutomationLane {
    if lane.mode == mode || !is_continuous(base) {
        return lane.clone();
    }
    let delta = match (mode, base) {
        (AutomationMode::Trim, ParamValue::Float(value)) => ParamValue::Float(-value),
        (AutomationMode::Absolute, ParamValue::Float(value)) => ParamValue::Float(value),
        (AutomationMode::Trim, ParamValue::Int(value)) => ParamValue::Int(-value),
        (AutomationMode::Absolute, ParamValue::Int(value)) => ParamValue::Int(value),
        _ => return lane.clone(),
    };
    let mut converted = lane.clone();
    converted.mode = mode;
    converted.keyframes = lane
        .keyframes
        .iter()
        .map(|point| Keyframe {
            value: point.value.offset(delta),
            ..*point
        })
        .collect();
    converted
}

#[allow(clippy::too_many_arguments)]
fn add_draw_point(
    lane: &mut AutomationLane,
    position: Pos2,
    plot: Rect,
    pixels_per_beat: f32,
    range_min: f64,
    range_max: f64,
    base: ParamValue,
    snap: SnapSettings,
) {
    if !plot.contains(position) {
        return;
    }
    let tick = Tick::from_raw(
        snap.snap(x_to_tick(position.x, plot, pixels_per_beat))
            .max(0),
    );
    let value = clamp_value(
        value_from_f64(base, y_to_value(position.y, plot, range_min, range_max)),
        range_min,
        range_max,
    );
    if let Some(last) = lane.keyframes.back_mut() {
        if last.tick == tick {
            last.value = value;
            return;
        }
    }
    lane.keyframes.push_back(Keyframe {
        tick,
        value,
        curve: Curve::Linear,
    });
    *lane = normalize(lane.clone());
}

#[allow(clippy::too_many_arguments)]
fn draw_background(
    painter: &egui::Painter,
    rect: Rect,
    plot: Rect,
    pixels_per_beat: f32,
    total_beats: i64,
    beats_per_bar: i64,
    snap: SnapSettings,
    range_min: f64,
    range_max: f64,
    info: Option<&DeviceParameter>,
    base: ParamValue,
) {
    painter.rect_filled(rect, 3.0, Color32::from_rgb(24, 26, 30));
    painter.rect_filled(plot, 0.0, Color32::from_rgb(27, 29, 34));
    for beat in 0..=total_beats {
        let x = plot.min.x + beat as f32 * pixels_per_beat;
        if x > plot.max.x {
            break;
        }
        let is_bar = beat % beats_per_bar.max(1) == 0;
        painter.line_segment(
            [pos2(x, plot.min.y), pos2(x, plot.max.y)],
            Stroke::new(
                if is_bar { 1.0_f32 } else { 0.5_f32 },
                if is_bar {
                    Color32::from_rgb(70, 75, 86)
                } else {
                    Color32::from_rgb(42, 45, 52)
                },
            ),
        );
        if is_bar {
            painter.text(
                pos2(x + 3.0, rect.min.y + 4.0),
                Align2::LEFT_TOP,
                format!("{}", beat / beats_per_bar.max(1) + 1),
                FontId::monospace(9.0),
                Color32::from_rgb(150, 158, 172),
            );
        }
    }
    if snap.enabled {
        let subdivision = snap.ticks();
        if subdivision > 0 {
            let mut tick = subdivision;
            let total = total_beats * PPQ;
            while tick < total {
                if tick % (beats_per_bar.max(1) * PPQ) != 0 && tick % PPQ != 0 {
                    let x = plot.min.x + (tick as f32 / PPQ as f32) * pixels_per_beat;
                    if x > plot.max.x {
                        break;
                    }
                    painter.line_segment(
                        [pos2(x, plot.min.y), pos2(x, plot.max.y)],
                        Stroke::new(0.3_f32, Color32::from_rgb(38, 42, 50)),
                    );
                }
                tick += subdivision;
            }
        }
    }

    for (index, ratio) in [0.0_f64, 0.25, 0.5, 0.75, 1.0].into_iter().enumerate() {
        let value = range_min + (range_max - range_min) * ratio;
        let y = value_to_y(value, plot, range_min, range_max);
        let color = if value == 0.0 {
            Color32::from_rgb(82, 90, 104)
        } else {
            Color32::from_rgb(38, 42, 50)
        };
        painter.line_segment(
            [pos2(plot.min.x, y), pos2(plot.max.x, y)],
            Stroke::new(if value == 0.0 { 1.0_f32 } else { 0.5_f32 }, color),
        );
        if index == 0 || index == 4 {
            painter.text(
                pos2(rect.min.x + 5.0, y),
                Align2::LEFT_CENTER,
                format_value(info, value_from_f64(base, value), AutomationMode::Absolute),
                FontId::monospace(8.0),
                Color32::from_rgb(150, 158, 172),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_curve(
    painter: &egui::Painter,
    plot: Rect,
    points: &[Keyframe],
    pixels_per_beat: f32,
    range_min: f64,
    range_max: f64,
    selected: BTreeSet<usize>,
    enabled: bool,
    info: Option<&DeviceParameter>,
) {
    let alpha = if enabled { 255 } else { 110 };
    let color = Color32::from_rgba_unmultiplied(90, 170, 255, alpha);
    for pair in points.windows(2) {
        let left = point_pos(pair[0].clone(), plot, pixels_per_beat, range_min, range_max);
        let right = point_pos(pair[1].clone(), plot, pixels_per_beat, range_min, range_max);
        match pair[0].curve {
            Curve::Hold => {
                painter.line_segment([left, pos2(right.x, left.y)], Stroke::new(2.0_f32, color));
                painter.line_segment([pos2(right.x, left.y), right], Stroke::new(2.0_f32, color));
            }
            Curve::Linear => {
                painter.line_segment([left, right], Stroke::new(2.0_f32, color));
            }
        }
    }
    for (index, point) in points.iter().enumerate() {
        let position = point_pos(point.clone(), plot, pixels_per_beat, range_min, range_max);
        let fill = if selected.contains(&index) {
            Color32::from_rgb(244, 162, 48)
        } else {
            color
        };
        painter.circle_filled(position, POINT_RADIUS, fill);
        painter.circle_stroke(
            position,
            POINT_RADIUS,
            Stroke::new(1.0_f32, Color32::from_rgb(230, 236, 245)),
        );
        if selected.contains(&index) {
            painter.text(
                pos2(position.x + 6.0, position.y - 6.0),
                Align2::LEFT_BOTTOM,
                format_value(info, point.value, AutomationMode::Absolute),
                FontId::monospace(8.0),
                Color32::from_rgb(240, 225, 190),
            );
        }
    }
}

fn draw_playhead(
    painter: &egui::Painter,
    plot: Rect,
    pixels_per_beat: f32,
    playhead: Tick,
    cursor: Tick,
) {
    let playhead_x = plot.min.x + (playhead.raw() as f32 / PPQ as f32) * pixels_per_beat;
    if playhead_x >= plot.min.x && playhead_x <= plot.max.x {
        painter.line_segment(
            [pos2(playhead_x, plot.min.y), pos2(playhead_x, plot.max.y)],
            Stroke::new(1.5_f32, Color32::from_rgb(244, 92, 92)),
        );
    }
    let cursor_x = plot.min.x + (cursor.raw() as f32 / PPQ as f32) * pixels_per_beat;
    if cursor_x >= plot.min.x && cursor_x <= plot.max.x {
        painter.line_segment(
            [pos2(cursor_x, plot.min.y), pos2(cursor_x, plot.max.y)],
            Stroke::new(0.8_f32, Color32::from_rgb(112, 178, 255)),
        );
    }
}

fn hit_point(
    points: &[Keyframe],
    position: Pos2,
    plot: Rect,
    pixels_per_beat: f32,
    range_min: f64,
    range_max: f64,
) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .rev()
        .find(|(_, point)| {
            point_rect(
                (**point).clone(),
                plot,
                pixels_per_beat,
                range_min,
                range_max,
            )
            .contains(position)
        })
        .map(|(index, _)| index)
}

fn point_rect(
    point: Keyframe,
    plot: Rect,
    pixels_per_beat: f32,
    range_min: f64,
    range_max: f64,
) -> Rect {
    Rect::from_center_size(
        point_pos(point, plot, pixels_per_beat, range_min, range_max),
        vec2(POINT_RADIUS * 3.0, POINT_RADIUS * 3.0),
    )
}

fn point_pos(
    point: Keyframe,
    plot: Rect,
    pixels_per_beat: f32,
    range_min: f64,
    range_max: f64,
) -> Pos2 {
    let x = plot.min.x + point.tick.raw().max(0) as f32 / PPQ as f32 * pixels_per_beat;
    let y = value_to_y(point.value.as_f64(), plot, range_min, range_max);
    pos2(
        x.clamp(plot.min.x, plot.max.x),
        y.clamp(plot.min.y, plot.max.y),
    )
}

fn value_to_y(value: f64, plot: Rect, min: f64, max: f64) -> f32 {
    let ratio = if max <= min {
        0.0
    } else {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    };
    plot.max.y - plot.height() * ratio as f32
}

fn y_to_value(y: f32, plot: Rect, min: f64, max: f64) -> f64 {
    let ratio = ((plot.max.y - y) / plot.height().max(1.0)).clamp(0.0, 1.0);
    min + (max - min) * f64::from(ratio)
}

fn x_to_tick(x: f32, plot: Rect, pixels_per_beat: f32) -> i64 {
    ((x - plot.min.x) / pixels_per_beat.max(0.01) * PPQ as f32).round() as i64
}

fn value_range(
    info: Option<&DeviceParameter>,
    lane: &AutomationLane,
    base: ParamValue,
) -> (f64, f64) {
    if let Some(parameter) = info {
        if parameter.max > parameter.min {
            return (parameter.min, parameter.max);
        }
    }
    let mut min: f64 = 0.0;
    let mut max: f64 = 1.0;
    for point in &lane.keyframes {
        let value = point.value.as_f64();
        min = min.min(value);
        max = max.max(value);
    }
    let value = base.as_f64();
    min = min.min(value);
    max = max.max(value);
    if max <= min {
        max = min + 1.0;
    }
    (min, max)
}

fn value_from_f64(reference: ParamValue, value: f64) -> ParamValue {
    match reference {
        ParamValue::Float(_) => ParamValue::Float(value),
        ParamValue::Int(_) => ParamValue::Int(value.round() as i64),
        ParamValue::Bool(_) => ParamValue::Bool(value >= 0.5),
        ParamValue::Enum(_) => ParamValue::Enum(value.max(0.0).round() as u32),
    }
}

fn add_value(value: ParamValue, delta: f64) -> ParamValue {
    match value {
        ParamValue::Float(value) => ParamValue::Float(value + delta),
        ParamValue::Int(value) => ParamValue::Int(value + delta.round() as i64),
        _ => value,
    }
}

fn clamp_value(value: ParamValue, min: f64, max: f64) -> ParamValue {
    match value {
        ParamValue::Float(value) => ParamValue::Float(value.clamp(min, max)),
        ParamValue::Int(value) => ParamValue::Int((value as f64).clamp(min, max).round() as i64),
        _ => value,
    }
}

fn is_continuous(value: ParamValue) -> bool {
    value.is_continuous()
}

fn total_beats(project: &Project) -> i64 {
    let end = odaw_render::project_end_tick(project).unwrap_or(Tick::from_raw(16 * PPQ));
    (end.raw().max(1) as f64 / PPQ as f64).ceil() as i64
}

fn beats_per_bar(project: &Project) -> i64 {
    project
        .signatures
        .iter()
        .next()
        .map_or(4, |signature| signature.numerator.max(1) as i64)
}

fn bar_beat(tick: Tick, beats_per_bar: i64) -> String {
    let beats_per_bar = beats_per_bar.max(1);
    let bar_ticks = beats_per_bar * PPQ;
    let bar = tick.raw().div_euclid(bar_ticks) + 1;
    let beat = tick.raw().rem_euclid(bar_ticks) as f64 / PPQ as f64 + 1.0;
    format!("{bar}.{beat:.2}")
}

fn format_value(info: Option<&DeviceParameter>, value: ParamValue, mode: AutomationMode) -> String {
    if let Some(parameter) = info {
        if parameter.id.as_str() == odaw_dsp::STRIP_PAN_PARAM {
            return pan_label(value.as_f64());
        }
        let mut text = parameter.format(value.as_f64());
        if mode == AutomationMode::Trim && is_continuous(value) {
            text.push_str(" trim");
        }
        return text;
    }
    format!("{:.3}", value.as_f64())
}

fn pan_label(value: f64) -> String {
    let value = value.clamp(-1.0, 1.0);
    if value.abs() < 0.005 {
        "C".to_owned()
    } else if value < 0.0 {
        format!("L{:.0}", -value * 100.0)
    } else {
        format!("R{:.0}", value * 100.0)
    }
}

fn zoom_factor(ui: &egui::Ui) -> f32 {
    let zoom = ui.input(|input| input.zoom_delta());
    let ctrl_scroll = ui.input(|input| {
        if input.modifiers.command {
            input.raw_scroll_delta.y
        } else {
            0.0
        }
    });
    if (zoom - 1.0).abs() > 0.001 {
        zoom
    } else if ctrl_scroll.abs() > 0.0 {
        (ctrl_scroll * 0.005).exp()
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(tick: i64, value: f64) -> Keyframe {
        Keyframe {
            tick: Tick::from_raw(tick),
            value: ParamValue::Float(value),
            curve: Curve::Linear,
        }
    }

    fn lane() -> AutomationLane {
        AutomationLane {
            mode: AutomationMode::Absolute,
            enabled: true,
            keyframes: [point(0, 0.25), point(10, 0.75)].into_iter().collect(),
        }
    }

    #[test]
    fn normalization_sorts_and_keeps_the_last_point() {
        let mut lane = lane();
        lane.keyframes.push_back(point(5, 0.5));
        lane.keyframes.push_back(point(10, 0.9));
        let normalized = normalize(lane);
        assert_eq!(normalized.keyframes.len(), 3);
        assert_eq!(normalized.keyframes[1].tick, Tick::from_raw(5));
        assert_eq!(normalized.keyframes[2].value, ParamValue::Float(0.9));
    }

    #[test]
    fn absolute_to_trim_rebases_around_the_base() {
        let lane = lane();
        let converted = convert_mode(&lane, AutomationMode::Trim, ParamValue::Float(0.5));
        assert_eq!(converted.keyframes[0].value, ParamValue::Float(-0.25));
        assert_eq!(converted.keyframes[1].value, ParamValue::Float(0.25));
    }

    #[test]
    fn discrete_values_do_not_convert_to_trim() {
        let lane = AutomationLane {
            mode: AutomationMode::Absolute,
            enabled: true,
            keyframes: [Keyframe {
                tick: Tick::ZERO,
                value: ParamValue::Bool(true),
                curve: Curve::Hold,
            }]
            .into_iter()
            .collect(),
        };
        let converted = convert_mode(&lane, AutomationMode::Trim, ParamValue::Bool(false));
        assert_eq!(converted.mode, AutomationMode::Absolute);
    }

    #[test]
    fn y_mapping_round_trips() {
        let plot = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
        let y = value_to_y(0.25, plot, 0.0, 1.0);
        assert!((y_to_value(y, plot, 0.0, 1.0) - 0.25).abs() < 1e-6);
    }
}

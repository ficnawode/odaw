use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use eframe::egui::{self, Color32};
use odaw_core::{ClipContent, Node, NodeId, PPQ, ParamValue, Tick};
use odaw_vst3::{Catalog, EditorWindow};

use crate::editor::{Editor, TRACK_COLORS};
use crate::mixer::MixerState;
use crate::options;
use crate::piano_roll::{PianoRollState, note_name};
use crate::timeline::{self, TimelineState};

pub struct OdawApp {
    editor: Editor,
    piano_roll: PianoRollState,
    piano_roll_open: bool,
    track_options_open: bool,
    mixer: MixerState,
    timeline: TimelineState,
    rename: Option<RenameState>,
    tempo_edit: Option<f64>,
    time_dialog: Option<TimeDraft>,
    plugin_browser_open: bool,
    plugin_search: String,
    plugin_filter: PluginFilter,
    plugin_scan: Option<Receiver<Catalog>>,
    plugin_windows: HashMap<NodeId, EditorWindow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginFilter {
    All,
    Instruments,
    Effects,
}

struct RenameState {
    track: NodeId,
    name: String,
}

#[derive(Clone, Copy, Debug)]
struct TimeDraft {
    bar: i64,
    beat: f64,
    bpm: f64,
    numerator: u16,
    denominator: u16,
}

impl OdawApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        configure_style(&cc.egui_ctx);
        let editor = Editor::new(path);
        let needs_scan = editor.needs_plugin_scan();
        let mut app = Self {
            editor,
            piano_roll: PianoRollState::default(),
            piano_roll_open: true,
            track_options_open: false,
            mixer: MixerState::default(),
            timeline: TimelineState::default(),
            rename: None,
            tempo_edit: None,
            time_dialog: None,
            plugin_browser_open: false,
            plugin_search: String::new(),
            plugin_filter: PluginFilter::All,
            plugin_scan: None,
            plugin_windows: HashMap::new(),
        };
        if needs_scan {
            app.start_plugin_scan();
        }
        app
    }

    fn start_plugin_scan(&mut self) {
        if self.plugin_scan.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let catalog = Catalog::scan();
            let _ = catalog.save();
            let _ = sender.send(catalog);
        });
        self.plugin_scan = Some(receiver);
    }

    fn piano_roll_visible(&self) -> bool {
        self.piano_roll_open
            && !self.track_options_open
            && self
                .editor
                .selected_clip_any()
                .is_some_and(|(_, clip)| matches!(clip.content, ClipContent::Midi { .. }))
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let space = ctx.input(|input| input.key_pressed(egui::Key::Space));
        let undo = ctx.input(|input| {
            input.modifiers.command && !input.modifiers.shift && input.key_pressed(egui::Key::Z)
        });
        let redo = ctx.input(|input| {
            input.modifiers.command && input.modifiers.shift && input.key_pressed(egui::Key::Z)
        });
        let save = ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S));
        let split = ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::E));
        let time_dialog =
            ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::T));
        let copy = ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::C));
        let paste = ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::V));
        let select_all =
            ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::A));
        let duplicate =
            ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::D));
        let transpose_up = ctx.input(|input| input.key_pressed(egui::Key::ArrowUp));
        let transpose_down = ctx.input(|input| input.key_pressed(egui::Key::ArrowDown));
        let shift = ctx.input(|input| input.modifiers.shift);
        let widget_focus = ctx.memory(|memory| memory.focused().is_some());
        let home = ctx.input(|input| input.key_pressed(egui::Key::Home));
        let delete = ctx.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        });

        if space {
            self.editor.toggle_play();
        }
        if redo {
            self.editor.redo();
            self.piano_roll.clear();
            self.timeline.clear();
            self.mixer.clear();
        } else if undo {
            self.editor.undo();
            self.piano_roll.clear();
            self.timeline.clear();
            self.mixer.clear();
        }
        if save {
            self.editor.save();
        }
        if split {
            self.editor.split_selected_at_cursor();
            self.timeline.clear();
        }
        if time_dialog {
            self.open_time_dialog();
        }
        if copy {
            self.editor.copy_selected_notes();
        }
        if paste {
            self.editor.paste_notes_at_cursor();
            self.piano_roll.clear();
        }
        if duplicate {
            self.editor.duplicate_selected_notes();
            self.piano_roll.clear();
        }
        if select_all {
            if self.piano_roll_visible() {
                if let Some(clip) = self.editor.selected_clip() {
                    self.editor.select_all_notes(clip);
                }
            } else {
                self.editor.select_all_clips();
            }
        }
        if !widget_focus {
            let has_notes = !self.editor.session.selection().notes.is_empty();
            if has_notes && transpose_up {
                self.editor
                    .transpose_selected_notes(if shift { 12 } else { 1 });
            } else if has_notes && transpose_down {
                self.editor
                    .transpose_selected_notes(if shift { -12 } else { -1 });
            } else if !has_notes && transpose_up {
                self.editor.move_selected_track(-1);
                self.timeline.clear();
            } else if !has_notes && transpose_down {
                self.editor.move_selected_track(1);
                self.timeline.clear();
            }
        }
        if home {
            self.editor.set_cursor(Tick::ZERO);
        }
        if delete {
            if self.editor.session.selection().notes.is_empty() {
                self.editor.delete_selected_clips();
            } else {
                self.editor.delete_selected_notes();
            }
            self.piano_roll.clear();
            self.timeline.clear();
        }
    }

    fn open_time_dialog(&mut self) {
        let signature = self.editor.time_signature();
        let (bar, beat) = bars_beats(self.editor.cursor_tick(), signature.numerator.max(1) as i64);
        self.time_dialog = Some(TimeDraft {
            bar,
            beat,
            bpm: self.editor.tempo_bpm(),
            numerator: signature.numerator,
            denominator: signature.denominator,
        });
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        let project = self.editor.session.project().clone();
        let signature = project
            .signatures
            .iter()
            .next()
            .copied()
            .unwrap_or_default();
        let beats_per_bar = signature.numerator.max(1) as i64;
        let title = match &self.editor.path {
            Some(path) => format!("{} — {}", project.meta.name, path.display()),
            None => format!("{} (demo)", project.meta.name),
        };

        ui.horizontal(|ui| {
            ui.heading("odaw");
            ui.separator();
            let play_label = if self.editor.playing { "Stop" } else { "Play" };
            if ui.button(play_label).clicked() {
                self.editor.toggle_play();
            }
            if ui
                .button("Start")
                .on_hover_text("Move the cursor to bar 1")
                .clicked()
            {
                self.editor.set_cursor(Tick::ZERO);
            }
            let tick = self.editor.playhead_tick();
            let (bar, beat) = bars_beats(tick, beats_per_bar);
            ui.monospace(format!("{bar}.{beat:.1}"));
            ui.label(format!("{}/{}", signature.numerator, signature.denominator));
            if ui
                .button("Time")
                .on_hover_text("Edit position, tempo, and time signature")
                .clicked()
            {
                self.open_time_dialog();
            }
            ui.label(format!(
                "{:.2}s / {:.2}s",
                self.editor.position_seconds(),
                self.editor.duration_seconds()
            ));
            ui.separator();
            ui.label("Tempo");
            let mut bpm = self.tempo_edit.unwrap_or_else(|| self.editor.tempo_bpm());
            let response = ui.add(
                egui::DragValue::new(&mut bpm)
                    .range(20.0..=999.0)
                    .speed(0.1)
                    .suffix(" BPM"),
            );
            if response.changed() {
                self.tempo_edit = Some(bpm);
            }
            let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
            let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
            if escape {
                self.tempo_edit = None;
            } else if response.drag_stopped()
                || response.lost_focus()
                || (enter && self.tempo_edit.is_some())
            {
                if let Some(value) = self.tempo_edit.take() {
                    self.editor.set_tempo_bpm(value);
                    self.mixer.clear();
                    self.piano_roll.clear();
                }
            }
            ui.label(format!("{} Hz", project.meta.sample_rate));
            ui.separator();
            if self.editor.audio_ready() {
                ui.label("audio ready");
            } else {
                let response =
                    ui.colored_label(Color32::from_rgb(240, 90, 90), "audio unavailable");
                if let Some(error) = self.editor.audio_error() {
                    response.on_hover_text(error);
                }
                if ui.button("Retry audio").clicked() {
                    self.editor.start_audio();
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.small(title);
            ui.separator();
            let undo_label = self
                .editor
                .session
                .undo_label()
                .map_or_else(|| "Undo".to_owned(), |label| format!("Undo {label}"));
            if ui
                .add_enabled(
                    self.editor.session.can_undo(),
                    egui::Button::new(undo_label),
                )
                .clicked()
            {
                self.editor.undo();
                self.piano_roll.clear();
                self.timeline.clear();
            }
            let redo_label = self
                .editor
                .session
                .redo_label()
                .map_or_else(|| "Redo".to_owned(), |label| format!("Redo {label}"));
            if ui
                .add_enabled(
                    self.editor.session.can_redo(),
                    egui::Button::new(redo_label),
                )
                .clicked()
            {
                self.editor.redo();
                self.piano_roll.clear();
                self.timeline.clear();
            }
            ui.separator();
            if ui.button("Save").clicked() {
                self.editor.save();
            }
            if ui.button("+ Track").clicked() {
                self.editor.add_track();
                self.piano_roll.clear();
                self.timeline.clear();
            }
            if ui.button("Plugins").clicked() {
                self.plugin_browser_open = true;
                if self.editor.plugin_entries().is_empty() {
                    self.start_plugin_scan();
                }
            }
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let project = self.editor.session.project().clone();
        let selected = self.editor.selected_track();
        ui.heading("Inspector");
        ui.separator();

        match selected.and_then(|track| project.nodes.get(&track).map(|node| (track, node))) {
            Some((track, node)) => {
                self.track_inspector(ui, track, node);
            }
            None => {
                ui.label("No track selected");
            }
        }

        if let Some(note) = self.editor.selected_note() {
            ui.separator();
            ui.label("Selected note");
            ui.label(format!(
                "{} · velocity {} · start {:.2} beats · length {:.2} beats",
                note_name(note.pitch),
                note.velocity,
                note.start.raw() as f32 / PPQ as f32,
                note.length.raw() as f32 / PPQ as f32
            ));
            if ui.button("Delete note").clicked() {
                self.editor.delete_selected_notes();
                self.piano_roll.clear();
            }
        }
    }

    fn track_inspector(&mut self, ui: &mut egui::Ui, track: NodeId, node: &Node) {
        let name = track_name(node, track);
        ui.horizontal(|ui| {
            ui.colored_label(track_color(node, track), "■");
            ui.strong(name);
        });
        ui.horizontal(|ui| {
            if ui.button("Rename").clicked() {
                self.rename = Some(RenameState {
                    track,
                    name: track_name(node, track),
                });
            }
            if self.editor.first_midi_clip(track).is_none() && ui.button("Add MIDI clip").clicked()
            {
                self.editor.add_midi_clip();
                self.piano_roll.clear();
            }
        });

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.label("Color");
            let current = node.color.or_else(|| {
                Some(
                    TRACK_COLORS
                        [(track.raw().saturating_sub(1) % TRACK_COLORS.len() as u64) as usize],
                )
            });
            for color in TRACK_COLORS {
                let selected = current == Some(color);
                let button = egui::Button::new(if selected { "✓" } else { " " })
                    .fill(rgb(color))
                    .min_size(egui::vec2(18.0, 18.0));
                if ui.add(button).clicked() {
                    self.editor.set_track_color(track, Some(color));
                }
            }
        });

        ui.label("Mixer");
        ui.horizontal(|ui| {
            self.mixer.gain(ui, &mut self.editor, track, false);
            self.mixer.meter(ui, &self.editor, track);
        });
        self.mixer.pan(ui, &mut self.editor, track, false);

        ui.separator();
        ui.label("Devices");
        if node.rack.is_empty() {
            ui.small("No devices. Add an instrument or effect from Plugins.");
        } else {
            ui.small("Signal chain, top to bottom");
        }
        let devices: Vec<NodeId> = node.rack.iter().copied().collect();
        let device_count = devices.len();
        for (index, device) in devices.into_iter().enumerate() {
            let (kind, device_name) = self.editor.session.project().nodes.get(&device).map_or(
                ("device".to_owned(), "Device".to_owned()),
                |node| {
                    (
                        node.kind
                            .as_ref()
                            .map_or_else(|| "device".to_owned(), |kind| kind.as_str().to_owned()),
                        node.name.clone().unwrap_or_else(|| "Device".to_owned()),
                    )
                },
            );
            let is_vst3 = kind.starts_with("vst3:");
            let bypassed = is_vst3 && self.editor.plugin_bypassed(device);
            let label = if bypassed {
                format!("{}. {device_name} (bypassed)", index + 1)
            } else if is_vst3 {
                format!("{}. {device_name} (VST3)", index + 1)
            } else {
                format!("{}. {device_name}", index + 1)
            };
            egui::CollapsingHeader::new(label)
                .id_salt(("device", device.raw()))
                .show(ui, |ui| {
                    ui.small(&kind);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(index > 0, egui::Button::new("Up"))
                            .on_hover_text("Move earlier in the chain")
                            .clicked()
                        {
                            self.editor.move_device(track, device, -1);
                        }
                        if ui
                            .add_enabled(index + 1 < device_count, egui::Button::new("Down"))
                            .on_hover_text("Move later in the chain")
                            .clicked()
                        {
                            self.editor.move_device(track, device, 1);
                        }
                        if ui.button("Remove").clicked() {
                            self.editor.remove_device(track, device);
                        }
                    });
                    if is_vst3 {
                        ui.horizontal(|ui| {
                            let bypass_label = if bypassed { "Enable" } else { "Bypass" };
                            if ui
                                .button(bypass_label)
                                .on_hover_text(if bypassed {
                                    "Resume plugin processing"
                                } else {
                                    "Pass audio through without processing"
                                })
                                .clicked()
                            {
                                self.editor.set_plugin_bypass(device, !bypassed);
                            }
                            if ui.button("Open editor").clicked() {
                                match self.editor.open_plugin_editor(device) {
                                    Ok(window) => {
                                        self.plugin_windows.insert(device, window);
                                    }
                                    Err(error) => self.editor.error = Some(error),
                                }
                            }
                            if self.plugin_windows.contains_key(&device)
                                && ui.button("Close editor").clicked()
                            {
                                if let Some(mut window) = self.plugin_windows.remove(&device) {
                                    window.close();
                                }
                                self.editor.persist_plugin_state();
                            }
                            if ui
                                .button("Save state")
                                .on_hover_text("Store the current plugin patch in the project")
                                .clicked()
                            {
                                let changed = self.editor.persist_plugin_state();
                                self.editor.status = Some(if changed {
                                    "plugin state saved".to_owned()
                                } else {
                                    "plugin state is already up to date".to_owned()
                                });
                            }
                        });
                        let parameters = self.editor.plugin_parameters(device);
                        if parameters.is_empty() {
                            ui.small("parameters load once the plugin is live");
                        } else {
                            egui::ScrollArea::vertical()
                                .max_height(260.0)
                                .show(ui, |ui| {
                                    for parameter in parameters {
                                        let current = self
                                            .editor
                                            .param_value(device, parameter.id.as_str())
                                            .unwrap_or_else(|| default_value(&parameter.default));
                                        let mut value = current;
                                        ui.horizontal(|ui| {
                                            let response = ui.add(
                                                egui::Slider::new(&mut value, 0.0..=1.0)
                                                    .text(&parameter.label),
                                            );
                                            let text =
                                                if response.changed() || response.hovered() {
                                                    self.editor.plugin_parameter_text(
                                                        device,
                                                        parameter.id.as_str(),
                                                        value,
                                                    )
                                                } else {
                                                    None
                                                }
                                                .unwrap_or_else(|| parameter.format(value));
                                            ui.small(text);
                                            if response.changed() {
                                                self.editor.preview_param(
                                                    device,
                                                    parameter.id.as_str(),
                                                    value,
                                                );
                                            }
                                            if response.drag_stopped() || response.lost_focus() {
                                                self.editor.commit_param(
                                                    device,
                                                    parameter.id.as_str(),
                                                    value,
                                                );
                                            }
                                        });
                                    }
                                });
                        }
                    }
                });
        }
    }

    fn plugin_browser(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.plugin_scan {
            match receiver.try_recv() {
                Ok(catalog) => {
                    self.editor.apply_plugin_catalog(catalog);
                    self.plugin_scan = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.plugin_scan = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.plugin_browser_open {
            return;
        }
        let mut open = self.plugin_browser_open;
        egui::Window::new("Plugins")
            .open(&mut open)
            .default_width(520.0)
            .default_height(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.text_edit_singleline(&mut self.plugin_search);
                    egui::ComboBox::from_id_salt("plugin_filter")
                        .selected_text(match self.plugin_filter {
                            PluginFilter::All => "All",
                            PluginFilter::Instruments => "Instruments",
                            PluginFilter::Effects => "Effects",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.plugin_filter, PluginFilter::All, "All");
                            ui.selectable_value(
                                &mut self.plugin_filter,
                                PluginFilter::Instruments,
                                "Instruments",
                            );
                            ui.selectable_value(
                                &mut self.plugin_filter,
                                PluginFilter::Effects,
                                "Effects",
                            );
                        });
                    if self.plugin_scan.is_some() {
                        ui.spinner();
                        ui.label("scanning");
                    } else if ui.button("Rescan").clicked() {
                        self.start_plugin_scan();
                    }
                });
                ui.separator();
                let entries: Vec<odaw_vst3::PluginEntry> = self
                    .editor
                    .plugin_entries()
                    .iter()
                    .filter(|entry| match self.plugin_filter {
                        PluginFilter::All => true,
                        PluginFilter::Instruments => entry.is_instrument(),
                        PluginFilter::Effects => entry.is_effect(),
                    })
                    .filter(|entry| entry.matches(&self.plugin_search))
                    .cloned()
                    .collect();
                if entries.is_empty() {
                    ui.label("No plugins match. Use Rescan if you just installed one.");
                }
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for entry in entries {
                            ui.horizontal(|ui| {
                                ui.strong(&entry.name);
                                ui.small(&entry.vendor);
                                ui.small(if entry.is_instrument() {
                                    "instrument"
                                } else {
                                    "effect"
                                });
                                let clicked = ui
                                    .button(if entry.is_instrument() {
                                        "Add instrument"
                                    } else {
                                        "Add effect"
                                    })
                                    .clicked();
                                if clicked && self.editor.add_plugin(&entry.class_id) {
                                    self.piano_roll.clear();
                                    self.timeline.clear();
                                }
                            });
                            ui.small(entry.path.display().to_string());
                        }
                    });
            });
        self.plugin_browser_open = open;
    }

    fn service_plugin_windows(&mut self) {
        let mut closed = Vec::new();
        for (node, window) in &self.plugin_windows {
            window.service();
            if !window.is_open() || !self.editor.plugin_window_is_current(*node, window) {
                closed.push(*node);
            }
        }
        for node in closed {
            if let Some(mut window) = self.plugin_windows.remove(&node) {
                window.close();
            }
        }
    }

    fn rename_window(&mut self, ctx: &egui::Context) {
        let Some(mut rename) = self.rename.take() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Rename track")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let response = ui.text_edit_singleline(&mut rename.name);
                ui.horizontal(|ui| {
                    if ui.button("Rename").clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    apply = true;
                }
            });
        if apply {
            let name = rename.name.trim();
            let name = if name.is_empty() {
                None
            } else {
                Some(name.to_owned())
            };
            self.editor.set_track_name(rename.track, name);
            open = false;
        }
        if cancel {
            open = false;
        }
        if open {
            self.rename = Some(rename);
        }
    }

    fn time_window(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.time_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Time")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Bar");
                    ui.add(
                        egui::DragValue::new(&mut draft.bar)
                            .range(1..=1_000_000)
                            .speed(0.25),
                    );
                    let max_beat = draft.numerator.max(1) as f64;
                    ui.label("Beat");
                    ui.add(
                        egui::DragValue::new(&mut draft.beat)
                            .range(1.0..=max_beat)
                            .speed(0.25)
                            .max_decimals(2),
                    );
                    if ui.button("From cursor").clicked() {
                        let (bar, beat) =
                            bars_beats(self.editor.cursor_tick(), draft.numerator.max(1) as i64);
                        draft.bar = bar;
                        draft.beat = beat;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Tempo");
                    ui.add(
                        egui::DragValue::new(&mut draft.bpm)
                            .range(20.0..=999.0)
                            .speed(0.1)
                            .suffix(" BPM"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Time signature");
                    ui.add(egui::DragValue::new(&mut draft.numerator).range(1..=32));
                    ui.label("/");
                    egui::ComboBox::from_id_salt("time_signature_denominator")
                        .selected_text(draft.denominator.to_string())
                        .show_ui(ui, |ui| {
                            for denominator in [1u16, 2, 4, 8, 16, 32] {
                                ui.selectable_value(
                                    &mut draft.denominator,
                                    denominator,
                                    denominator.to_string(),
                                );
                            }
                        });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    apply = true;
                }
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });

        if apply {
            if (draft.bpm - self.editor.tempo_bpm()).abs() > 0.05 {
                self.editor.set_tempo_bpm(draft.bpm);
                self.mixer.clear();
                self.piano_roll.clear();
            }
            let signature = self.editor.time_signature();
            if draft.numerator != signature.numerator || draft.denominator != signature.denominator
            {
                self.editor
                    .set_time_signature(draft.numerator, draft.denominator);
                self.piano_roll.clear();
            }
            self.editor.set_cursor_bar_beat(draft.bar, draft.beat);
            open = false;
        }
        if cancel {
            open = false;
        }
        if open {
            self.time_dialog = Some(draft);
        }
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(error) = &self.editor.error {
                ui.colored_label(Color32::from_rgb(240, 90, 90), error);
            } else if let Some(status) = &self.editor.status {
                ui.label(status);
            } else {
                ui.label(
                    "Space play/stop · Ctrl+E split · Ctrl+T time · Ctrl+C/V notes · Up/Down moves tracks or transposes notes · right-click menus",
                );
            }
            if let Some(clip) = self.editor.selected_clip() {
                ui.separator();
                ui.label(format!("clip {}", clip.raw()));
            }
        });
    }
}

impl eframe::App for OdawApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::from_rgb(18, 19, 22).to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.editor.tick();
        self.shortcuts(ctx);

        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            self.transport(ui);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            self.status(ui);
        });
        let selected_any = self.editor.selected_clip_any_owned();
        let clip_is_midi = selected_any
            .as_ref()
            .is_some_and(|(_, clip)| matches!(clip.content, ClipContent::Midi { .. }));
        let clip_is_audio = selected_any
            .as_ref()
            .is_some_and(|(_, clip)| matches!(clip.content, ClipContent::Audio { .. }));
        let selected_track = self.editor.selected_track();
        let piano_visible = self.piano_roll_open && !self.track_options_open && clip_is_midi;
        let options_visible = self.piano_roll_open && !piano_visible;

        if self.piano_roll_open {
            let panel_height = (ctx.available_rect().height() * 0.5).clamp(240.0, 620.0);
            let mut rename_request = None;
            egui::TopBottomPanel::bottom("bottom_panel")
                .resizable(true)
                .default_height(panel_height)
                .min_height(180.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let title = if piano_visible {
                            "Piano roll"
                        } else if clip_is_audio && !self.track_options_open {
                            "Audio clip"
                        } else {
                            "Track options"
                        };
                        ui.strong(title);
                        if ui.button("Hide panel").clicked() {
                            self.piano_roll_open = false;
                        }
                        if clip_is_midi {
                            if self.track_options_open {
                                if ui.button("Piano roll").clicked() {
                                    self.track_options_open = false;
                                }
                            } else if ui.button("Track options").clicked() {
                                self.track_options_open = true;
                            }
                        }
                        ui.separator();
                        if piano_visible {
                            ui.small(
                                "tap to add a note · drag empty space to select · drag notes to move · right-click menu",
                            );
                        } else {
                            ui.small("track mixer, colors, clip list, and audio clip details");
                        }
                    });
                    if piano_visible {
                        self.piano_roll.show(ui, &mut self.editor);
                    } else if options_visible {
                        options::show(
                            ui,
                            &mut self.editor,
                            &mut self.mixer,
                            selected_track,
                            selected_any,
                            &mut rename_request,
                        );
                    }
                });

            if let Some(track) = rename_request {
                let name = self.editor.session.project().nodes.get(&track).map_or_else(
                    || format!("Track {}", track.raw()),
                    |node| track_name(node, track),
                );
                self.rename = Some(RenameState { track, name });
            }
        } else {
            egui::TopBottomPanel::bottom("bottom_collapsed")
                .exact_height(28.0)
                .show(ctx, |ui| {
                    if ui.button("Show panel").clicked() {
                        self.piano_roll_open = true;
                    }
                });
        }
        egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(235.0)
            .show(ctx, |ui| {
                self.inspector(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            timeline::show(ui, &mut self.editor, &mut self.mixer, &mut self.timeline);
        });

        if let Some(track) = self.timeline.rename_request.take() {
            let name = self.editor.session.project().nodes.get(&track).map_or_else(
                || format!("Track {}", track.raw()),
                |node| track_name(node, track),
            );
            self.rename = Some(RenameState { track, name });
        }

        let dropped: Vec<std::path::PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            if let Some((track, tick)) = self.timeline.hover_target.or_else(|| {
                self.editor
                    .selected_track()
                    .map(|track| (track, Tick::ZERO))
            }) {
                for path in dropped {
                    self.editor.import_audio(&path, track, tick);
                }
            } else {
                self.editor.error = Some("select a track before dropping audio".to_owned());
            }
        }

        self.rename_window(ctx);
        self.time_window(ctx);
        self.plugin_browser(ctx);
        self.service_plugin_windows();

        if self.editor.playing
            || self.mixer.active()
            || self.plugin_scan.is_some()
            || !self.plugin_windows.is_empty()
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

impl Drop for OdawApp {
    fn drop(&mut self) {
        // Detach VST3 editors and destroy their native windows while the X
        // connection is still alive. Relying on field-drop order (after the
        // audio engine and main window) makes some plugin GUIs query a dead
        // X window and terminate the process with a BadWindow error.
        for (_, mut window) in self.plugin_windows.drain() {
            window.close();
        }
    }
}

pub fn run(path: Option<PathBuf>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 600.0])
            .with_transparent(false)
            .with_title("odaw"),
        ..Default::default()
    };
    eframe::run_native(
        "odaw",
        options,
        Box::new(move |cc| Ok(Box::new(OdawApp::new(cc, path)))),
    )
}

fn track_name(node: &Node, id: NodeId) -> String {
    node.name
        .clone()
        .unwrap_or_else(|| format!("Track {}", id.raw()))
}

fn bars_beats(tick: Tick, beats_per_bar: i64) -> (i64, f64) {
    let beats_per_bar = beats_per_bar.max(1);
    let bar_ticks = beats_per_bar * PPQ;
    let bar = tick.raw().div_euclid(bar_ticks) + 1;
    let beat = tick.raw().rem_euclid(bar_ticks) as f64 / PPQ as f64 + 1.0;
    (bar, beat)
}

fn track_color(node: &Node, id: NodeId) -> Color32 {
    let value = node.color.unwrap_or_else(|| {
        TRACK_COLORS[(id.raw().saturating_sub(1) % TRACK_COLORS.len() as u64) as usize]
    });
    rgb(value)
}

fn default_value(value: &ParamValue) -> f64 {
    match value {
        ParamValue::Float(value) => *value,
        ParamValue::Int(value) => *value as f64,
        ParamValue::Bool(value) => f64::from(u8::from(*value)),
        ParamValue::Enum(value) => f64::from(*value),
    }
}

fn rgb(value: u32) -> Color32 {
    Color32::from_rgb(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(22, 24, 28);
    visuals.window_fill = Color32::from_rgb(26, 28, 33);
    visuals.faint_bg_color = Color32::from_rgb(30, 32, 38);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_beat_display_follows_the_time_signature() {
        assert_eq!(bars_beats(Tick::ZERO, 4), (1, 1.0));
        assert_eq!(bars_beats(Tick::from_raw(4 * PPQ), 4), (2, 1.0));
        assert_eq!(bars_beats(Tick::from_raw(3 * PPQ), 3), (2, 1.0));
        assert_eq!(bars_beats(Tick::from_raw(4 * PPQ + PPQ / 2), 3), (2, 2.5));
    }
}

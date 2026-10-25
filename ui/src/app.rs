use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, Color32};
use odaw_core::{ClipContent, Node, NodeId, NodeKind, PPQ, Tick};

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
}

struct RenameState {
    track: NodeId,
    name: String,
}

impl OdawApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        configure_style(&cc.egui_ctx);
        Self {
            editor: Editor::new(path),
            piano_roll: PianoRollState::default(),
            piano_roll_open: true,
            track_options_open: false,
            mixer: MixerState::default(),
            timeline: TimelineState::default(),
            rename: None,
            tempo_edit: None,
        }
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
        if !widget_focus && !self.editor.session.selection().notes.is_empty() {
            let semitones = if shift { 12 } else { 1 };
            if transpose_up {
                self.editor.transpose_selected_notes(semitones);
            } else if transpose_down {
                self.editor.transpose_selected_notes(-semitones);
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

    fn transport(&mut self, ui: &mut egui::Ui) {
        let project = self.editor.session.project().clone();
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
            let (bar, beat) = bars_beats(tick);
            ui.monospace(format!("{bar}.{beat:.1}"));
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
        for device in &node.rack {
            let kind = self
                .editor
                .session
                .project()
                .nodes
                .get(device)
                .and_then(|node| node.kind.as_ref())
                .map_or("device", NodeKind::as_str);
            ui.small(format!("• {kind}"));
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

    fn status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(error) = &self.editor.error {
                ui.colored_label(Color32::from_rgb(240, 90, 90), error);
            } else if let Some(status) = &self.editor.status {
                ui.label(status);
            } else {
                ui.label(
                    "Space play/stop · Ctrl+E split · Ctrl+C/V notes · Up/Down transpose · drag clips/notes · right-click menus",
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
            timeline::show(
                ui,
                &mut self.editor,
                &mut self.mixer,
                &mut self.timeline,
                &mut self.piano_roll.snap,
            );
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

        if self.editor.playing || self.mixer.active() {
            ctx.request_repaint_after(Duration::from_millis(16));
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

fn bars_beats(tick: Tick) -> (i64, f64) {
    let bar_ticks = 4 * PPQ;
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

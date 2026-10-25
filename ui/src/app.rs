use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, Color32};
use odaw_core::{NodeKind, NodeRole, PPQ};

use crate::editor::Editor;
use crate::piano_roll::{PianoRollState, note_name};
use crate::timeline;

pub struct OdawApp {
    editor: Editor,
    piano_roll: PianoRollState,
}

impl OdawApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        configure_style(&cc.egui_ctx);
        Self {
            editor: Editor::new(path),
            piano_roll: PianoRollState::default(),
        }
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
        let delete = ctx.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        });

        if space {
            self.editor.toggle_play();
        }
        if redo {
            self.editor.redo();
            self.piano_roll.clear();
        } else if undo {
            self.editor.undo();
            self.piano_roll.clear();
        }
        if save {
            self.editor.save();
        }
        if delete {
            self.editor.delete_selected_notes();
            self.piano_roll.clear();
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        let project = self.editor.session.project().clone();
        let bpm = project
            .tempo
            .points()
            .next()
            .map(|point| 60_000_000.0 / f64::from(point.micros_per_quarter))
            .unwrap_or(120.0);
        let title = match &self.editor.path {
            Some(path) => format!("{} — {}", project.meta.name, path.display()),
            None => format!("{} (demo)", project.meta.name),
        };

        ui.horizontal(|ui| {
            ui.heading("odaw");
            ui.separator();
            ui.strong(title);
            ui.separator();
            let play_label = if self.editor.playing { "Stop" } else { "Play" };
            if ui.button(play_label).clicked() {
                self.editor.toggle_play();
            }
            ui.label(format!(
                "{:.2}s / {:.2}s",
                self.editor.position_seconds(),
                self.editor.duration_seconds()
            ));
            ui.separator();
            ui.label(format!("{bpm:.1} BPM"));
            ui.label(format!("{} Hz", project.meta.sample_rate));
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
            }
            ui.separator();
            if ui.button("Save").clicked() {
                self.editor.save();
            }
            if ui.button("+ Track").clicked() {
                self.editor.add_track();
                self.piano_roll.clear();
            }
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
    }

    fn tracks(&mut self, ui: &mut egui::Ui) {
        let project = self.editor.session.project().clone();
        let selected = self.editor.selected_track();

        ui.heading("Tracks");
        ui.horizontal(|ui| {
            if ui.button("+ Track").clicked() {
                self.editor.add_track();
                self.piano_roll.clear();
            }
            ui.label(format!(
                "{} track{}",
                project
                    .nodes
                    .values()
                    .filter(|node| node.role == NodeRole::Track)
                    .count(),
                if project
                    .nodes
                    .values()
                    .filter(|node| node.role == NodeRole::Track)
                    .count()
                    == 1
                {
                    ""
                } else {
                    "s"
                }
            ));
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                for (id, node) in project
                    .nodes
                    .iter()
                    .filter(|(_, node)| node.role == NodeRole::Track)
                {
                    let label = format!("Track {}", id.raw());
                    if ui.selectable_label(selected == Some(*id), label).clicked() {
                        self.editor.select_track(*id);
                        self.piano_roll.clear();
                    }
                    ui.indent(id, |ui| {
                        for device in &node.rack {
                            let kind = project
                                .nodes
                                .get(device)
                                .and_then(|node| node.kind.as_ref())
                                .map_or("device", NodeKind::as_str);
                            ui.small(format!("• {kind}"));
                        }
                    });
                }
            });

        ui.separator();
        if let Some(track) = selected {
            ui.label(format!("Selected track {}", track.raw()));
            if self.editor.first_midi_clip(track).is_none() && ui.button("Add MIDI clip").clicked()
            {
                self.editor.add_midi_clip();
                self.piano_roll.clear();
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
        } else {
            ui.label("No track selected");
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
                    "Space play/stop · Ctrl+Z undo · Ctrl+Shift+Z redo · drag notes · \
                     drag right edge to resize · right-click delete",
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.editor.tick();
        self.shortcuts(ctx);

        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            self.transport(ui);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            self.status(ui);
        });
        egui::SidePanel::left("tracks")
            .resizable(true)
            .default_width(250.0)
            .show(ctx, |ui| {
                self.tracks(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            timeline::show(ui, &mut self.editor);
            ui.separator();
            self.piano_roll.show(ui, &mut self.editor);
        });

        if self.editor.playing {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

pub fn run(path: Option<PathBuf>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("odaw"),
        ..Default::default()
    };
    eframe::run_native(
        "odaw",
        options,
        Box::new(move |cc| Ok(Box::new(OdawApp::new(cc, path)))),
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

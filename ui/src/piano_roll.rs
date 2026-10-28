use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};
use odaw_core::{Clip, ClipContent, ClipId, Note, NoteId, PPQ, Tick};

use crate::editor::Editor;

const PITCH_LOW: u8 = 12; // C0
const PITCH_HIGH: u8 = 96; // C7
const ROW_HEIGHT: f32 = 14.0;
const KEY_WIDTH: f32 = 48.0;
const MIN_ZOOM: f32 = 0.01;
const MAX_ZOOM: f32 = 4096.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Snap {
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
    HundredTwentyEighth,
}

impl Snap {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Whole => "1/1",
            Self::Half => "1/2",
            Self::Quarter => "1/4",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
            Self::ThirtySecond => "1/32",
            Self::SixtyFourth => "1/64",
            Self::HundredTwentyEighth => "1/128",
        }
    }

    pub(crate) fn ticks(self) -> i64 {
        match self {
            Self::Whole => PPQ * 4,
            Self::Half => PPQ * 2,
            Self::Quarter => PPQ,
            Self::Eighth => PPQ / 2,
            Self::Sixteenth => PPQ / 4,
            Self::ThirtySecond => PPQ / 8,
            Self::SixtyFourth => PPQ / 16,
            Self::HundredTwentyEighth => PPQ / 32,
        }
    }

    /// Pick a useful snap division for the current horizontal zoom.
    pub(crate) fn for_zoom(pixels_per_beat: f32) -> Self {
        if pixels_per_beat < 40.0 {
            Self::Whole
        } else if pixels_per_beat < 80.0 {
            Self::Half
        } else if pixels_per_beat < 170.0 {
            Self::Quarter
        } else if pixels_per_beat < 340.0 {
            Self::Eighth
        } else if pixels_per_beat < 680.0 {
            Self::Sixteenth
        } else if pixels_per_beat < 1360.0 {
            Self::ThirtySecond
        } else if pixels_per_beat < 2720.0 {
            Self::SixtyFourth
        } else {
            Self::HundredTwentyEighth
        }
    }
}

/// Auto zoom division plus the two user switches. A disabled snap returns raw
/// ticks; triplets scale the straight division by 2/3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapSettings {
    pub division: Snap,
    pub triplets: bool,
    pub enabled: bool,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            division: Snap::Quarter,
            triplets: false,
            enabled: true,
        }
    }
}

impl SnapSettings {
    pub(crate) fn at_zoom(pixels_per_beat: f32) -> Self {
        Self {
            division: Snap::for_zoom(pixels_per_beat),
            ..Self::default()
        }
    }

    pub(crate) fn update_zoom(&mut self, pixels_per_beat: f32) {
        self.division = Snap::for_zoom(pixels_per_beat);
    }

    pub(crate) fn ticks(self) -> i64 {
        let ticks = self.division.ticks();
        if self.triplets {
            (ticks * 2 / 3).max(1)
        } else {
            ticks
        }
    }

    pub(crate) fn snap(self, tick: i64) -> i64 {
        if !self.enabled {
            return tick;
        }
        let step = self.ticks().max(1);
        ((tick as f64 / step as f64).round() as i64) * step
    }

    pub(crate) fn label(self) -> String {
        if self.triplets {
            format!("{}T", self.division.label())
        } else {
            self.division.label().to_owned()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    Move,
    Resize,
}

#[derive(Clone, Debug)]
struct DragState {
    originals: Vec<Note>,
    previews: Vec<Note>,
    mode: DragMode,
    start_pointer: Pos2,
}

#[derive(Clone, Copy, Debug)]
struct Marquee {
    start: Pos2,
    current: Pos2,
    moved: bool,
}

#[derive(Debug)]
pub struct PianoRollState {
    pub pixels_per_beat: f32,
    pub(crate) snap: SnapSettings,
    fit_requested: bool,
    drag: Option<DragState>,
    marquee: Option<Marquee>,
    active_clip: Option<ClipId>,
    context_note: Option<NoteId>,
    context_tick: Option<Tick>,
    context_pitch: Option<u8>,
}

impl Default for PianoRollState {
    fn default() -> Self {
        Self {
            pixels_per_beat: 90.0,
            snap: SnapSettings::at_zoom(90.0),
            fit_requested: false,
            drag: None,
            marquee: None,
            active_clip: None,
            context_note: None,
            context_tick: None,
            context_pitch: None,
        }
    }
}

impl PianoRollState {
    pub fn clear(&mut self) {
        self.drag = None;
        self.marquee = None;
        self.active_clip = None;
        self.context_note = None;
        self.context_tick = None;
        self.context_pitch = None;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        editor.ensure_selection();
        self.controls(ui, editor);
        if ui.rect_contains_pointer(ui.max_rect()) {
            let factor = zoom_factor(ui);
            if (factor - 1.0).abs() > 0.001 {
                self.pixels_per_beat = (self.pixels_per_beat * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                ui.ctx().request_repaint();
            }
        }
        self.snap.update_zoom(self.pixels_per_beat);

        let Some((clip_id, clip)) = editor
            .selected_clip_data()
            .map(|(id, clip)| (id, clip.clone()))
        else {
            ui.add_space(20.0);
            ui.label("No MIDI clip. Select a track, then add a MIDI clip.");
            return;
        };
        let ClipContent::Midi { .. } = &clip.content else {
            ui.label("The selected clip is not a MIDI clip.");
            return;
        };
        if self.active_clip != Some(clip_id) {
            self.active_clip = Some(clip_id);
            self.drag = None;
            self.marquee = None;
            self.context_note = None;
        }

        let notes: Vec<Note> = editor.notes(clip_id);
        let beats_per_bar = editor
            .session
            .project()
            .signatures
            .iter()
            .next()
            .map_or(4, |signature| signature.numerator.max(1) as i64);
        let selected: BTreeSet<NoteId> = editor.session.selection().notes.clone();
        let previews: BTreeMap<NoteId, Note> = self
            .drag
            .as_ref()
            .map(|drag| drag.previews.iter().map(|note| (note.id, *note)).collect())
            .unwrap_or_default();
        let height = f32::from(PITCH_HIGH - PITCH_LOW + 1) * ROW_HEIGHT;
        let viewport_width = (ui.available_width() - KEY_WIDTH - 12.0).max(240.0);
        if self.fit_requested {
            let beats = clip.length.raw().max(1) as f32 / PPQ as f32;
            self.pixels_per_beat = (viewport_width / beats).clamp(MIN_ZOOM, MAX_ZOOM);
            self.snap.update_zoom(self.pixels_per_beat);
            self.fit_requested = false;
        }
        let px_per_tick = self.pixels_per_beat / PPQ as f32;
        let grid_width = (clip.length.raw().max(1) as f32 * px_per_tick).max(viewport_width);

        egui::ScrollArea::vertical()
            .id_salt("piano_roll_vertical")
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let (key_response, key_painter) =
                        ui.allocate_painter(vec2(KEY_WIDTH, height), Sense::hover());
                    draw_keyboard(&key_painter, key_response.rect.min, height);

                    egui::ScrollArea::horizontal()
                        .id_salt("piano_roll_grid")
                        .show(ui, |ui| {
                            let (response, painter) = ui.allocate_painter(
                                vec2(grid_width, height),
                                Sense::click_and_drag(),
                            );
                            let origin = response.rect.min;
                            draw_grid(
                                &painter,
                                origin,
                                grid_width,
                                height,
                                clip.length,
                                self.pixels_per_beat,
                                self.snap,
                                beats_per_bar,
                            );
                            let hovered = ui
                                .input(|input| input.pointer.hover_pos())
                                .and_then(|position| {
                                    hit_note(&notes, position, origin, px_per_tick)
                                })
                                .map(|note| note.id);
                            draw_notes(
                                &painter,
                                origin,
                                grid_width,
                                &notes,
                                &selected,
                                &previews,
                                hovered,
                                px_per_tick,
                            );
                            self.interact(
                                ui,
                                editor,
                                &response,
                                clip_id,
                                &clip,
                                &notes,
                                origin,
                                px_per_tick,
                            );
                            if let Some(marquee) = self.marquee.filter(|marquee| marquee.moved) {
                                let rect = Rect::from_two_pos(marquee.start, marquee.current);
                                painter.rect_filled(
                                    rect,
                                    0.0,
                                    Color32::from_rgba_unmultiplied(90, 160, 255, 28),
                                );
                                painter.rect_stroke(
                                    rect,
                                    0.0,
                                    Stroke::new(1.0_f32, Color32::from_rgb(112, 178, 255)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        });
                });
            });
    }

    fn controls(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Zoom");
            ui.add(
                egui::Slider::new(&mut self.pixels_per_beat, MIN_ZOOM..=MAX_ZOOM)
                    .logarithmic(true)
                    .show_value(false),
            );
            if ui
                .button("Fit")
                .on_hover_text("Fit the whole clip")
                .clicked()
            {
                self.fit_requested = true;
            }
            ui.separator();
            ui.checkbox(&mut self.snap.enabled, "Snap")
                .on_hover_text("Turn grid snapping on or off");
            ui.checkbox(&mut self.snap.triplets, "1/3")
                .on_hover_text("Use triplet divisions");
            ui.small(self.snap.label());
            ui.separator();

            let selected = editor.session.selection().notes.len();
            let has_clipboard = editor.note_clipboard_len().is_some();
            if ui
                .add_enabled(selected > 0, egui::Button::new("Copy"))
                .on_hover_text("Copy selected notes (Ctrl+C)")
                .clicked()
            {
                editor.copy_selected_notes();
            }
            if ui
                .add_enabled(has_clipboard, egui::Button::new("Paste @ cursor"))
                .on_hover_text("Paste at the red timeline cursor (Ctrl+V)")
                .clicked()
            {
                editor.paste_notes_at_cursor();
                self.drag = None;
                self.marquee = None;
            }
            if ui
                .add_enabled(selected > 0, egui::Button::new("Duplicate"))
                .on_hover_text("Duplicate the selected notes (Ctrl+D)")
                .clicked()
            {
                editor.duplicate_selected_notes();
                self.drag = None;
            }
            ui.menu_button("Transpose", |ui| {
                for (label, semitones) in [("-12", -12), ("-1", -1), ("+1", 1), ("+12", 12)] {
                    if ui
                        .add_enabled(selected > 0, egui::Button::new(label))
                        .clicked()
                    {
                        editor.transpose_selected_notes(semitones);
                        ui.close();
                    }
                }
            });
            ui.separator();
            if selected == 0 {
                ui.small(
                    "Tap a grid cell to add a note · drag empty space to select a pattern · drag notes to move",
                );
            } else {
                ui.small(format!(
                    "{selected} note(s) selected · drag to move · Up/Down transpose"
                ));
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn interact(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        response: &egui::Response,
        clip_id: ClipId,
        clip: &Clip,
        notes: &[Note],
        origin: Pos2,
        px_per_tick: f32,
    ) {
        let grid = Rect::from_min_size(
            pos2(origin.x, origin.y),
            vec2(
                clip.length.raw().max(1) as f32 * px_per_tick,
                f32::from(PITCH_HIGH - PITCH_LOW + 1) * ROW_HEIGHT,
            ),
        );
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let primary_pressed = ui.input(|input| input.pointer.primary_pressed());
        let primary_down = ui.input(|input| input.pointer.primary_down());
        let primary_released = ui.input(|input| input.pointer.primary_released());
        let secondary_pressed = ui.input(|input| input.pointer.secondary_pressed());
        let shift = ui.input(|input| input.modifiers.shift);

        if primary_pressed && response.hovered() {
            if let Some(position) = pointer {
                if grid.contains(position) {
                    if let Some(note) = hit_note(notes, position, origin, px_per_tick) {
                        if shift {
                            editor.select_note(note.id, true);
                        } else if !editor.session.selection().notes.contains(&note.id) {
                            editor.select_note(note.id, false);
                        }
                        let still_selected = editor.session.selection().notes.contains(&note.id);
                        if still_selected {
                            let selected = editor.selected_notes_in(clip_id);
                            let originals = if selected.is_empty() {
                                vec![note]
                            } else {
                                selected
                            };
                            let mode = if originals.len() == 1
                                && position.x > note_rect(note, origin, px_per_tick).max.x - 6.0
                            {
                                DragMode::Resize
                            } else {
                                DragMode::Move
                            };
                            self.marquee = None;
                            self.drag = Some(DragState {
                                previews: originals.clone(),
                                originals,
                                mode,
                                start_pointer: position,
                            });
                        } else {
                            self.drag = None;
                        }
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
                if let Some(drag) = self.drag.clone() {
                    let delta_ticks =
                        ((position.x - drag.start_pointer.x) / px_per_tick).round() as i64;
                    let delta_pitch =
                        -((position.y - drag.start_pointer.y) / ROW_HEIGHT).round() as i32;
                    let previews = match drag.mode {
                        DragMode::Move => moved_notes(
                            &drag.originals,
                            delta_ticks,
                            delta_pitch,
                            clip.length,
                            self.snap,
                        ),
                        DragMode::Resize => drag
                            .originals
                            .first()
                            .map(|note| {
                                vec![resized_note(*note, delta_ticks, clip.length, self.snap)]
                            })
                            .unwrap_or_default(),
                    };
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
            }
        }

        if primary_released {
            if let Some(marquee) = self.marquee.take() {
                if marquee.moved {
                    let rect = Rect::from_two_pos(marquee.start, marquee.current);
                    let ids: BTreeSet<NoteId> = notes
                        .iter()
                        .filter(|note| rect.intersects(note_rect(**note, origin, px_per_tick)))
                        .map(|note| note.id)
                        .collect();
                    editor.select_notes(ids, shift);
                } else {
                    let tick = x_to_tick(marquee.start.x - origin.x, px_per_tick);
                    let max_start = (clip.length.raw() - 1).max(0);
                    let start = self.snap.snap(tick).clamp(0, max_start);
                    let pitch = y_to_pitch(marquee.start.y - origin.y);
                    let length = self
                        .snap
                        .ticks()
                        .min((clip.length.raw() - start).max(1))
                        .max(1);
                    editor.add_note(
                        clip_id,
                        pitch,
                        Tick::from_raw(start),
                        Tick::from_raw(length),
                        100,
                    );
                }
            }
            if let Some(drag) = self.drag.take() {
                let changed: Vec<Note> = drag
                    .previews
                    .into_iter()
                    .filter(|preview| {
                        drag.originals
                            .iter()
                            .find(|original| original.id == preview.id)
                            .is_some_and(|original| original != preview)
                    })
                    .collect();
                if !changed.is_empty() {
                    editor.replace_notes(clip_id, changed);
                }
            }
        }

        if secondary_pressed && response.hovered() {
            if let Some(position) = pointer {
                if let Some(note) = hit_note(notes, position, origin, px_per_tick) {
                    if !editor.session.selection().notes.contains(&note.id) {
                        editor.select_note(note.id, false);
                    }
                    self.context_note = Some(note.id);
                    self.context_tick = Some(note.start);
                    self.context_pitch = Some(note.pitch);
                } else if grid.contains(position) {
                    self.context_note = None;
                    self.context_tick = Some(Tick::from_raw(x_to_tick(
                        position.x - origin.x,
                        px_per_tick,
                    )));
                    self.context_pitch = Some(y_to_pitch(position.y - origin.y));
                } else {
                    self.context_note = None;
                    self.context_tick = None;
                    self.context_pitch = None;
                }
            }
        }

        let context_note = self.context_note;
        let context_tick = self.context_tick;
        let context_pitch = self.context_pitch;
        let snap = self.snap;
        let selection = editor.session.selection().notes.clone();
        let group =
            context_note.is_some_and(|note| selection.contains(&note)) && selection.len() > 1;
        response.context_menu(|ui| {
            if let Some(note_id) = context_note {
                if group {
                    if ui.button("Copy selection").clicked() {
                        editor.copy_selected_notes();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            editor.note_clipboard_len().is_some(),
                            egui::Button::new("Paste at cursor"),
                        )
                        .clicked()
                    {
                        editor.paste_notes_at_cursor();
                        ui.close();
                    }
                    if ui.button("Duplicate selection").clicked() {
                        editor.duplicate_selected_notes();
                        ui.close();
                    }
                    ui.menu_button("Transpose selection", |ui| {
                        for (label, semitones) in [("-12", -12), ("-1", -1), ("+1", 1), ("+12", 12)]
                        {
                            if ui.button(label).clicked() {
                                editor.transpose_selected_notes(semitones);
                                ui.close();
                            }
                        }
                    });
                    if ui.button("Delete selection").clicked() {
                        editor.delete_selected_notes();
                        ui.close();
                    }
                } else {
                    if ui.button("Duplicate note").clicked() {
                        editor.duplicate_note(clip_id, note_id);
                        ui.close();
                    }
                    if ui.button("Copy note").clicked() {
                        editor.copy_selected_notes();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            editor.note_clipboard_len().is_some(),
                            egui::Button::new("Paste at cursor"),
                        )
                        .clicked()
                    {
                        editor.paste_notes_at_cursor();
                        ui.close();
                    }
                    if ui.button("Delete note").clicked() {
                        editor.remove_note(clip_id, note_id);
                        ui.close();
                    }
                    ui.menu_button("Velocity", |ui| {
                        for velocity in [32, 64, 96, 127] {
                            if ui.button(velocity.to_string()).clicked() {
                                editor.set_note_velocity(clip_id, note_id, velocity);
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Length", |ui| {
                        for (label, ticks) in [
                            ("1/16", PPQ / 4),
                            ("1/8", PPQ / 2),
                            ("1/4", PPQ),
                            ("1/2", PPQ * 2),
                            ("1 bar", PPQ * 4),
                        ] {
                            if ui.button(label).clicked() {
                                editor.set_note_length(clip_id, note_id, Tick::from_raw(ticks));
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Transpose", |ui| {
                        if ui.button("+12").clicked() {
                            editor.transpose_note(clip_id, note_id, 12);
                            ui.close();
                        }
                        if ui.button("-12").clicked() {
                            editor.transpose_note(clip_id, note_id, -12);
                            ui.close();
                        }
                    });
                }
            } else {
                if let (Some(tick), Some(pitch)) = (context_tick, context_pitch) {
                    if ui.button("Add note here").clicked() {
                        let start = snap
                            .snap(tick.raw())
                            .clamp(0, (clip.length.raw() - 1).max(0));
                        let length = snap.ticks().min((clip.length.raw() - start).max(1)).max(1);
                        editor.add_note(
                            clip_id,
                            pitch,
                            Tick::from_raw(start),
                            Tick::from_raw(length),
                            100,
                        );
                        ui.close();
                    }
                    ui.separator();
                }
                if ui
                    .add_enabled(
                        editor.note_clipboard_len().is_some(),
                        egui::Button::new("Paste at cursor"),
                    )
                    .clicked()
                {
                    editor.paste_notes_at_cursor();
                    ui.close();
                }
                if ui.button("Select all notes").clicked() {
                    editor.select_all_notes(clip_id);
                    ui.close();
                }
                if ui.button("Clear selection").clicked() {
                    editor.clear_selection();
                    ui.close();
                }
            }
        });

        if primary_down && (self.drag.is_some() || self.marquee.is_some()) {
            ui.ctx().request_repaint();
        }
    }
}

fn draw_keyboard(painter: &egui::Painter, origin: Pos2, height: f32) {
    let background = Color32::from_rgb(24, 26, 30);
    painter.rect_filled(
        Rect::from_min_size(origin, vec2(KEY_WIDTH, height)),
        0.0,
        background,
    );

    for pitch in PITCH_LOW..=PITCH_HIGH {
        let y = pitch_to_y(pitch);
        let rect = Rect::from_min_size(pos2(origin.x, origin.y + y), vec2(KEY_WIDTH, ROW_HEIGHT));
        let fill = if is_black_key(pitch) {
            Color32::from_rgb(38, 40, 46)
        } else {
            Color32::from_rgb(196, 198, 202)
        };
        painter.rect_filled(rect, 1.0, fill);
        painter.line_segment(
            [pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)],
            Stroke::new(0.5_f32, Color32::from_rgb(28, 30, 34)),
        );
        let color = if is_black_key(pitch) {
            Color32::from_rgb(180, 184, 192)
        } else {
            Color32::from_rgb(36, 38, 42)
        };
        painter.text(
            pos2(rect.min.x + 5.0, rect.center().y),
            Align2::LEFT_CENTER,
            note_name(pitch),
            FontId::monospace(9.0),
            color,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_grid(
    painter: &egui::Painter,
    origin: Pos2,
    width: f32,
    height: f32,
    clip_length: Tick,
    pixels_per_beat: f32,
    snap: SnapSettings,
    beats_per_bar: i64,
) {
    painter.rect_filled(
        Rect::from_min_size(origin, vec2(width, height)),
        0.0,
        Color32::from_rgb(27, 29, 34),
    );

    for pitch in PITCH_LOW..=PITCH_HIGH {
        if is_black_key(pitch) {
            let y = pitch_to_y(pitch);
            painter.rect_filled(
                Rect::from_min_size(pos2(origin.x, origin.y + y), vec2(width, ROW_HEIGHT)),
                0.0,
                Color32::from_rgb(23, 25, 30),
            );
        }
        let y = origin.y + pitch_to_y(pitch) + ROW_HEIGHT;
        painter.line_segment(
            [pos2(origin.x, y), pos2(origin.x + width, y)],
            Stroke::new(0.5_f32, Color32::from_rgb(39, 42, 48)),
        );
    }

    let beats = (clip_length.raw().max(0) as f64 / PPQ as f64).ceil() as i64;
    for beat in 0..=beats {
        let x = origin.x + beat as f32 * pixels_per_beat;
        if x > origin.x + width {
            break;
        }
        let is_bar = beat % beats_per_bar.max(1) == 0;
        painter.line_segment(
            [pos2(x, origin.y), pos2(x, origin.y + height)],
            Stroke::new(
                if is_bar { 1.0_f32 } else { 0.5_f32 },
                if is_bar {
                    Color32::from_rgb(70, 75, 86)
                } else {
                    Color32::from_rgb(42, 45, 52)
                },
            ),
        );
    }

    let subdivision = snap.ticks();
    if snap.enabled {
        let total = clip_length.raw().max(0);
        let bar_ticks = beats_per_bar.max(1) * PPQ;
        let mut tick = subdivision;
        while tick < total {
            if tick % PPQ != 0 && tick % bar_ticks != 0 {
                let x = origin.x + tick as f32 / PPQ as f32 * pixels_per_beat;
                if x > origin.x + width {
                    break;
                }
                painter.line_segment(
                    [pos2(x, origin.y), pos2(x, origin.y + height)],
                    Stroke::new(0.3_f32, Color32::from_rgb(38, 42, 50)),
                );
            }
            tick += subdivision;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_notes(
    painter: &egui::Painter,
    origin: Pos2,
    width: f32,
    notes: &[Note],
    selected: &BTreeSet<NoteId>,
    previews: &BTreeMap<NoteId, Note>,
    hovered: Option<NoteId>,
    px_per_tick: f32,
) {
    let grid = Rect::from_min_size(
        origin,
        vec2(width, f32::from(PITCH_HIGH - PITCH_LOW + 1) * ROW_HEIGHT),
    );
    for note in notes {
        let note = previews.get(&note.id).copied().unwrap_or(*note);
        if note.pitch < PITCH_LOW || note.pitch > PITCH_HIGH {
            continue;
        }
        let rect = note_rect(note, origin, px_per_tick);
        if !grid.intersects(rect) {
            continue;
        }
        let is_selected = selected.contains(&note.id);
        let color = if is_selected {
            Color32::from_rgb(244, 162, 48)
        } else {
            Color32::from_rgb(64, 132, 214)
        };
        let previewing = previews.contains_key(&note.id);
        draw_note(
            painter,
            rect,
            note.pitch,
            color,
            previewing || Some(note.id) == hovered,
        );
    }
}

fn draw_note(painter: &egui::Painter, rect: Rect, pitch: u8, color: Color32, hovered: bool) {
    painter.rect_filled(rect, 3.0, color);
    painter.line_segment(
        [rect.left_top(), rect.left_bottom()],
        Stroke::new(1.0_f32, Color32::from_rgb(230, 236, 245)),
    );
    if hovered {
        painter.line_segment(
            [rect.right_top(), rect.right_bottom()],
            Stroke::new(1.5_f32, Color32::from_rgb(255, 255, 255)),
        );
    }
    if rect.width() > 30.0 {
        painter.text(
            pos2(rect.min.x + 4.0, rect.center().y),
            Align2::LEFT_CENTER,
            note_name(pitch),
            FontId::monospace(8.0),
            Color32::from_rgb(20, 22, 26),
        );
    }
}

fn note_rect(note: Note, origin: Pos2, px_per_tick: f32) -> Rect {
    let x = origin.x + note.start.raw().max(0) as f32 * px_per_tick;
    let y = origin.y + pitch_to_y(note.pitch) + 1.0;
    let width = (note.length.raw().max(1) as f32 * px_per_tick).max(3.0);
    Rect::from_min_size(pos2(x, y), vec2(width, ROW_HEIGHT - 2.0))
}

fn hit_note(notes: &[Note], position: Pos2, origin: Pos2, px_per_tick: f32) -> Option<Note> {
    notes
        .iter()
        .rev()
        .copied()
        .find(|note| note_rect(*note, origin, px_per_tick).contains(position))
}

fn moved_notes(
    notes: &[Note],
    delta_ticks: i64,
    delta_pitch: i32,
    clip_length: Tick,
    snap: SnapSettings,
) -> Vec<Note> {
    let Some(min_start) = notes.iter().map(|note| note.start.raw()).min() else {
        return Vec::new();
    };
    let max_end = notes
        .iter()
        .map(|note| note.start.raw().saturating_add(note.length.raw()))
        .max()
        .unwrap_or(min_start);
    let min_pitch = notes
        .iter()
        .map(|note| i32::from(note.pitch))
        .min()
        .unwrap_or(0);
    let max_pitch = notes
        .iter()
        .map(|note| i32::from(note.pitch))
        .max()
        .unwrap_or(0);

    let lower_ticks = -min_start;
    let upper_ticks = (clip_length.raw() - max_end).max(lower_ticks);
    let delta_ticks = snap.snap(delta_ticks).clamp(lower_ticks, upper_ticks);
    let delta_pitch = delta_pitch.clamp(-min_pitch, 127 - max_pitch);

    notes
        .iter()
        .map(|note| Note {
            start: Tick::from_raw(note.start.raw() + delta_ticks),
            pitch: (i32::from(note.pitch) + delta_pitch).clamp(0, 127) as u8,
            ..*note
        })
        .collect()
}

fn resized_note(note: Note, delta_ticks: i64, clip_length: Tick, snap: SnapSettings) -> Note {
    let length = snap
        .snap(note.length.raw() + delta_ticks)
        .max(snap.ticks())
        .min((clip_length.raw() - note.start.raw()).max(1));
    Note {
        length: Tick::from_raw(length),
        ..note
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

fn x_to_tick(x: f32, px_per_tick: f32) -> i64 {
    (x / px_per_tick).round() as i64
}

fn pitch_to_y(pitch: u8) -> f32 {
    (i32::from(PITCH_HIGH) - i32::from(pitch)) as f32 * ROW_HEIGHT
}

fn y_to_pitch(y: f32) -> u8 {
    let row = (y / ROW_HEIGHT).floor() as i32;
    (i32::from(PITCH_HIGH) - row).clamp(0, 127) as u8
}

fn is_black_key(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

pub fn note_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(pitch % 12)],
        i32::from(pitch) / 12 - 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> Note {
        Note {
            id: NoteId::from_raw(1),
            pitch: 60,
            start: Tick::from_raw(PPQ),
            length: Tick::from_raw(PPQ),
            velocity: 100,
            channel: 0,
        }
    }

    fn settings(division: Snap) -> SnapSettings {
        SnapSettings {
            division,
            triplets: false,
            enabled: true,
        }
    }

    #[test]
    fn zoom_chooses_a_sensible_snap() {
        assert_eq!(Snap::for_zoom(30.0), Snap::Whole);
        assert_eq!(Snap::for_zoom(60.0), Snap::Half);
        assert_eq!(Snap::for_zoom(100.0), Snap::Quarter);
        assert_eq!(Snap::for_zoom(220.0), Snap::Eighth);
        assert_eq!(Snap::for_zoom(400.0), Snap::Sixteenth);
        assert_eq!(Snap::for_zoom(1000.0), Snap::ThirtySecond);
        assert_eq!(Snap::for_zoom(2000.0), Snap::SixtyFourth);
        assert_eq!(Snap::for_zoom(3000.0), Snap::HundredTwentyEighth);
    }

    #[test]
    fn snapping_rounds_to_the_nearest_step() {
        assert_eq!(settings(Snap::Quarter).snap(0), 0);
        assert_eq!(settings(Snap::Quarter).snap(PPQ / 2), PPQ);
        assert_eq!(settings(Snap::Quarter).snap(PPQ / 4), 0);
        assert_eq!(settings(Snap::Eighth).snap(PPQ), PPQ);
    }

    #[test]
    fn snap_can_be_disabled_and_triplets_are_thirds() {
        let mut spec = settings(Snap::Quarter);
        spec.enabled = false;
        assert_eq!(spec.snap(PPQ / 3), PPQ / 3);
        spec.enabled = true;
        spec.triplets = true;
        assert_eq!(spec.ticks(), PPQ * 2 / 3);
        assert_eq!(spec.snap(PPQ / 3), PPQ * 2 / 3);
        assert_eq!(spec.snap(PPQ * 2 / 3), PPQ * 2 / 3);
    }

    #[test]
    fn moving_a_group_keeps_its_pattern_and_clamps_to_the_clip() {
        let first = note();
        let mut second = note();
        second.id = NoteId::from_raw(2);
        second.start = Tick::from_raw(2 * PPQ);
        second.pitch = 64;
        let group = [first, second];

        let moved = moved_notes(
            &group,
            PPQ,
            2,
            Tick::from_raw(4 * PPQ),
            settings(Snap::Quarter),
        );
        assert_eq!(moved[0].start, Tick::from_raw(2 * PPQ));
        assert_eq!(moved[1].start, Tick::from_raw(3 * PPQ));
        assert_eq!(moved[0].pitch, 62);
        assert_eq!(moved[1].pitch, 66);

        let moved = moved_notes(
            &group,
            -PPQ * 8,
            2,
            Tick::from_raw(4 * PPQ),
            settings(Snap::Quarter),
        );
        assert_eq!(moved[0].start, Tick::ZERO);
        assert_eq!(moved[1].start, Tick::from_raw(PPQ));

        let moved = moved_notes(
            &group,
            PPQ * 8,
            -100,
            Tick::from_raw(4 * PPQ),
            settings(Snap::Quarter),
        );
        assert_eq!(moved[0].start, Tick::from_raw(2 * PPQ));
        assert_eq!(moved[1].start, Tick::from_raw(3 * PPQ));
        assert_eq!(moved[0].pitch, 0);
        assert_eq!(moved[1].pitch, 4);
    }

    #[test]
    fn resizing_keeps_a_positive_length() {
        let resized = resized_note(
            note(),
            -PPQ * 4,
            Tick::from_raw(PPQ * 4),
            settings(Snap::Quarter),
        );
        assert_eq!(resized.length, Tick::from_raw(PPQ));
        let resized = resized_note(
            note(),
            PPQ * 2,
            Tick::from_raw(PPQ * 4),
            settings(Snap::Quarter),
        );
        assert_eq!(resized.length, Tick::from_raw(PPQ * 3));
    }

    #[test]
    fn pitch_mapping_round_trips_visible_rows() {
        for pitch in PITCH_LOW..=PITCH_HIGH {
            let y = pitch_to_y(pitch) + 1.0;
            assert_eq!(y_to_pitch(y), pitch);
        }
    }

    #[test]
    fn note_names_use_midi_octaves() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(48), "C3");
    }
}

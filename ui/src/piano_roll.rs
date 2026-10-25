use std::collections::BTreeSet;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};
use odaw_core::{Clip, ClipContent, ClipId, Note, NoteId, PPQ, Tick};

use crate::editor::Editor;

const PITCH_LOW: u8 = 48;
const PITCH_HIGH: u8 = 84;
const ROW_HEIGHT: f32 = 14.0;
const KEY_WIDTH: f32 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
}

impl Snap {
    pub(crate) const ALL: [Self; 5] = [
        Self::Whole,
        Self::Half,
        Self::Quarter,
        Self::Eighth,
        Self::Sixteenth,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Whole => "1/1",
            Self::Half => "1/2",
            Self::Quarter => "1/4",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
        }
    }

    fn ticks(self) -> i64 {
        match self {
            Self::Whole => PPQ * 4,
            Self::Half => PPQ * 2,
            Self::Quarter => PPQ,
            Self::Eighth => PPQ / 2,
            Self::Sixteenth => PPQ / 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    Create,
    Move,
    Resize,
}

#[derive(Clone, Copy, Debug)]
struct DragState {
    clip: ClipId,
    original: Option<Note>,
    preview: Note,
    mode: DragMode,
    start_pointer: Pos2,
}

#[derive(Debug)]
pub struct PianoRollState {
    pub pixels_per_beat: f32,
    pub snap: Snap,
    drag: Option<DragState>,
    marquee: Option<(Pos2, Pos2)>,
    active_clip: Option<ClipId>,
    context_note: Option<NoteId>,
    context_tick: Option<Tick>,
    context_pitch: Option<u8>,
}

impl Default for PianoRollState {
    fn default() -> Self {
        Self {
            pixels_per_beat: 90.0,
            snap: Snap::Quarter,
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
        self.controls(ui);

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
        let selected: BTreeSet<NoteId> = editor.session.selection().notes.clone();
        let px_per_tick = self.pixels_per_beat / PPQ as f32;
        let height = f32::from(PITCH_HIGH - PITCH_LOW + 1) * ROW_HEIGHT;
        let viewport_width = (ui.available_width() - KEY_WIDTH - 12.0).max(240.0);
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
                                self.drag.map(|drag| drag.preview),
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
                            if let Some((start, end)) = self.marquee {
                                let rect = Rect::from_two_pos(start, end);
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

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Zoom");
            ui.add(egui::Slider::new(&mut self.pixels_per_beat, 30.0..=500.0).show_value(false));
            ui.separator();
            ui.label("Snap");
            egui::ComboBox::from_id_salt("piano_roll_snap")
                .selected_text(self.snap.label())
                .show_ui(ui, |ui| {
                    for snap in Snap::ALL {
                        ui.selectable_value(&mut self.snap, snap, snap.label());
                    }
                });
            ui.separator();
            ui.label("Drag empty grid to create · drag notes to move · right edge resizes · Shift-drag selects notes · right-click menu · pinch or Ctrl-scroll to zoom");
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

        if response.hovered() {
            let zoom = ui.input(|input| input.zoom_delta());
            let ctrl_scroll = ui.input(|input| {
                if input.modifiers.command {
                    input.raw_scroll_delta.y
                } else {
                    0.0
                }
            });
            let factor = if (zoom - 1.0).abs() > 0.001 {
                zoom
            } else if ctrl_scroll.abs() > 0.0 {
                (ctrl_scroll * 0.01).exp()
            } else {
                1.0
            };
            if (factor - 1.0).abs() > 0.001 {
                self.pixels_per_beat = (self.pixels_per_beat * factor).clamp(30.0, 500.0);
                ui.ctx().request_repaint();
            }
        }

        if primary_pressed && response.hovered() {
            if let Some(position) = pointer {
                if grid.contains(position) {
                    if let Some(note) = hit_note(notes, position, origin, px_per_tick) {
                        let rect = note_rect(note, origin, px_per_tick);
                        let mode = if position.x > rect.max.x - 6.0 {
                            DragMode::Resize
                        } else {
                            DragMode::Move
                        };
                        editor.select_note(note.id, shift);
                        self.drag = Some(DragState {
                            clip: clip_id,
                            original: Some(note),
                            preview: note,
                            mode,
                            start_pointer: position,
                        });
                    } else if shift {
                        self.drag = None;
                        self.marquee = Some((position, position));
                    } else {
                        let tick = x_to_tick(position.x - origin.x, px_per_tick);
                        let max_start = (clip.length.raw() - 1).max(0);
                        let start = snap_tick(tick, self.snap).clamp(0, max_start);
                        let pitch = y_to_pitch(position.y - origin.y);
                        let length = self
                            .snap
                            .ticks()
                            .min((clip.length.raw() - start).max(1))
                            .max(1);
                        self.drag = Some(DragState {
                            clip: clip_id,
                            original: None,
                            preview: Note {
                                id: NoteId::from_raw(0),
                                pitch,
                                start: Tick::from_raw(start),
                                length: Tick::from_raw(length),
                                velocity: 100,
                                channel: 0,
                            },
                            mode: DragMode::Create,
                            start_pointer: position,
                        });
                    }
                }
            }
        }

        if primary_down {
            if let (Some(drag), Some(position)) = (self.drag, pointer) {
                let delta_ticks =
                    ((position.x - drag.start_pointer.x) / px_per_tick).round() as i64;
                let delta_pitch =
                    -((position.y - drag.start_pointer.y) / ROW_HEIGHT).round() as i32;
                let preview = match drag.mode {
                    DragMode::Create => {
                        let length = snap_tick(delta_ticks, self.snap)
                            .max(self.snap.ticks())
                            .min((clip.length.raw() - drag.preview.start.raw()).max(1));
                        Note {
                            length: Tick::from_raw(length),
                            ..drag.preview
                        }
                    }
                    DragMode::Move => moved_note(
                        drag.original.expect("move drags have an original note"),
                        delta_ticks,
                        delta_pitch,
                        clip.length,
                        self.snap,
                    ),
                    DragMode::Resize => resized_note(
                        drag.original.expect("resize drags have an original note"),
                        delta_ticks,
                        clip.length,
                        self.snap,
                    ),
                };
                self.drag = Some(DragState { preview, ..drag });
            }
            if let (Some((start, _)), Some(position)) = (self.marquee, pointer) {
                self.marquee = Some((start, position));
            }
        }

        if primary_released {
            if let Some((start, end)) = self.marquee.take() {
                let rect = Rect::from_two_pos(start, end);
                let ids: BTreeSet<NoteId> = notes
                    .iter()
                    .filter(|note| rect.intersects(note_rect(**note, origin, px_per_tick)))
                    .map(|note| note.id)
                    .collect();
                editor.select_notes(ids, true);
            }
            if let Some(drag) = self.drag.take() {
                match drag.mode {
                    DragMode::Create => editor.add_note(
                        drag.clip,
                        drag.preview.pitch,
                        drag.preview.start,
                        drag.preview.length,
                        drag.preview.velocity,
                    ),
                    DragMode::Move | DragMode::Resize => {
                        if let Some(original) = drag.original {
                            if drag.preview != original {
                                editor.replace_note(drag.clip, drag.preview);
                            }
                        }
                    }
                }
            }
        }

        if secondary_pressed && response.hovered() {
            if let Some(position) = pointer {
                if let Some(note) = hit_note(notes, position, origin, px_per_tick) {
                    editor.select_note(note.id, shift);
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
        response.context_menu(|ui| {
            if let Some(note_id) = context_note {
                if ui.button("Duplicate note").clicked() {
                    editor.duplicate_note(clip_id, note_id);
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
            } else {
                if let (Some(tick), Some(pitch)) = (context_tick, context_pitch) {
                    if ui.button("Add note here").clicked() {
                        let start = snap_tick(tick.raw(), snap).clamp(0, clip.length.raw() - 1);
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

fn draw_grid(
    painter: &egui::Painter,
    origin: Pos2,
    width: f32,
    height: f32,
    clip_length: Tick,
    pixels_per_beat: f32,
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
        let is_bar = beat % 4 == 0;
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
}

#[allow(clippy::too_many_arguments)]
fn draw_notes(
    painter: &egui::Painter,
    origin: Pos2,
    width: f32,
    notes: &[Note],
    selected: &BTreeSet<NoteId>,
    preview: Option<Note>,
    hovered: Option<NoteId>,
    px_per_tick: f32,
) {
    let grid = Rect::from_min_size(
        origin,
        vec2(width, f32::from(PITCH_HIGH - PITCH_LOW + 1) * ROW_HEIGHT),
    );
    let mut preview_drawn = false;
    for note in notes {
        let note = preview
            .filter(|preview| preview.id == note.id)
            .unwrap_or(*note);
        if preview.is_some_and(|preview| preview.id == note.id) {
            preview_drawn = true;
        }
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
        draw_note(painter, rect, note.pitch, color, Some(note.id) == hovered);
    }
    if let Some(preview) = preview {
        if !preview_drawn && preview.pitch >= PITCH_LOW && preview.pitch <= PITCH_HIGH {
            let rect = note_rect(preview, origin, px_per_tick);
            draw_note(
                painter,
                rect,
                preview.pitch,
                Color32::from_rgb(83, 184, 139),
                true,
            );
        }
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

fn moved_note(
    note: Note,
    delta_ticks: i64,
    delta_pitch: i32,
    clip_length: Tick,
    snap: Snap,
) -> Note {
    let start = snap_tick(note.start.raw() + delta_ticks, snap)
        .clamp(0, (clip_length.raw() - note.length.raw()).max(0));
    let pitch = (i32::from(note.pitch) + delta_pitch).clamp(0, 127) as u8;
    Note {
        start: Tick::from_raw(start),
        pitch,
        ..note
    }
}

fn resized_note(note: Note, delta_ticks: i64, clip_length: Tick, snap: Snap) -> Note {
    let length = snap_tick(note.length.raw() + delta_ticks, snap)
        .max(snap.ticks())
        .min((clip_length.raw() - note.start.raw()).max(1));
    Note {
        length: Tick::from_raw(length),
        ..note
    }
}

fn x_to_tick(x: f32, px_per_tick: f32) -> i64 {
    (x / px_per_tick).round() as i64
}

pub(crate) fn snap_tick(tick: i64, snap: Snap) -> i64 {
    let step = snap.ticks();
    ((tick as f64 / step as f64).round() as i64) * step
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

    #[test]
    fn snapping_rounds_to_the_nearest_step() {
        assert_eq!(snap_tick(0, Snap::Quarter), 0);
        assert_eq!(snap_tick(PPQ / 2, Snap::Quarter), PPQ);
        assert_eq!(snap_tick(PPQ / 4, Snap::Quarter), 0);
        assert_eq!(snap_tick(PPQ, Snap::Eighth), PPQ);
    }

    #[test]
    fn moving_a_note_clamps_to_the_clip() {
        let moved = moved_note(note(), -PPQ * 2, 2, Tick::from_raw(PPQ * 4), Snap::Quarter);
        assert_eq!(moved.start, Tick::ZERO);
        assert_eq!(moved.pitch, 62);

        let moved = moved_note(
            note(),
            PPQ * 4,
            -100,
            Tick::from_raw(PPQ * 4),
            Snap::Quarter,
        );
        assert_eq!(moved.start, Tick::from_raw(PPQ * 3));
        assert_eq!(moved.pitch, 0);
    }

    #[test]
    fn resizing_keeps_a_positive_length() {
        let resized = resized_note(note(), -PPQ * 4, Tick::from_raw(PPQ * 4), Snap::Quarter);
        assert_eq!(resized.length, Tick::from_raw(PPQ));
        let resized = resized_note(note(), PPQ * 2, Tick::from_raw(PPQ * 4), Snap::Quarter);
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

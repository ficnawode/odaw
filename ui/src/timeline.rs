use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, Sense, Stroke, pos2, vec2,
};
use odaw_core::{Clip, ClipContent, ClipId, Node, NodeId, NodeRole, PPQ, Project, Tick};

use crate::editor::{Editor, TRACK_COLORS};
use crate::piano_roll::{Snap, snap_tick};

const ROW_HEIGHT: f32 = 52.0;
const HEADER_WIDTH: f32 = 205.0;
const RULER_HEIGHT: f32 = 24.0;

#[derive(Clone, Debug)]
struct TrackRow {
    id: NodeId,
    name: String,
    color: Color32,
    devices: Vec<String>,
    clips: Vec<ClipRow>,
}

#[derive(Clone, Debug)]
struct ClipRow {
    id: ClipId,
    clip: Clip,
    peaks: Option<Arc<[f32]>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    Move,
    ResizeStart,
    ResizeEnd,
}

#[derive(Clone, Debug)]
struct ClipDrag {
    clip: ClipId,
    original: Clip,
    preview: Clip,
    track: NodeId,
    mode: DragMode,
    start_pointer: Pos2,
}

#[derive(Debug)]
pub struct TimelineState {
    pub pixels_per_beat: f32,
    drag: Option<ClipDrag>,
    context_clip: Option<(NodeId, ClipId)>,
    context_tick: Option<Tick>,
    pub hover_target: Option<(NodeId, Tick)>,
    pub rename_request: Option<NodeId>,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            pixels_per_beat: 84.0,
            drag: None,
            context_clip: None,
            context_tick: None,
            hover_target: None,
            rename_request: None,
        }
    }
}

impl TimelineState {
    pub fn clear(&mut self) {
        self.drag = None;
        self.context_clip = None;
        self.context_tick = None;
    }
}

pub fn show(ui: &mut egui::Ui, editor: &mut Editor, state: &mut TimelineState, snap: &mut Snap) {
    let project = editor.session.project().clone();
    let tracks = track_rows(&project, editor);
    let selected_track = editor.selected_track();
    let selected_clip = editor.selected_clip();

    ui.horizontal_wrapped(|ui| {
        if ui.button("+ Track").clicked() {
            editor.add_track();
        }
        ui.separator();
        ui.label("Zoom");
        ui.add(egui::Slider::new(&mut state.pixels_per_beat, 24.0..=320.0).show_value(false));
        ui.separator();
        ui.label("Snap");
        egui::ComboBox::from_id_salt("timeline_snap")
            .selected_text(snap.label())
            .show_ui(ui, |ui| {
                for value in Snap::ALL {
                    ui.selectable_value(snap, value, value.label());
                }
            });
        ui.separator();
        ui.small(
            "Drag clips to move · edges resize · right-click actions · Ctrl-scroll/pinch zoom",
        );
    });

    let end_tick = odaw_render::project_end_tick(&project)
        .unwrap_or(Tick::from_raw(16 * PPQ))
        .max(Tick::from_raw(16 * PPQ));
    let total_beats = (end_tick.raw().max(1) as f64 / PPQ as f64).ceil() as i64;
    let viewport_width = (ui.available_width() - HEADER_WIDTH - 18.0).max(320.0);
    let content_width = (total_beats as f32 * state.pixels_per_beat).max(viewport_width);
    let grid_height = (RULER_HEIGHT + tracks.len().max(1) as f32 * ROW_HEIGHT)
        .max((ui.available_height() - 10.0).max(160.0));

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.allocate_ui_with_layout(
                vec2(HEADER_WIDTH, RULER_HEIGHT),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_width(HEADER_WIDTH);
                    ui.add_space(6.0);
                    ui.small(format!(
                        "Tracks · {track_count}",
                        track_count = tracks.len()
                    ));
                },
            );
            for track in &tracks {
                track_header(ui, editor, track, selected_track == Some(track.id), state);
            }
        });

        egui::ScrollArea::horizontal()
            .id_salt("timeline_grid")
            .show(ui, |ui| {
                let (response, painter) =
                    ui.allocate_painter(vec2(content_width, grid_height), Sense::click_and_drag());
                let origin = response.rect.min;
                draw_grid(
                    &painter,
                    origin,
                    content_width,
                    grid_height,
                    state.pixels_per_beat,
                    total_beats,
                );

                let pointer = ui.input(|input| input.pointer.hover_pos());
                let mut hits: Vec<(Rect, NodeId, ClipId, Clip)> = Vec::new();
                let drag = state.drag.clone();

                for (index, track) in tracks.iter().enumerate() {
                    for clip in &track.clips {
                        let preview = drag
                            .as_ref()
                            .filter(|drag| drag.clip == clip.id)
                            .map(|drag| (&drag.preview, drag.track));
                        let (draw, draw_track) =
                            preview.map_or((&clip.clip, track.id), |(clip, track)| (clip, track));
                        let draw_index = tracks
                            .iter()
                            .position(|row| row.id == draw_track)
                            .unwrap_or(index);
                        let draw_y = origin.y + RULER_HEIGHT + draw_index as f32 * ROW_HEIGHT;
                        let rect = clip_rect(draw, origin, draw_y, state.pixels_per_beat);
                        let selected = selected_clip == Some(clip.id);
                        let hovered = pointer.is_some_and(|position| rect.contains(position));
                        draw_clip(
                            &painter,
                            rect,
                            draw,
                            clip.peaks.as_deref(),
                            track.color,
                            selected,
                            hovered,
                        );
                        if preview.is_some() {
                            painter.line_segment(
                                [rect.left_top(), rect.right_top()],
                                Stroke::new(2.0_f32, Color32::from_rgb(118, 224, 166)),
                            );
                        }
                        hits.push((rect, track.id, clip.id, clip.clip.clone()));
                    }
                }

                if let Some(position) = pointer {
                    let track = track_at_y(position.y, origin, &tracks);
                    let tick = if position.x >= origin.x {
                        Some(Tick::from_raw(
                            ((position.x - origin.x) / state.pixels_per_beat * PPQ as f32).round()
                                as i64,
                        ))
                    } else {
                        None
                    };
                    state.hover_target = track.zip(tick);
                } else {
                    state.hover_target = None;
                }

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
                        state.pixels_per_beat = (state.pixels_per_beat * factor).clamp(24.0, 320.0);
                        ui.ctx().request_repaint();
                    }
                }

                let secondary = ui.input(|input| input.pointer.secondary_pressed());
                if secondary && response.hovered() {
                    if let Some(position) = pointer {
                        state.context_clip = hits
                            .iter()
                            .find(|(rect, _, _, _)| rect.contains(position))
                            .map(|(_, track, clip, _)| (*track, *clip));
                        state.context_tick = Some(Tick::from_raw(
                            ((position.x - origin.x) / state.pixels_per_beat * PPQ as f32)
                                .round()
                                .max(0.0) as i64,
                        ));
                    }
                }

                if ui.input(|input| input.pointer.primary_pressed()) && response.hovered() {
                    if let Some(position) = pointer {
                        if let Some((rect, track, clip, value)) =
                            hits.iter().find(|(rect, _, _, _)| rect.contains(position))
                        {
                            editor.select_clip(*track, *clip);
                            let mode = if position.x > rect.max.x - 7.0 {
                                DragMode::ResizeEnd
                            } else if position.x < rect.min.x + 7.0 {
                                DragMode::ResizeStart
                            } else {
                                DragMode::Move
                            };
                            state.drag = Some(ClipDrag {
                                clip: *clip,
                                original: value.clone(),
                                preview: value.clone(),
                                track: *track,
                                mode,
                                start_pointer: position,
                            });
                        } else {
                            state.drag = None;
                        }
                    }
                }

                if let (Some(drag), Some(position)) = (state.drag.clone(), pointer) {
                    if ui.input(|input| input.pointer.primary_down()) {
                        let delta_ticks = ((position.x - drag.start_pointer.x)
                            / state.pixels_per_beat
                            * PPQ as f32)
                            .round() as i64;
                        let target_track =
                            track_at_y(position.y, origin, &tracks).unwrap_or(drag.track);
                        let mut preview = drag.original.clone();
                        match drag.mode {
                            DragMode::Move => {
                                preview.start = Tick::from_raw(
                                    snap_tick(drag.original.start.raw() + delta_ticks, *snap)
                                        .max(0),
                                );
                                preview.node = target_track;
                            }
                            DragMode::ResizeStart => {
                                let end = drag.original.start.raw() + drag.original.length.raw();
                                let start =
                                    snap_tick(drag.original.start.raw() + delta_ticks, *snap)
                                        .clamp(0, (end - 1).max(0));
                                preview.start = Tick::from_raw(start);
                                preview.length = Tick::from_raw((end - start).max(1));
                            }
                            DragMode::ResizeEnd => {
                                preview.length = Tick::from_raw(
                                    snap_tick(drag.original.length.raw() + delta_ticks, *snap)
                                        .max(1),
                                );
                            }
                        }
                        state.drag = Some(ClipDrag {
                            preview,
                            track: target_track,
                            ..drag
                        });
                    }
                }

                if ui.input(|input| input.pointer.primary_released()) {
                    if let Some(drag) = state.drag.take() {
                        if drag.preview != drag.original {
                            editor.replace_clip(drag.clip, drag.preview);
                        }
                    }
                }

                if ui.input(|input| !input.raw.hovered_files.is_empty()) {
                    if let Some((track, tick)) = state.hover_target {
                        if let Some(index) = tracks.iter().position(|row| row.id == track) {
                            let y = origin.y + RULER_HEIGHT + index as f32 * ROW_HEIGHT;
                            let x =
                                origin.x + tick.raw() as f32 / PPQ as f32 * state.pixels_per_beat;
                            painter.line_segment(
                                [pos2(x, y), pos2(x, y + ROW_HEIGHT)],
                                Stroke::new(2.0_f32, Color32::from_rgb(83, 184, 139)),
                            );
                            painter.text(
                                pos2(x + 4.0, y + 5.0),
                                Align2::LEFT_TOP,
                                "Drop audio here",
                                FontId::proportional(10.0),
                                Color32::from_rgb(180, 240, 205),
                            );
                        }
                    }
                }

                let progress = editor.progress();
                let playhead_x = origin.x + progress * content_width;
                painter.line_segment(
                    [
                        pos2(playhead_x, origin.y + RULER_HEIGHT - 6.0),
                        pos2(playhead_x, origin.y + grid_height),
                    ],
                    Stroke::new(1.5_f32, Color32::from_rgb(244, 92, 92)),
                );

                let context_clip = state.context_clip;
                let context_tick = state.context_tick;
                response.context_menu(|ui| {
                    if let Some((track, clip)) = context_clip {
                        if ui.button("Open in piano roll").clicked() {
                            editor.select_clip(track, clip);
                            ui.close();
                        }
                        if ui.button("Duplicate clip").clicked() {
                            editor.duplicate_clip(clip);
                            ui.close();
                        }
                        if let Some(tick) = context_tick {
                            if ui.button("Split clip here").clicked() {
                                editor.split_clip(clip, tick);
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button("Delete clip").clicked() {
                            editor.delete_clip(clip);
                            ui.close();
                        }
                    } else {
                        if ui.button("Add track").clicked() {
                            editor.add_track();
                            ui.close();
                        }
                        if ui.button("Add MIDI clip").clicked() {
                            editor.add_midi_clip();
                            ui.close();
                        }
                        ui.separator();
                        ui.label("Drop .wav or .mp3 to import audio");
                    }
                });
            });
    });
}

fn track_header(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    track: &TrackRow,
    selected: bool,
    state: &mut TimelineState,
) {
    ui.allocate_ui_with_layout(
        vec2(HEADER_WIDTH, ROW_HEIGHT),
        Layout::top_down(Align::Min),
        |ui| {
            ui.set_width(HEADER_WIDTH);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.colored_label(track.color, "■");
                let response = ui.selectable_label(selected, &track.name);
                if response.clicked() {
                    editor.select_track(track.id);
                }
                response.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        state.rename_request = Some(track.id);
                        ui.close();
                    }
                    ui.menu_button("Color", |ui| {
                        for color in TRACK_COLORS {
                            let button = egui::Button::new("   ").fill(rgb(color));
                            if ui.add(button).clicked() {
                                editor.set_track_color(track.id, Some(color));
                                ui.close();
                            }
                        }
                        if ui.button("No color").clicked() {
                            editor.set_track_color(track.id, None);
                            ui.close();
                        }
                    });
                    if ui.button("Add MIDI clip").clicked() {
                        editor.select_track(track.id);
                        editor.add_midi_clip();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Delete track").clicked() {
                        editor.delete_track(track.id);
                        ui.close();
                    }
                });

                let mute = ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("M").color(Color32::from_rgb(255, 225, 225)),
                        )
                        .min_size(vec2(18.0, 18.0))
                        .fill(if track_muted(track, editor) {
                            Color32::from_rgb(244, 58, 58)
                        } else {
                            Color32::from_rgb(190, 104, 104)
                        }),
                    )
                    .on_hover_text("Mute track");
                if mute.clicked() {
                    let muted = track_muted(track, editor);
                    editor.set_track_muted(track.id, !muted);
                }
                let solo = ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("S").color(Color32::from_rgb(220, 235, 255)),
                        )
                        .min_size(vec2(18.0, 18.0))
                        .fill(if track_soloed(track, editor) {
                            Color32::from_rgb(52, 150, 255)
                        } else {
                            Color32::from_rgb(86, 128, 190)
                        }),
                    )
                    .on_hover_text("Solo track");
                if solo.clicked() {
                    let soloed = track_soloed(track, editor);
                    editor.set_track_soloed(track.id, !soloed);
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.small(
                    track
                        .devices
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(" · "),
                );
            });
        },
    );
}

fn draw_grid(
    painter: &egui::Painter,
    origin: Pos2,
    width: f32,
    height: f32,
    pixels_per_beat: f32,
    total_beats: i64,
) {
    painter.rect_filled(
        Rect::from_min_size(origin, vec2(width, height)),
        0.0,
        Color32::from_rgb(23, 25, 29),
    );
    painter.rect_filled(
        Rect::from_min_size(origin, vec2(width, RULER_HEIGHT)),
        0.0,
        Color32::from_rgb(27, 29, 34),
    );

    let subdivisions = if pixels_per_beat >= 220.0 {
        4
    } else if pixels_per_beat >= 110.0 {
        2
    } else {
        1
    };
    for beat in 0..=total_beats {
        for subdivision in 0..subdivisions {
            if subdivision == 0 && beat == 0 {
                continue;
            }
            let position = beat as f32 + subdivision as f32 / subdivisions as f32;
            let x = origin.x + position * pixels_per_beat;
            if x > origin.x + width {
                break;
            }
            let bar = beat % 4 == 0 && subdivision == 0;
            painter.line_segment(
                [pos2(x, origin.y + RULER_HEIGHT), pos2(x, origin.y + height)],
                Stroke::new(
                    if bar {
                        1.0_f32
                    } else if subdivision == 0 {
                        0.6_f32
                    } else {
                        0.3_f32
                    },
                    if bar {
                        Color32::from_rgb(62, 66, 76)
                    } else if subdivision == 0 {
                        Color32::from_rgb(42, 46, 53)
                    } else {
                        Color32::from_rgb(33, 36, 42)
                    },
                ),
            );
        }
        if beat % 4 == 0 {
            let x = origin.x + beat as f32 * pixels_per_beat;
            painter.text(
                pos2(x + 4.0, origin.y + 3.0),
                Align2::LEFT_TOP,
                format!("{}", beat / 4 + 1),
                FontId::monospace(9.0),
                Color32::from_rgb(145, 151, 164),
            );
        }
    }
}

fn draw_clip(
    painter: &egui::Painter,
    rect: Rect,
    clip: &Clip,
    peaks: Option<&[f32]>,
    color: Color32,
    selected: bool,
    hovered: bool,
) {
    painter.rect_filled(
        rect,
        4.0,
        color.gamma_multiply(if selected { 1.08 } else { 0.9 }),
    );

    match &clip.content {
        ClipContent::Midi { notes, .. } => draw_midi_preview(painter, rect, notes, clip.length),
        ClipContent::Audio { .. } => {
            if let Some(peaks) = peaks {
                draw_waveform(painter, rect, peaks);
            }
        }
        ClipContent::Empty => {}
    }

    if selected {
        painter.line_segment(
            [rect.left_top(), rect.right_top()],
            Stroke::new(2.0_f32, Color32::from_rgb(250, 205, 96)),
        );
    } else if hovered {
        painter.line_segment(
            [rect.left_top(), rect.right_top()],
            Stroke::new(1.0_f32, Color32::from_rgb(225, 232, 242)),
        );
    }
    let kind = match &clip.content {
        ClipContent::Midi { .. } => "MIDI",
        ClipContent::Audio { .. } => "AUDIO",
        ClipContent::Empty => "EMPTY",
    };
    painter.text(
        pos2(rect.min.x + 5.0, rect.min.y + 4.0),
        Align2::LEFT_TOP,
        kind.to_owned(),
        FontId::monospace(8.0),
        Color32::from_rgb(18, 20, 24),
    );
}

fn draw_midi_preview(
    painter: &egui::Painter,
    rect: Rect,
    notes: &odaw_core::PersistentVector<odaw_core::Note>,
    clip_length: Tick,
) {
    if notes.is_empty() || clip_length.raw() <= 0 {
        return;
    }
    let mut low = u8::MAX;
    let mut high = 0u8;
    for note in notes {
        low = low.min(note.pitch);
        high = high.max(note.pitch);
    }
    let low = low.saturating_sub(2) as f32;
    let high = (high.saturating_add(2) as f32).max(low + 12.0);
    let span = high - low;
    let top = rect.min.y + 12.0;
    let usable = (rect.height() - 14.0).max(2.0);
    for note in notes {
        let x =
            rect.min.x + note.start.raw() as f32 / clip_length.raw().max(1) as f32 * rect.width();
        let width =
            (note.length.raw() as f32 / clip_length.raw().max(1) as f32 * rect.width()).max(1.0);
        let normalized = (high - note.pitch as f32) / span;
        let y = top + normalized * usable;
        painter.rect_filled(
            Rect::from_min_size(pos2(x, y), vec2(width, 1.5)),
            0.0,
            Color32::from_rgba_unmultiplied(10, 14, 20, 150),
        );
    }
}

fn draw_waveform(painter: &egui::Painter, rect: Rect, peaks: &[f32]) {
    if peaks.is_empty() || rect.width() < 4.0 {
        return;
    }
    let center = rect.center().y;
    let amplitude = (rect.height() * 0.5 - 6.0).max(1.0);
    let color = Color32::from_rgba_unmultiplied(10, 14, 20, 150);
    let mut x = rect.min.x + 1.0;
    while x < rect.max.x - 1.0 {
        let ratio = (x - rect.min.x) / rect.width();
        let index = ((ratio * peaks.len() as f32) as usize).min(peaks.len() - 1);
        let height = (peaks[index] * amplitude).clamp(0.4, amplitude);
        painter.line_segment(
            [pos2(x, center - height), pos2(x, center + height)],
            Stroke::new(1.0_f32, color),
        );
        x += 2.0;
    }
}

fn track_rows(project: &Project, editor: &Editor) -> Vec<TrackRow> {
    project
        .nodes
        .iter()
        .filter(|(_, node)| node.role == NodeRole::Track)
        .map(|(id, node)| TrackRow {
            id: *id,
            name: node
                .name
                .clone()
                .unwrap_or_else(|| format!("Track {}", id.raw())),
            color: track_color(node, *id),
            devices: node
                .rack
                .iter()
                .filter_map(|device| project.nodes.get(device))
                .filter_map(|node| node.kind.as_ref())
                .map(|kind| kind.as_str().to_owned())
                .collect(),
            clips: node
                .clips
                .iter()
                .filter_map(|clip_id| {
                    project.clips.get(clip_id).map(|clip| ClipRow {
                        id: *clip_id,
                        clip: clip.clone(),
                        peaks: match &clip.content {
                            ClipContent::Audio { asset, .. } => editor.audio_peaks(*asset),
                            ClipContent::Midi { .. } | ClipContent::Empty => None,
                        },
                    })
                })
                .collect(),
        })
        .collect()
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

fn track_muted(track: &TrackRow, editor: &Editor) -> bool {
    editor
        .session
        .project()
        .nodes
        .get(&track.id)
        .is_some_and(|node| node.muted)
}

fn track_soloed(track: &TrackRow, editor: &Editor) -> bool {
    editor
        .session
        .project()
        .nodes
        .get(&track.id)
        .is_some_and(|node| node.soloed)
}

fn track_at_y(y: f32, origin: Pos2, tracks: &[TrackRow]) -> Option<NodeId> {
    let row = ((y - origin.y - RULER_HEIGHT) / ROW_HEIGHT).floor() as isize;
    if row < 0 {
        return None;
    }
    tracks.get(row as usize).map(|track| track.id)
}

fn clip_rect(clip: &Clip, origin: Pos2, row_y: f32, pixels_per_beat: f32) -> Rect {
    let x = origin.x + clip.start.raw().max(0) as f32 / PPQ as f32 * pixels_per_beat;
    let width = clip.length.raw().max(1) as f32 / PPQ as f32 * pixels_per_beat;
    Rect::from_min_size(pos2(x, row_y + 4.0), vec2(width.max(4.0), ROW_HEIGHT - 8.0))
}

use std::collections::BTreeSet;
use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, Sense, Stroke, pos2, vec2,
};
use odaw_core::{Clip, ClipContent, ClipId, Frames, Node, NodeId, NodeRole, PPQ, Project, Tick};

use crate::editor::{Editor, TRACK_COLORS};
use crate::mixer::MixerState;
use crate::piano_roll::{Snap, snap_tick};

const ROW_HEIGHT: f32 = 58.0;
const HEADER_WIDTH: f32 = 215.0;
const RULER_HEIGHT: f32 = 24.0;

#[derive(Clone, Debug)]
struct TrackRow {
    id: NodeId,
    name: String,
    color: Color32,
    clips: Vec<ClipRow>,
}

#[derive(Clone, Debug)]
struct ClipRow {
    id: ClipId,
    clip: Clip,
    peaks: Option<Arc<[f32]>>,
    source_frames: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    Move,
    ResizeStart,
    ResizeEnd,
}

#[derive(Clone, Debug)]
struct ClipDrag {
    primary: ClipId,
    primary_track: NodeId,
    originals: Vec<(ClipId, Clip)>,
    previews: Vec<(ClipId, Clip)>,
    mode: DragMode,
    start_pointer: Pos2,
}

impl ClipDrag {
    fn preview(&self, clip: ClipId) -> Option<&Clip> {
        self.previews
            .iter()
            .find(|(id, _)| *id == clip)
            .map(|(_, value)| value)
    }
}

#[derive(Clone, Debug)]
struct EmptyDrag {
    start: Pos2,
    current: Pos2,
    moved: bool,
}

impl EmptyDrag {
    fn rect(&self) -> Rect {
        Rect::from_two_pos(self.start, self.current)
    }
}

#[derive(Debug)]
pub struct TimelineState {
    pub pixels_per_beat: f32,
    pub snap: Snap,
    pub auto_snap: bool,
    drag: Option<ClipDrag>,
    marquee: Option<EmptyDrag>,
    seek_drag: Option<Pos2>,
    context_clip: Option<(NodeId, ClipId)>,
    context_tick: Option<Tick>,
    pub hover_target: Option<(NodeId, Tick)>,
    pub rename_request: Option<NodeId>,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            pixels_per_beat: 84.0,
            snap: Snap::Quarter,
            auto_snap: true,
            drag: None,
            marquee: None,
            seek_drag: None,
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
        self.marquee = None;
        self.seek_drag = None;
        self.context_clip = None;
        self.context_tick = None;
    }
}

pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    mixer: &mut MixerState,
    state: &mut TimelineState,
) {
    if state.auto_snap {
        state.snap = Snap::for_zoom(state.pixels_per_beat);
    }
    let project = editor.session.project().clone();
    let tracks = track_rows(&project, editor);
    let master = master_node(&project);
    let selected_track = editor.selected_track();
    let selected_clips = editor.selected_clips();

    ui.horizontal_wrapped(|ui| {
        if ui.button("+ Track").clicked() {
            editor.add_track();
        }
        if ui
            .button("Split @ cursor")
            .on_hover_text("Split the selected clips at the red cursor (Ctrl+E)")
            .clicked()
        {
            editor.split_selected_at_cursor();
        }
        ui.separator();
        ui.label("Zoom");
        ui.add(egui::Slider::new(&mut state.pixels_per_beat, 24.0..=320.0).show_value(false));
        ui.separator();
        ui.label("Snap");
        egui::ComboBox::from_id_salt("timeline_snap")
            .selected_text(state.snap.label())
            .show_ui(ui, |ui| {
                for value in Snap::ALL {
                    if ui
                        .selectable_label(state.snap == value, value.label())
                        .clicked()
                    {
                        state.snap = value;
                        state.auto_snap = false;
                    }
                }
            });
        ui.checkbox(&mut state.auto_snap, "Auto")
            .on_hover_text("Follow the zoom level");
        ui.separator();
        ui.small(
            "Click the ruler or empty space to seek · drag empty space to marquee-select · \
             Shift adds · Ctrl+E splits at cursor",
        );
    });

    let end_tick = odaw_render::project_end_tick(&project)
        .unwrap_or(Tick::from_raw(16 * PPQ))
        .max(Tick::from_raw(16 * PPQ));
    let total_beats = (end_tick.raw().max(1) as f64 / PPQ as f64).ceil() as i64;
    let beats_per_bar = project
        .signatures
        .iter()
        .next()
        .map_or(4, |signature| signature.numerator.max(1) as i64);
    let viewport_width = (ui.available_width() - HEADER_WIDTH - 18.0).max(320.0);
    let content_width = (total_beats as f32 * state.pixels_per_beat).max(viewport_width);
    let row_count = tracks.len() + usize::from(master.is_some());
    let grid_height = (RULER_HEIGHT + row_count.max(1) as f32 * ROW_HEIGHT)
        .max((ui.available_height() - 10.0).max(160.0));

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            header_row(
                ui,
                RULER_HEIGHT,
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.add_space(6.0);
                    ui.small(format!("Tracks · {}", tracks.len()));
                },
            );
            for track in &tracks {
                track_header(
                    ui,
                    editor,
                    mixer,
                    track,
                    selected_track == Some(track.id),
                    state,
                );
            }
            if let Some(master) = master {
                master_header(ui, editor, mixer, master);
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
                    state.snap,
                    total_beats,
                    beats_per_bar,
                    tracks.len(),
                    master.is_some(),
                );

                let pointer = ui.input(|input| input.pointer.hover_pos());
                let shift = ui.input(|input| input.modifiers.shift);
                let mut hits: Vec<(Rect, NodeId, ClipId, Clip)> = Vec::new();
                let drag = state.drag.clone();

                for (index, track) in tracks.iter().enumerate() {
                    for clip in &track.clips {
                        let preview = drag
                            .as_ref()
                            .and_then(|drag| drag.preview(clip.id))
                            .map(|value| (value, value.node))
                            .unwrap_or((&clip.clip, track.id));
                        let (draw, draw_track) = preview;
                        let draw_index = tracks
                            .iter()
                            .position(|row| row.id == draw_track)
                            .unwrap_or(index);
                        let draw_y = origin.y + RULER_HEIGHT + draw_index as f32 * ROW_HEIGHT;
                        let rect = clip_rect(draw, origin, draw_y, state.pixels_per_beat);
                        let selected = selected_clips.contains(&clip.id);
                        let hovered = pointer.is_some_and(|position| rect.contains(position));
                        draw_clip(
                            &painter,
                            rect,
                            draw,
                            clip.peaks.as_deref(),
                            clip.source_frames,
                            track.color,
                            selected,
                            hovered,
                        );
                        if drag
                            .as_ref()
                            .is_some_and(|drag| drag.preview(clip.id).is_some())
                        {
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
                        Some(tick_at(
                            position.x,
                            origin,
                            state.pixels_per_beat,
                            state.snap,
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
                        if state.auto_snap {
                            state.snap = Snap::for_zoom(state.pixels_per_beat);
                        }
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
                        state.context_tick = Some(tick_at(
                            position.x,
                            origin,
                            state.pixels_per_beat,
                            state.snap,
                        ));
                    }
                }

                if ui.input(|input| input.pointer.primary_pressed()) && response.hovered() {
                    if let Some(position) = pointer {
                        let ruler = position.y < origin.y + RULER_HEIGHT;
                        let track_lane = track_at_y(position.y, origin, &tracks).is_some();
                        let hit = hits
                            .iter()
                            .find(|(rect, _, _, _)| rect.contains(position))
                            .cloned();
                        state.marquee = None;
                        state.seek_drag = None;

                        if ruler {
                            state.seek_drag = Some(position);
                            let tick =
                                tick_at(position.x, origin, state.pixels_per_beat, state.snap);
                            editor.preview_cursor(tick);
                        } else if let Some((rect, track, clip, value)) = hit {
                            if shift {
                                editor.toggle_clip(track, clip);
                                state.drag = None;
                            } else {
                                if !editor.is_clip_selected(clip) {
                                    editor.select_clip(track, clip);
                                }
                                let selected = editor.selected_clips();
                                let mut originals: Vec<(ClipId, Clip)> = selected
                                    .iter()
                                    .filter_map(|id| {
                                        editor
                                            .session
                                            .project()
                                            .clips
                                            .get(id)
                                            .map(|clip| (*id, clip.clone()))
                                    })
                                    .collect();
                                if originals.is_empty() {
                                    originals.push((clip, value.clone()));
                                }
                                let single = originals.len() == 1;
                                let mode = if single && position.x > rect.max.x - 7.0 {
                                    DragMode::ResizeEnd
                                } else if single && position.x < rect.min.x + 7.0 {
                                    DragMode::ResizeStart
                                } else {
                                    DragMode::Move
                                };
                                state.drag = Some(ClipDrag {
                                    primary: clip,
                                    primary_track: track,
                                    previews: originals.clone(),
                                    originals,
                                    mode,
                                    start_pointer: position,
                                });
                            }
                        } else if track_lane || position.y < origin.y + grid_height {
                            state.marquee = Some(EmptyDrag {
                                start: position,
                                current: position,
                                moved: false,
                            });
                        }
                    }
                }

                if let Some(position) = pointer {
                    if ui.input(|input| input.pointer.primary_down()) {
                        if state.seek_drag.is_some() {
                            let tick =
                                tick_at(position.x, origin, state.pixels_per_beat, state.snap);
                            editor.preview_cursor(tick);
                        }
                        if let Some(drag) = state.drag.clone() {
                            let delta_ticks = ((position.x - drag.start_pointer.x)
                                / state.pixels_per_beat
                                * PPQ as f32)
                                .round() as i64;
                            let target_track = track_at_y(position.y, origin, &tracks)
                                .unwrap_or(drag.primary_track);
                            let target_index = tracks
                                .iter()
                                .position(|row| row.id == target_track)
                                .unwrap_or(0);
                            let primary_index = tracks
                                .iter()
                                .position(|row| row.id == drag.primary_track)
                                .unwrap_or(target_index);
                            let track_delta = target_index as isize - primary_index as isize;

                            let mut previews = drag.previews.clone();
                            match drag.mode {
                                DragMode::Move => {
                                    for (id, preview) in &mut previews {
                                        let Some((_, original)) =
                                            drag.originals.iter().find(|(other, _)| other == id)
                                        else {
                                            continue;
                                        };
                                        preview.start = Tick::from_raw(
                                            snap_tick(
                                                original.start.raw() + delta_ticks,
                                                state.snap,
                                            )
                                            .max(0),
                                        );
                                        if let Some(original_index) =
                                            tracks.iter().position(|row| row.id == original.node)
                                        {
                                            let index = (original_index as isize + track_delta)
                                                .clamp(0, tracks.len() as isize - 1)
                                                as usize;
                                            preview.node = tracks[index].id;
                                        }
                                    }
                                }
                                DragMode::ResizeStart => {
                                    if let Some((_, original)) =
                                        drag.originals.iter().find(|(id, _)| *id == drag.primary)
                                    {
                                        let end = original.start.raw() + original.length.raw();
                                        let start = snap_tick(
                                            original.start.raw() + delta_ticks,
                                            state.snap,
                                        )
                                        .clamp(0, (end - 1).max(0));
                                        if let Some((_, preview)) =
                                            previews.iter_mut().find(|(id, _)| *id == drag.primary)
                                        {
                                            preview.start = Tick::from_raw(start);
                                            preview.length = Tick::from_raw((end - start).max(1));
                                            trim_audio_start(
                                                asset_frames(editor, original),
                                                original,
                                                preview,
                                                start,
                                            );
                                        }
                                    }
                                }
                                DragMode::ResizeEnd => {
                                    if let Some((_, original)) =
                                        drag.originals.iter().find(|(id, _)| *id == drag.primary)
                                    {
                                        let length = snap_tick(
                                            original.length.raw() + delta_ticks,
                                            state.snap,
                                        )
                                        .max(1);
                                        if let Some((_, preview)) =
                                            previews.iter_mut().find(|(id, _)| *id == drag.primary)
                                        {
                                            preview.length = Tick::from_raw(length);
                                            trim_audio_end(
                                                asset_frames(editor, original),
                                                original,
                                                preview,
                                                length,
                                            );
                                        }
                                    }
                                }
                            }
                            state.drag = Some(ClipDrag { previews, ..drag });
                        }
                        if let Some(drag) = state.marquee.clone() {
                            let moved = drag.start.distance(position) > 4.0;
                            state.marquee = Some(EmptyDrag {
                                current: position,
                                moved,
                                ..drag
                            });
                        }
                    }
                }

                if ui.input(|input| input.pointer.primary_released()) {
                    if let Some(drag) = state.drag.take() {
                        let changed: Vec<(ClipId, Clip)> = drag
                            .previews
                            .into_iter()
                            .filter_map(|(id, preview)| {
                                let (_, original) =
                                    drag.originals.iter().find(|(other, _)| *other == id)?;
                                (preview != *original).then_some((id, preview))
                            })
                            .collect();
                        editor.replace_clips(changed);
                    }
                    if let Some(drag) = state.marquee.take() {
                        if drag.moved {
                            let rect = drag.rect();
                            let clips: BTreeSet<ClipId> = hits
                                .iter()
                                .filter(|(clip_rect, _, _, _)| rect.intersects(*clip_rect))
                                .map(|(_, _, clip, _)| *clip)
                                .collect();
                            if clips.is_empty() && !shift {
                                editor.clear_clip_selection();
                            } else if !clips.is_empty() {
                                if shift {
                                    let mut combined = editor.selected_clips();
                                    combined.extend(clips);
                                    editor.select_clips(combined);
                                } else {
                                    editor.select_clips(clips);
                                }
                            }
                        } else if let Some(position) = pointer {
                            editor.clear_clip_selection();
                            let tick =
                                tick_at(position.x, origin, state.pixels_per_beat, state.snap);
                            editor.set_cursor(tick);
                        }
                    }
                    if state.seek_drag.take().is_some() {
                        if let Some(position) = pointer {
                            let tick =
                                tick_at(position.x, origin, state.pixels_per_beat, state.snap);
                            editor.set_cursor(tick);
                        }
                    }
                }

                if let Some(drag) = &state.marquee {
                    if drag.moved {
                        let rect = drag.rect();
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

                let cursor = editor.playhead_tick();
                let playhead_x = (origin.x
                    + cursor.raw().max(0) as f32 / PPQ as f32 * state.pixels_per_beat)
                    .clamp(origin.x, origin.x + content_width);
                painter.line_segment(
                    [
                        pos2(playhead_x, origin.y + RULER_HEIGHT - 8.0),
                        pos2(playhead_x, origin.y + grid_height),
                    ],
                    Stroke::new(1.5_f32, Color32::from_rgb(244, 92, 92)),
                );
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(playhead_x - 5.0, origin.y + 2.0),
                        pos2(playhead_x + 5.0, origin.y + 2.0),
                        pos2(playhead_x, origin.y + 10.0),
                    ],
                    Color32::from_rgb(244, 92, 92),
                    Stroke::NONE,
                ));

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
                        if ui.button("Split at cursor").clicked() {
                            editor.split_selected_at_cursor();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Delete clip").clicked() {
                            editor.delete_clip(clip);
                            ui.close();
                        }
                    } else {
                        if let Some(tick) = context_tick {
                            if ui.button("Move cursor here").clicked() {
                                editor.set_cursor(tick);
                                ui.close();
                            }
                        }
                        if ui.button("Split clips at cursor").clicked() {
                            editor.split_selected_at_cursor();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Select all clips").clicked() {
                            editor.select_all_clips();
                            ui.close();
                        }
                        if ui.button("Clear selection").clicked() {
                            editor.clear_clip_selection();
                            ui.close();
                        }
                        ui.separator();
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

fn header_row(
    ui: &mut egui::Ui,
    height: f32,
    layout: Layout,
    contents: impl FnOnce(&mut egui::Ui),
) {
    let (rect, _) = ui.allocate_exact_size(vec2(HEADER_WIDTH, height), Sense::hover());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
    child.set_width(HEADER_WIDTH);
    child.set_clip_rect(rect);
    child.spacing_mut().item_spacing.y = 0.0;
    contents(&mut child);
}

fn track_header(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    mixer: &mut MixerState,
    track: &TrackRow,
    selected: bool,
    state: &mut TimelineState,
) {
    header_row(ui, ROW_HEIGHT, Layout::top_down(Align::Min), |ui| {
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
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(8.0);
            mixer.gain(ui, editor, track.id, true);
            mixer.pan(ui, editor, track.id, true);
            mixer.meter(ui, editor, track.id);
        });
    });
}

fn master_header(ui: &mut egui::Ui, editor: &mut Editor, mixer: &mut MixerState, master: NodeId) {
    header_row(ui, ROW_HEIGHT, Layout::top_down(Align::Min), |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.colored_label(Color32::from_rgb(196, 202, 214), "◆");
            ui.strong("Master");
            ui.small("stereo out");
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(8.0);
            mixer.gain(ui, editor, master, true);
            mixer.pan(ui, editor, master, true);
            mixer.meter(ui, editor, master);
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_grid(
    painter: &egui::Painter,
    origin: Pos2,
    width: f32,
    height: f32,
    pixels_per_beat: f32,
    snap: Snap,
    total_beats: i64,
    beats_per_bar: i64,
    track_rows: usize,
    has_master: bool,
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

    if has_master {
        let y = origin.y + RULER_HEIGHT + track_rows as f32 * ROW_HEIGHT;
        painter.rect_filled(
            Rect::from_min_size(pos2(origin.x, y), vec2(width, ROW_HEIGHT)),
            0.0,
            Color32::from_rgb(20, 22, 26),
        );
        painter.text(
            pos2(origin.x + 8.0, y + ROW_HEIGHT / 2.0),
            Align2::LEFT_CENTER,
            "MASTER OUT · STEREO",
            FontId::monospace(8.0),
            Color32::from_rgb(92, 98, 110),
        );
    }

    let bar_ticks = beats_per_bar.max(1) * PPQ;
    for beat in 0..=total_beats {
        let x = origin.x + beat as f32 * pixels_per_beat;
        if x > origin.x + width {
            break;
        }
        let bar = beat % beats_per_bar.max(1) == 0;
        painter.line_segment(
            [pos2(x, origin.y + RULER_HEIGHT), pos2(x, origin.y + height)],
            Stroke::new(
                if bar { 1.0_f32 } else { 0.6_f32 },
                if bar {
                    Color32::from_rgb(62, 66, 76)
                } else {
                    Color32::from_rgb(42, 46, 53)
                },
            ),
        );
        if bar {
            painter.text(
                pos2(x + 4.0, origin.y + 3.0),
                Align2::LEFT_TOP,
                format!("{}", beat / beats_per_bar.max(1) + 1),
                FontId::monospace(9.0),
                Color32::from_rgb(145, 151, 164),
            );
        }
    }

    let subdivision = snap.ticks();
    if subdivision < PPQ {
        let total_ticks = total_beats * PPQ;
        let mut tick = subdivision;
        while tick < total_ticks {
            if tick % PPQ != 0 && tick % bar_ticks != 0 {
                let x = origin.x + tick as f32 / PPQ as f32 * pixels_per_beat;
                if x > origin.x + width {
                    break;
                }
                painter.line_segment(
                    [pos2(x, origin.y + RULER_HEIGHT), pos2(x, origin.y + height)],
                    Stroke::new(0.3_f32, Color32::from_rgb(33, 36, 42)),
                );
            }
            tick += subdivision;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_clip(
    painter: &egui::Painter,
    rect: Rect,
    clip: &Clip,
    peaks: Option<&[f32]>,
    source_frames: Option<u64>,
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
        ClipContent::Audio {
            source_start,
            source_length,
            ..
        } => {
            if let Some(peaks) = peaks {
                draw_waveform(
                    painter,
                    rect,
                    peaks,
                    source_frames.unwrap_or(1).max(1),
                    source_start.raw().max(0) as f64,
                    source_length.raw().max(0) as f64,
                );
            }
        }
        ClipContent::Empty => {}
    }

    if selected {
        painter.line_segment(
            [rect.left_top(), rect.right_top()],
            Stroke::new(2.0_f32, Color32::from_rgb(250, 205, 96)),
        );
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0_f32, Color32::from_rgb(250, 205, 96)),
            egui::StrokeKind::Inside,
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

fn draw_waveform(
    painter: &egui::Painter,
    rect: Rect,
    peaks: &[f32],
    source_frames: u64,
    source_start: f64,
    source_length: f64,
) {
    if peaks.is_empty() || rect.width() < 4.0 || source_frames == 0 {
        return;
    }
    let start_ratio = (source_start / source_frames as f64).clamp(0.0, 1.0);
    let length_ratio = (source_length / source_frames as f64).clamp(0.0, 1.0 - start_ratio);
    if length_ratio <= 0.0 {
        return;
    }
    let center = rect.center().y;
    let amplitude = (rect.height() * 0.5 - 6.0).max(1.0);
    let color = Color32::from_rgba_unmultiplied(12, 15, 20, 215);
    let mut x = rect.min.x + 1.0;
    while x < rect.max.x - 1.0 {
        let ratio = (x - rect.min.x) / rect.width();
        let peak_ratio = start_ratio + f64::from(ratio) * length_ratio;
        let index = ((peak_ratio * peaks.len() as f64) as usize).min(peaks.len() - 1);
        let height = (peaks[index] * amplitude).clamp(0.4, amplitude);
        painter.line_segment(
            [pos2(x, center - height), pos2(x, center + height)],
            Stroke::new(1.25_f32, color),
        );
        x += 2.0;
    }
}

fn track_rows(project: &Project, editor: &Editor) -> Vec<TrackRow> {
    project
        .ordered_track_ids()
        .into_iter()
        .filter_map(|id| {
            let node = project.nodes.get(&id)?;
            Some(TrackRow {
                id,
                name: node
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Track {}", id.raw())),
                color: track_color(node, id),
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
                            source_frames: match &clip.content {
                                ClipContent::Audio { asset, .. } => {
                                    editor.audio_asset(*asset).map(|audio| audio.frames as u64)
                                }
                                ClipContent::Midi { .. } | ClipContent::Empty => None,
                            },
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}

fn master_node(project: &Project) -> Option<NodeId> {
    project
        .nodes
        .iter()
        .find(|(_, node)| node.role == NodeRole::Master)
        .map(|(id, _)| *id)
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

fn asset_frames(editor: &Editor, clip: &Clip) -> Option<i64> {
    let ClipContent::Audio { asset, .. } = &clip.content else {
        return None;
    };
    editor.audio_asset(*asset).map(|audio| audio.frames as i64)
}

fn trim_audio_start(available: Option<i64>, original: &Clip, preview: &mut Clip, new_start: i64) {
    let ClipContent::Audio {
        source_start,
        source_length,
        ..
    } = &original.content
    else {
        return;
    };
    let delta = new_start - original.start.raw();
    let delta_source = (delta as f64 * source_length.raw() as f64
        / original.length.raw().max(1) as f64)
        .round() as i64;
    let source_end = source_start.raw() + source_length.raw();
    let mut start = source_start.raw() + delta_source;
    if let Some(frames) = available {
        start = start.clamp(0, frames.saturating_sub(1));
    } else {
        start = start.max(0);
    }
    let length = (source_end - start).max(1);
    if let ClipContent::Audio {
        source_start,
        source_length,
        ..
    } = &mut preview.content
    {
        *source_start = Frames::from_raw(start);
        *source_length = Frames::from_raw(length);
    }
}

fn trim_audio_end(available: Option<i64>, original: &Clip, preview: &mut Clip, new_length: i64) {
    let ClipContent::Audio {
        source_start,
        source_length,
        ..
    } = &original.content
    else {
        return;
    };
    let available = available.map(|frames| (frames - source_start.raw()).max(1));
    let delta = new_length - original.length.raw();
    let delta_source = (delta as f64 * source_length.raw() as f64
        / original.length.raw().max(1) as f64)
        .round() as i64;
    let length = (source_length.raw() + delta_source).max(1);
    let length = available.map_or(length, |available| length.min(available));
    if let ClipContent::Audio { source_length, .. } = &mut preview.content {
        *source_length = Frames::from_raw(length);
    }
}

fn tick_at(x: f32, origin: Pos2, pixels_per_beat: f32, snap: Snap) -> Tick {
    let raw = ((x - origin.x) / pixels_per_beat * PPQ as f32).round() as i64;
    Tick::from_raw(snap_tick(raw.max(0), snap))
}

fn clip_rect(clip: &Clip, origin: Pos2, row_y: f32, pixels_per_beat: f32) -> Rect {
    let x = origin.x + clip.start.raw().max(0) as f32 / PPQ as f32 * pixels_per_beat;
    let width = clip.length.raw().max(1) as f32 / PPQ as f32 * pixels_per_beat;
    Rect::from_min_size(pos2(x, row_y + 4.0), vec2(width.max(4.0), ROW_HEIGHT - 8.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use odaw_core::AssetId;

    fn audio_clip(start: i64, source_start: i64) -> Clip {
        Clip {
            node: NodeId::from_raw(1),
            start: Tick::from_raw(start),
            length: Tick::from_raw(4 * PPQ),
            content: ClipContent::Audio {
                asset: AssetId::from_bytes([1; 32]),
                source_start: Frames::from_raw(source_start),
                source_length: Frames::from_raw(4_800),
            },
        }
    }

    #[test]
    fn trimming_the_start_moves_the_source_window() {
        let original = audio_clip(PPQ, 4_800);
        let mut preview = original.clone();
        let start = original.start.raw() + PPQ;
        preview.start = Tick::from_raw(start);
        preview.length = Tick::from_raw(3 * PPQ);
        trim_audio_start(Some(20_000), &original, &mut preview, start);

        let ClipContent::Audio {
            source_start,
            source_length,
            ..
        } = preview.content
        else {
            panic!("expected audio");
        };
        assert_eq!(source_start.raw(), 6_000);
        assert_eq!(source_length.raw(), 3_600);
    }

    #[test]
    fn extending_the_start_reveals_earlier_source() {
        let original = audio_clip(PPQ, 4_800);
        let mut preview = original.clone();
        preview.start = Tick::ZERO;
        preview.length = Tick::from_raw(5 * PPQ);
        trim_audio_start(Some(20_000), &original, &mut preview, 0);

        let ClipContent::Audio {
            source_start,
            source_length,
            ..
        } = preview.content
        else {
            panic!("expected audio");
        };
        assert_eq!(source_start.raw(), 3_600);
        assert_eq!(source_length.raw(), 6_000);
    }

    #[test]
    fn trimming_and_extending_the_end_respects_the_asset() {
        let original = audio_clip(0, 0);
        let mut preview = original.clone();
        preview.length = Tick::from_raw(5 * PPQ);
        trim_audio_end(Some(7_000), &original, &mut preview, 5 * PPQ);

        let ClipContent::Audio { source_length, .. } = preview.content else {
            panic!("expected audio");
        };
        assert_eq!(source_length.raw(), 6_000);

        let mut preview = original.clone();
        preview.length = Tick::from_raw(8 * PPQ);
        trim_audio_end(Some(5_000), &original, &mut preview, 8 * PPQ);
        let ClipContent::Audio { source_length, .. } = preview.content else {
            panic!("expected audio");
        };
        assert_eq!(source_length.raw(), 5_000);
    }
}

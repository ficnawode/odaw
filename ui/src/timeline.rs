use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, pos2, vec2};
use odaw_core::{ClipContent, ClipId, NodeId, NodeRole, PPQ, Project, Tick};

use crate::editor::Editor;

const ROW_HEIGHT: f32 = 22.0;
const LABEL_WIDTH: f32 = 62.0;
const RULER_HEIGHT: f32 = 18.0;

struct ClipRow {
    id: ClipId,
    start: Tick,
    length: Tick,
    midi: bool,
}

struct TrackRow {
    id: NodeId,
    clips: Vec<ClipRow>,
}

pub fn show(ui: &mut egui::Ui, editor: &mut Editor) {
    let project = editor.session.project().clone();
    let tracks = track_rows(&project);
    let end_tick = odaw_render::project_end_tick(&project)
        .unwrap_or(Tick::from_raw(16 * PPQ))
        .max(Tick::from_raw(PPQ));
    let total_ticks = end_tick.raw().max(1);

    ui.horizontal(|ui| {
        ui.label("Timeline");
        ui.separator();
        ui.label(format!(
            "{:.2}s / {:.2}s",
            editor.position_seconds(),
            editor.duration_seconds()
        ));
    });

    let width = ui.available_width().max(300.0);
    let height = RULER_HEIGHT + tracks.len().max(1) as f32 * ROW_HEIGHT;
    let (response, painter) = ui.allocate_painter(vec2(width, height), Sense::click());
    let origin = response.rect.min;
    let background = Color32::from_rgb(24, 26, 30);
    painter.rect_filled(response.rect, 0.0, background);

    let beat_width = (width - LABEL_WIDTH) / (total_ticks as f32 / PPQ as f32);
    let total_beats = (total_ticks as f64 / PPQ as f64).ceil() as i64;
    for beat in 0..=total_beats {
        let x = origin.x + LABEL_WIDTH + beat as f32 * beat_width;
        if x > origin.x + width {
            break;
        }
        let bar = beat % 4 == 0;
        if bar {
            painter.line_segment(
                [pos2(x, origin.y + RULER_HEIGHT), pos2(x, origin.y + height)],
                Stroke::new(1.0_f32, Color32::from_rgb(58, 62, 72)),
            );
            painter.text(
                pos2(x + 2.0, origin.y + 2.0),
                Align2::LEFT_TOP,
                format!("{}", beat / 4 + 1),
                FontId::monospace(9.0),
                Color32::from_rgb(140, 146, 158),
            );
        }
    }

    if tracks.is_empty() {
        painter.text(
            pos2(origin.x + 8.0, origin.y + RULER_HEIGHT + 8.0),
            Align2::LEFT_TOP,
            "No tracks yet. Add a track from the left panel.",
            FontId::proportional(12.0),
            Color32::from_rgb(150, 156, 168),
        );
        return;
    }

    let selected_track = editor.selected_track();
    let selected_clip = editor.selected_clip();
    let mut hits: Vec<(Rect, NodeId, ClipId)> = Vec::new();

    for (index, track) in tracks.iter().enumerate() {
        let y = origin.y + RULER_HEIGHT + index as f32 * ROW_HEIGHT;
        let row = Rect::from_min_size(pos2(origin.x, y), vec2(width, ROW_HEIGHT));
        let row_color = if selected_track == Some(track.id) {
            Color32::from_rgb(36, 40, 48)
        } else if index % 2 == 0 {
            Color32::from_rgb(29, 31, 36)
        } else {
            Color32::from_rgb(26, 28, 33)
        };
        painter.rect_filled(row, 0.0, row_color);
        painter.text(
            pos2(origin.x + 6.0, row.center().y),
            Align2::LEFT_CENTER,
            format!("Track {}", track.id.raw()),
            FontId::monospace(10.0),
            if selected_track == Some(track.id) {
                Color32::from_rgb(235, 238, 245)
            } else {
                Color32::from_rgb(150, 156, 168)
            },
        );

        for clip in &track.clips {
            let x = origin.x
                + LABEL_WIDTH
                + (clip.start.raw().max(0) as f32 / total_ticks as f32) * (width - LABEL_WIDTH);
            let clip_width =
                (clip.length.raw().max(1) as f32 / total_ticks as f32) * (width - LABEL_WIDTH);
            let rect = Rect::from_min_size(
                pos2(x, y + 3.0),
                vec2(clip_width.max(4.0), ROW_HEIGHT - 6.0),
            );
            let color = if clip.midi {
                Color32::from_rgb(62, 126, 204)
            } else {
                Color32::from_rgb(90, 96, 108)
            };
            painter.rect_filled(rect, 3.0, color);
            if selected_clip == Some(clip.id) {
                painter.line_segment(
                    [rect.left_top(), rect.right_top()],
                    Stroke::new(2.0_f32, Color32::from_rgb(244, 196, 92)),
                );
            }
            painter.text(
                pos2(rect.min.x + 4.0, rect.center().y),
                Align2::LEFT_CENTER,
                format!("Clip {}", clip.id.raw()),
                FontId::monospace(8.0),
                Color32::from_rgb(235, 240, 248),
            );
            hits.push((rect, track.id, clip.id));
        }
    }

    let progress = editor.progress();
    let playhead_x = origin.x + LABEL_WIDTH + progress * (width - LABEL_WIDTH);
    painter.line_segment(
        [
            pos2(playhead_x, origin.y + RULER_HEIGHT - 4.0),
            pos2(playhead_x, origin.y + height),
        ],
        Stroke::new(1.5_f32, Color32::from_rgb(244, 92, 92)),
    );

    if ui.input(|input| input.pointer.primary_pressed()) && response.hovered() {
        if let Some(position) = ui.input(|input| input.pointer.hover_pos()) {
            if let Some(&(_, track, clip)) =
                hits.iter().find(|(rect, _, _)| rect.contains(position))
            {
                editor.select_clip(track, clip);
            }
        }
    }
}

fn track_rows(project: &Project) -> Vec<TrackRow> {
    project
        .nodes
        .iter()
        .filter(|(_, node)| node.role == NodeRole::Track)
        .map(|(id, node)| TrackRow {
            id: *id,
            clips: node
                .clips
                .iter()
                .filter_map(|clip_id| {
                    project.clips.get(clip_id).map(|clip| ClipRow {
                        id: *clip_id,
                        start: clip.start,
                        length: clip.length,
                        midi: matches!(&clip.content, ClipContent::Midi { .. }),
                    })
                })
                .collect(),
        })
        .collect()
}

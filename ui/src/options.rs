use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, pos2, vec2};
use odaw_core::{Clip, ClipContent, ClipId, NodeId, NodeRole, PPQ};
use odaw_dsp::AudioAsset;

use crate::editor::{Editor, TRACK_COLORS};
use crate::mixer::MixerState;

/// Bottom panel replacement for the piano roll: audio-clip detail plus the
/// selected track's mixer, colors, devices, and clip list.
pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    mixer: &mut MixerState,
    track: Option<NodeId>,
    clip: Option<(ClipId, Clip)>,
    rename_request: &mut Option<NodeId>,
) {
    egui::ScrollArea::vertical()
        .id_salt("bottom_options")
        .show(ui, |ui| {
            if let Some((clip_id, clip)) = clip.as_ref() {
                if matches!(clip.content, ClipContent::Audio { .. }) {
                    audio_clip(ui, editor, *clip_id, clip);
                    ui.separator();
                }
            }
            match track {
                Some(track) => track_options(ui, editor, mixer, track, rename_request),
                None => {
                    ui.add_space(12.0);
                    ui.label("Select a track to see its options.");
                }
            }
        });
}

fn audio_clip(ui: &mut egui::Ui, editor: &mut Editor, clip_id: ClipId, clip: &Clip) {
    let ClipContent::Audio {
        asset,
        source_start,
        source_length,
    } = &clip.content
    else {
        return;
    };
    let project = editor.session.project().clone();
    let name = project
        .assets
        .get(asset)
        .map(|asset| asset.name.clone())
        .unwrap_or_else(|| "Audio clip".to_owned());
    let peaks = editor.audio_peaks(*asset);
    let audio = editor.audio_asset(*asset);

    ui.horizontal(|ui| {
        ui.heading(name);
        ui.small("AUDIO");
        ui.small(format!("clip {}", clip_id.raw()));
    });
    ui.horizontal_wrapped(|ui| {
        if let Some(audio) = &audio {
            ui.small(format!(
                "{} Hz · {} ch · {:.2}s source",
                audio.sample_rate,
                audio.channels,
                audio.duration_seconds()
            ));
        }
        ui.small(format!(
            "timeline start {:.2} beats · length {:.2} beats",
            clip.start.raw() as f64 / PPQ as f64,
            clip.length.raw() as f64 / PPQ as f64
        ));
        ui.small(format!(
            "source offset {:.2}s",
            source_start.raw() as f64
                / audio.as_ref().map_or(48_000, |audio| audio.sample_rate) as f64
        ));
    });

    let (response, painter) =
        ui.allocate_painter(vec2(ui.available_width().max(240.0), 92.0), Sense::hover());
    draw_source_waveform(
        &painter,
        response.rect,
        peaks.as_deref(),
        audio.as_deref(),
        *source_start,
        *source_length,
        clip,
        editor.playhead_tick(),
    );

    ui.horizontal_wrapped(|ui| {
        let cursor = editor.cursor_tick();
        let inside = cursor.raw() > clip.start.raw()
            && cursor.raw() < clip.start.raw().saturating_add(clip.length.raw());
        if ui
            .add_enabled(inside, egui::Button::new("Split at cursor"))
            .clicked()
        {
            editor.split_clip(clip_id, cursor);
        }
        if ui.button("Duplicate clip").clicked() {
            editor.duplicate_clip(clip_id);
        }
        if ui.button("Delete clip").clicked() {
            editor.delete_clip(clip_id);
        }
        ui.separator();
        ui.small("Drag the clip edges in the timeline to trim · drop WAV/MP3 to import");
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_source_waveform(
    painter: &egui::Painter,
    rect: Rect,
    peaks: Option<&[f32]>,
    audio: Option<&AudioAsset>,
    source_start: odaw_core::Frames,
    source_length: odaw_core::Frames,
    clip: &Clip,
    playhead: odaw_core::Tick,
) {
    painter.rect_filled(rect, 4.0, Color32::from_rgb(18, 20, 24));
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0_f32, Color32::from_rgb(42, 46, 54)),
        egui::StrokeKind::Inside,
    );

    let Some(peaks) = peaks else {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "waveform unavailable",
            FontId::proportional(11.0),
            Color32::from_rgb(120, 126, 138),
        );
        return;
    };
    if peaks.is_empty() {
        return;
    }
    let frames = audio.map_or(1.0, |audio| audio.frames.max(1) as f64);
    let region_start = (source_start.raw().max(0) as f64 / frames).clamp(0.0, 1.0);
    let region_length = (source_length.raw().max(0) as f64 / frames).clamp(0.0, 1.0);
    let region_x0 = rect.min.x + rect.width() * region_start as f32;
    let region_x1 = (region_x0 + rect.width() * region_length as f32).min(rect.max.x);

    draw_peaks(
        painter,
        rect,
        peaks,
        Color32::from_rgba_unmultiplied(120, 128, 142, 90),
        0..peaks.len(),
    );
    let region = Rect::from_min_max(pos2(region_x0, rect.min.y), pos2(region_x1, rect.max.y));
    painter.rect_filled(
        region,
        0.0,
        Color32::from_rgba_unmultiplied(76, 139, 245, 26),
    );
    painter.rect_stroke(
        region,
        0.0,
        Stroke::new(1.0_f32, Color32::from_rgb(96, 158, 245)),
        egui::StrokeKind::Inside,
    );

    let region_peaks = ((region_length * peaks.len() as f64) as usize).max(1).min(
        peaks
            .len()
            .saturating_sub((region_start * peaks.len() as f64) as usize),
    );
    let region_start_index = ((region_start * peaks.len() as f64) as usize).min(peaks.len() - 1);
    let region_end_index = (region_start_index + region_peaks).min(peaks.len());
    draw_peaks_in(
        painter,
        region,
        &peaks[region_start_index..region_end_index],
        Color32::from_rgb(118, 176, 255),
    );

    if clip.length.raw() > 0 {
        let start = clip.start.raw();
        let end = start.saturating_add(clip.length.raw());
        if playhead.raw() >= start && playhead.raw() <= end {
            let ratio = (playhead.raw() - start) as f32 / clip.length.raw() as f32;
            let x = region_x0 + (region_x1 - region_x0) * ratio.clamp(0.0, 1.0);
            painter.line_segment(
                [pos2(x, rect.min.y), pos2(x, rect.max.y)],
                Stroke::new(1.5_f32, Color32::from_rgb(244, 92, 92)),
            );
        }
    }

    if let Some(audio) = audio {
        painter.text(
            pos2(rect.min.x + 6.0, rect.min.y + 5.0),
            Align2::LEFT_TOP,
            format!(
                "{:.2}s · {} Hz · {} ch",
                audio.duration_seconds(),
                audio.sample_rate,
                audio.channels
            ),
            FontId::monospace(9.0),
            Color32::from_rgb(154, 160, 172),
        );
    }
}

fn draw_peaks(
    painter: &egui::Painter,
    rect: Rect,
    peaks: &[f32],
    color: Color32,
    range: std::ops::Range<usize>,
) {
    let center = rect.center().y;
    let amplitude = (rect.height() * 0.5 - 8.0).max(1.0);
    let count = range.len().max(1);
    let mut x = rect.min.x + 1.0;
    while x < rect.max.x - 1.0 {
        let ratio = (x - rect.min.x) / rect.width();
        let index = range.start + ((ratio * count as f32) as usize).min(count - 1);
        let height = (peaks[index] * amplitude).clamp(0.4, amplitude);
        painter.line_segment(
            [pos2(x, center - height), pos2(x, center + height)],
            Stroke::new(1.0_f32, color),
        );
        x += 2.0;
    }
}

fn draw_peaks_in(painter: &egui::Painter, rect: Rect, peaks: &[f32], color: Color32) {
    if peaks.is_empty() || rect.width() < 2.0 {
        return;
    }
    let center = rect.center().y;
    let amplitude = (rect.height() * 0.5 - 8.0).max(1.0);
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

fn track_options(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    mixer: &mut MixerState,
    track: NodeId,
    rename_request: &mut Option<NodeId>,
) {
    let project = editor.session.project().clone();
    let Some(node) = project.nodes.get(&track).cloned() else {
        ui.label("The selected track no longer exists.");
        return;
    };
    let name = node
        .name
        .clone()
        .unwrap_or_else(|| format!("Track {}", track.raw()));
    let color = node.color.unwrap_or_else(|| {
        TRACK_COLORS[(track.raw().saturating_sub(1) % TRACK_COLORS.len() as u64) as usize]
    });

    ui.horizontal(|ui| {
        ui.colored_label(rgb(color), "■");
        ui.heading(name);
        if ui.button("Rename").clicked() {
            *rename_request = Some(track);
        }
        if ui
            .add(
                egui::Button::new(egui::RichText::new("M").color(Color32::from_rgb(255, 225, 225)))
                    .fill(if node.muted {
                        Color32::from_rgb(244, 58, 58)
                    } else {
                        Color32::from_rgb(190, 104, 104)
                    }),
            )
            .on_hover_text("Mute track")
            .clicked()
        {
            editor.set_track_muted(track, !node.muted);
        }
        if ui
            .add(
                egui::Button::new(egui::RichText::new("S").color(Color32::from_rgb(220, 235, 255)))
                    .fill(if node.soloed {
                        Color32::from_rgb(52, 150, 255)
                    } else {
                        Color32::from_rgb(86, 128, 190)
                    }),
            )
            .on_hover_text("Solo track")
            .clicked()
        {
            editor.set_track_soloed(track, !node.soloed);
        }
        if ui.button("Delete track").clicked() {
            editor.delete_track(track);
        }
    });

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label("Color");
        for value in TRACK_COLORS {
            let selected = node.color == Some(value);
            let button = egui::Button::new(if selected { "✓" } else { " " })
                .fill(rgb(value))
                .min_size(vec2(18.0, 18.0));
            if ui.add(button).clicked() {
                editor.set_track_color(track, Some(value));
            }
        }
        if ui.button("Default").clicked() {
            editor.set_track_color(track, None);
        }
    });

    ui.allocate_ui_with_layout(
        vec2(310.0, 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            mixer.gain(ui, editor, track, false);
            mixer.meter(ui, editor, track);
        },
    );
    ui.allocate_ui_with_layout(
        vec2(310.0, 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            mixer.pan(ui, editor, track, false);
        },
    );

    ui.separator();
    ui.horizontal(|ui| {
        ui.strong("Clips");
        if ui.button("+ MIDI clip").clicked() {
            editor.select_track(track);
            editor.add_midi_clip();
        }
        ui.small("drop WAV/MP3 on the timeline to add audio");
    });
    let mut clips: Vec<(ClipId, Clip)> = node
        .clips
        .iter()
        .filter_map(|id| project.clips.get(id).map(|clip| (*id, clip.clone())))
        .collect();
    clips.sort_by_key(|(_, clip)| clip.start.raw());
    for (clip_id, clip) in clips {
        let kind = match &clip.content {
            ClipContent::Midi { .. } => "MIDI",
            ClipContent::Audio { .. } => "AUDIO",
            ClipContent::Empty => "EMPTY",
        };
        let selected = editor.selected_clip_any().map(|(id, _)| id) == Some(clip_id);
        let label = format!(
            "{kind} · {:.2} beats · {:.2} beats long",
            clip.start.raw() as f64 / PPQ as f64,
            clip.length.raw() as f64 / PPQ as f64
        );
        if ui.selectable_label(selected, label).clicked() {
            editor.select_clip(track, clip_id);
        }
    }
    if node.clips.is_empty() {
        ui.small("No clips on this track.");
    }

    ui.separator();
    ui.strong("Devices");
    for device in &node.rack {
        let kind = project
            .nodes
            .get(device)
            .and_then(|node| node.kind.as_ref())
            .map_or("device", |kind| kind.as_str());
        ui.small(format!("• {kind}"));
    }
    if node.rack.is_empty() {
        ui.small("No devices (audio clips still play through the track strip).");
    }
    if node.role != NodeRole::Track {
        ui.small("This node is not a track.");
    }
}

fn rgb(value: u32) -> Color32 {
    Color32::from_rgb(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

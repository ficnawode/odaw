use std::path::Path;

use odaw_core::{
    AutomationLane, AutomationMode, ChannelLayout, Clip, ClipContent, ClipId, Curve, Edge, EdgeId,
    Keyframe, LoopRegion, Marker, MarkerId, Node, NodeId, NodeKind, NodePort, NodeRole, Note,
    NoteId, ParamId, ParamValue, Port, PortId, Project, Tick,
};
use odaw_engine::{DeviceContext, DeviceFactory};
use odaw_format::Container;
use odaw_vst3::{Catalog, PluginEntry, Vst3Backend};

const DEXED: &str = "ABCDEF019182FAEB4447534244657864";
const BARS: i64 = 16;
const BEATS_PER_BAR: i64 = 4;
const TOTAL_BEATS: i64 = BARS * BEATS_PER_BAR;
const EIGHTH: i64 = odaw_core::PPQ / 2;

const DEXED_CUTOFF: &str = "param.2029770765";
const DEXED_RESONANCE: &str = "param.13936686";
const DEXED_FEEDBACK: &str = "param.591125381";
const DEXED_OP1_OUTPUT: &str = "param.501240629";
const HALL_LATE_LEVEL: &str = "param.4";
const HALL_SIZE: &str = "param.5";
const PLATE_LATE_LEVEL: &str = "param.4";

#[derive(Clone, Copy)]
enum EffectChoice {
    Hall,
    Plate,
}

#[derive(Clone)]
struct TrackSpec {
    name: &'static str,
    color: u32,
    program: i32,
    gain_db: f64,
    pan: f64,
    reverb: Option<EffectChoice>,
    reverb_wet: f64,
    notes: Vec<NoteSpec>,
}

#[derive(Clone, Copy)]
struct NoteSpec {
    pitch: u8,
    start: i64,
    length: i64,
    velocity: u8,
}

pub fn create(path: &Path, force: bool) -> Result<(), String> {
    create_with(path, force, false)
}

pub fn create_automation(path: &Path, force: bool) -> Result<(), String> {
    create_with(path, force, true)
}

fn create_with(path: &Path, force: bool, automation: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!("{} already exists; pass --force", path.display()));
    }

    let catalog = Catalog::scan();
    let dexed = catalog
        .get(DEXED)
        .cloned()
        .ok_or_else(|| "Dexed is not installed in ~/.vst3 or /usr/lib/vst3".to_owned())?;
    let hall = find_effect(&catalog, "Hall")
        .cloned()
        .ok_or_else(|| "Dragonfly Hall Reverb is not installed".to_owned())?;
    let plate = find_effect(&catalog, "Plate")
        .cloned()
        .ok_or_else(|| "Dragonfly Plate Reverb is not installed".to_owned())?;
    let backend = Vst3Backend::new(catalog);

    let title = if automation {
        "Automation — Dexed Layers"
    } else {
        "Ode to Joy — Dexed Layers"
    };
    let mut project = Project::empty(title, 48_000, 1).map_err(|error| error.to_string())?;
    project.loops.push_back(LoopRegion {
        start: Tick::ZERO,
        end: Tick::from_raw(TOTAL_BEATS * odaw_core::PPQ),
    });
    project.markers.insert(
        MarkerId::from_raw(1),
        Marker {
            name: "Ode to Joy — theme".to_owned(),
            tick: Tick::ZERO,
        },
    );
    project.markers.insert(
        MarkerId::from_raw(2),
        Marker {
            name: "Theme repeat".to_owned(),
            tick: Tick::from_raw(32 * odaw_core::PPQ),
        },
    );

    let master = NodeId::from_raw(1);
    project.nodes.insert(
        master,
        Node::new(NodeRole::Master)
            .with_name("Master")
            .with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ])
            .with_param(ParamId::new("gain_db"), ParamValue::Float(3.0)),
    );

    let mut next_node = 2u64;
    let mut next_note = 1u64;
    let mut track_order = Vec::new();
    for (track_index, spec) in track_specs().into_iter().enumerate() {
        let effect = spec.reverb.map(|choice| match choice {
            EffectChoice::Hall => &hall,
            EffectChoice::Plate => &plate,
        });
        let track_id = NodeId::from_raw(next_node);
        track_order.push(track_id);
        let plugin_id = NodeId::from_raw(next_node + 1);
        next_node += 2;
        let effect_id = effect.map(|_| {
            let id = NodeId::from_raw(next_node);
            next_node += 1;
            id
        });
        let clip_id = ClipId::from_raw(track_index as u64 + 1);
        let edge_id = EdgeId::from_raw(track_index as u64 + 1);

        let context = DeviceContext::new(plugin_id, 48_000, 8192, 2, 120.0);
        let state = capture_program(&backend, &dexed, plugin_id, spec.program, &context)?;

        let mut rack = vec![plugin_id];
        if let Some(effect_id) = effect_id {
            rack.push(effect_id);
        }
        project.nodes.insert(
            track_id,
            Node::new(NodeRole::Track)
                .with_name(spec.name)
                .with_color(spec.color)
                .with_ports([
                    Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                    Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
                ])
                .with_rack(rack)
                .with_clips([clip_id])
                .with_param(ParamId::new("gain_db"), ParamValue::Float(spec.gain_db))
                .with_param(ParamId::new("pan"), ParamValue::Float(spec.pan)),
        );
        project.nodes.insert(
            plugin_id,
            Node::device(NodeKind::new(dexed.kind()))
                .with_name(format!("Dexed — {}", spec.name))
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)])
                .with_state(state),
        );
        if let (Some(effect_id), Some(effect)) = (effect_id, effect) {
            project.nodes.insert(
                effect_id,
                Node::device(NodeKind::new(effect.kind()))
                    .with_name(effect.name.clone())
                    .with_ports([
                        Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                        Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
                    ])
                    .with_param(
                        ParamId::new("param.2"),
                        ParamValue::Float(1.0 - spec.reverb_wet),
                    )
                    .with_param(ParamId::new("param.3"), ParamValue::Float(spec.reverb_wet)),
            );
        }
        project.clips.insert(
            clip_id,
            Clip {
                node: track_id,
                start: Tick::ZERO,
                length: Tick::from_raw(TOTAL_BEATS * odaw_core::PPQ),
                content: ClipContent::Midi {
                    source_start: Tick::ZERO,
                    source_length: Tick::from_raw(TOTAL_BEATS * odaw_core::PPQ),
                    notes: spec
                        .notes
                        .into_iter()
                        .map(|note| {
                            let id = NoteId::from_raw(next_note);
                            next_note += 1;
                            Note {
                                id,
                                pitch: note.pitch,
                                velocity: note.velocity,
                                channel: 0,
                                start: Tick::from_raw(note.start * EIGHTH),
                                length: Tick::from_raw(note.length * EIGHTH),
                            }
                        })
                        .collect(),
                },
            },
        );
        project.edges.insert(
            edge_id,
            Edge {
                from: NodePort {
                    node: track_id,
                    port: PortId::from_raw(1),
                },
                to: NodePort {
                    node: master,
                    port: PortId::from_raw(0),
                },
            },
        );
    }

    project.track_order = track_order.into_iter().collect();
    if automation {
        add_automation(&mut project);
    }
    Container::new(path)
        .save(&project)
        .map_err(|error| error.to_string())?;
    println!(
        "created {} ({} tracks, {} beats at 120 BPM{})",
        path.display(),
        project
            .nodes
            .values()
            .filter(|node| node.role == NodeRole::Track)
            .count(),
        TOTAL_BEATS,
        if automation {
            format!(", {} automation lanes", automation_lane_count(&project))
        } else {
            String::new()
        }
    );
    Ok(())
}

fn automation_lane_count(project: &Project) -> usize {
    project
        .nodes
        .values()
        .map(|node| node.automation.len())
        .sum()
}

fn add_automation(project: &mut Project) {
    let bars = |bar: i64| Tick::from_raw((bar - 1) * BEATS_PER_BAR * odaw_core::PPQ);

    if let Some(lead) = track_named(project, "Lead") {
        if let Some(dexed) = rack_device(project, lead, DEXED) {
            insert_lane(
                project,
                dexed,
                DEXED_CUTOFF,
                lane(
                    [
                        (bars(1), 0.34, Curve::Linear),
                        (bars(4), 0.58, Curve::Linear),
                        (bars(9), 0.84, Curve::Linear),
                        (bars(13), 0.48, Curve::Linear),
                        (bars(17), 0.62, Curve::Linear),
                    ],
                    AutomationMode::Absolute,
                ),
            );
            insert_lane(
                project,
                dexed,
                DEXED_RESONANCE,
                lane(
                    [
                        (bars(1), 0.18, Curve::Linear),
                        (bars(8), 0.44, Curve::Linear),
                        (bars(16), 0.72, Curve::Linear),
                    ],
                    AutomationMode::Absolute,
                ),
            );
            insert_lane(
                project,
                dexed,
                DEXED_FEEDBACK,
                lane(
                    [
                        (bars(1), 0.08, Curve::Hold),
                        (bars(5), 0.08, Curve::Linear),
                        (bars(9), 0.52, Curve::Hold),
                        (bars(13), 0.52, Curve::Linear),
                        (bars(17), 0.16, Curve::Linear),
                    ],
                    AutomationMode::Absolute,
                ),
            );
        }
        if let Some(hall) = rack_device(project, lead, "Hall") {
            insert_lane(
                project,
                hall,
                HALL_LATE_LEVEL,
                lane(
                    [
                        (bars(1), 0.16, Curve::Linear),
                        (bars(8), 0.22, Curve::Linear),
                        (bars(12), 0.58, Curve::Linear),
                        (bars(17), 0.30, Curve::Linear),
                    ],
                    AutomationMode::Absolute,
                ),
            );
            insert_lane(
                project,
                hall,
                HALL_SIZE,
                lane(
                    [
                        (bars(1), 0.32, Curve::Linear),
                        (bars(10), 0.86, Curve::Linear),
                        (bars(17), 0.44, Curve::Linear),
                    ],
                    AutomationMode::Absolute,
                ),
            );
        }
        insert_lane(
            project,
            lead,
            "pan",
            lane(
                [
                    (bars(1), -0.38, Curve::Linear),
                    (bars(5), 0.12, Curve::Linear),
                    (bars(9), 0.46, Curve::Linear),
                    (bars(13), -0.08, Curve::Linear),
                    (bars(17), -0.38, Curve::Linear),
                ],
                AutomationMode::Absolute,
            ),
        );
    }

    if let Some(pad) = track_named(project, "Pad") {
        if let Some(dexed) = rack_device(project, pad, DEXED) {
            insert_lane(
                project,
                dexed,
                DEXED_CUTOFF,
                lane(
                    [
                        (bars(1), 0.16, Curve::Linear),
                        (bars(9), 0.34, Curve::Linear),
                        (bars(17), 0.72, Curve::Linear),
                    ],
                    AutomationMode::Absolute,
                ),
            );
            insert_lane(
                project,
                dexed,
                DEXED_RESONANCE,
                lane(
                    [
                        (bars(1), 0.10, Curve::Linear),
                        (bars(17), 0.38, Curve::Linear),
                    ],
                    AutomationMode::Absolute,
                ),
            );
        }
        if let Some(plate) = rack_device(project, pad, "Plate") {
            insert_lane(
                project,
                plate,
                PLATE_LATE_LEVEL,
                lane(
                    [
                        (bars(1), 0.22, Curve::Linear),
                        (bars(6), 0.34, Curve::Linear),
                        (bars(12), 0.64, Curve::Linear),
                        (bars(17), 0.40, Curve::Linear),
                    ],
                    AutomationMode::Absolute,
                ),
            );
        }
    }

    if let Some(bass) = track_named(project, "Bass") {
        if let Some(dexed) = rack_device(project, bass, DEXED) {
            insert_lane(
                project,
                dexed,
                DEXED_OP1_OUTPUT,
                lane(
                    [
                        (bars(1), 0.58, Curve::Linear),
                        (bars(8), 0.72, Curve::Linear),
                        (bars(15), 0.90, Curve::Linear),
                        (bars(17), 0.70, Curve::Linear),
                    ],
                    AutomationMode::Absolute,
                ),
            );
        }
        insert_lane(
            project,
            bass,
            "gain_db",
            lane(
                [
                    (bars(1), 4.0, Curve::Linear),
                    (bars(9), 2.0, Curve::Linear),
                    (bars(17), 5.0, Curve::Linear),
                ],
                AutomationMode::Absolute,
            ),
        );
    }

    if let Some(bell) = track_named(project, "Bass Bell") {
        insert_lane(
            project,
            bell,
            "pan",
            lane(
                [
                    (bars(1), -0.55, Curve::Linear),
                    (bars(5), 0.55, Curve::Linear),
                    (bars(9), -0.55, Curve::Linear),
                    (bars(13), 0.55, Curve::Linear),
                    (bars(17), -0.35, Curve::Linear),
                ],
                AutomationMode::Absolute,
            ),
        );
    }

    if let Some(arp) = track_named(project, "Arp") {
        insert_lane(
            project,
            arp,
            "pan",
            lane(
                [
                    (bars(1), -0.62, Curve::Linear),
                    (bars(3), 0.62, Curve::Linear),
                    (bars(5), -0.62, Curve::Linear),
                    (bars(7), 0.62, Curve::Linear),
                    (bars(9), -0.62, Curve::Linear),
                    (bars(11), 0.62, Curve::Linear),
                    (bars(13), -0.62, Curve::Linear),
                    (bars(15), 0.62, Curve::Linear),
                    (bars(17), 0.0, Curve::Linear),
                ],
                AutomationMode::Absolute,
            ),
        );
    }

    let master = project
        .nodes
        .iter()
        .find(|(_, node)| node.role == NodeRole::Master)
        .map(|(id, _)| *id);
    if let Some(master) = master {
        insert_lane(
            project,
            master,
            "gain_db",
            lane(
                [
                    (bars(1), 3.0, Curve::Linear),
                    (bars(13), 3.0, Curve::Linear),
                    (bars(17), -4.0, Curve::Linear),
                ],
                AutomationMode::Absolute,
            ),
        );
    }
}

fn track_named(project: &Project, name: &str) -> Option<NodeId> {
    project
        .nodes
        .iter()
        .find(|(_, node)| node.role == NodeRole::Track && node.name.as_deref() == Some(name))
        .map(|(id, _)| *id)
}

fn rack_device(project: &Project, track: NodeId, fragment: &str) -> Option<NodeId> {
    let node = project.nodes.get(&track)?;
    node.rack.iter().copied().find(|device| {
        project.nodes.get(device).is_some_and(|node| {
            node.kind
                .as_ref()
                .is_some_and(|kind| kind.as_str().contains(fragment))
                || node
                    .name
                    .as_deref()
                    .is_some_and(|name| name.contains(fragment))
        })
    })
}

fn insert_lane(project: &mut Project, node: NodeId, param: &str, lane: AutomationLane) {
    if let Some(node) = project.nodes.get_mut(&node) {
        node.automation.insert(ParamId::new(param), lane);
    }
}

fn lane(
    points: impl IntoIterator<Item = (Tick, f64, Curve)>,
    mode: AutomationMode,
) -> AutomationLane {
    AutomationLane {
        mode,
        enabled: true,
        keyframes: points
            .into_iter()
            .map(|(tick, value, curve)| Keyframe {
                tick,
                value: ParamValue::Float(value),
                curve,
            })
            .collect(),
    }
}

fn find_effect<'a>(catalog: &'a Catalog, fragment: &str) -> Option<&'a PluginEntry> {
    catalog
        .entries
        .iter()
        .find(|entry| entry.is_effect() && entry.name.contains(fragment))
}

fn capture_program(
    backend: &Vst3Backend,
    dexed: &PluginEntry,
    plugin_id: NodeId,
    program: i32,
    context: &DeviceContext,
) -> Result<Vec<u8>, String> {
    let node = Node::device(NodeKind::new(dexed.kind()));
    let processor = backend
        .factory()
        .build(&node, context)
        .map_err(|error| error.to_string())?;
    {
        let handle = backend
            .plugin(plugin_id)
            .ok_or_else(|| "the Dexed instance is not live".to_owned())?;
        let mut plugin = handle.lock().map_err(|error| error.to_string())?;
        plugin
            .select_program(0, program)
            .map_err(|error| format!("could not select Dexed program {program}: {error}"))?;
    }
    drop(processor);
    backend
        .capture_states()
        .remove(&plugin_id)
        .ok_or_else(|| format!("Dexed did not return state for program {program}"))
}

fn track_specs() -> Vec<TrackSpec> {
    let melody = melody();
    let harmony = melody
        .iter()
        .map(|note| NoteSpec {
            pitch: diatonic_third_below(note.pitch),
            ..*note
        })
        .collect();
    let mut bass = Vec::new();
    let mut bell = Vec::new();
    let mut pad = Vec::new();
    let mut arp = Vec::new();
    for (chord_index, chord) in PROGRESSION.iter().cycle().take(8).enumerate() {
        let start = chord_index as i64 * 16;
        for (offset, length, interval, velocity) in BASS_PATTERN {
            bass.push(NoteSpec {
                pitch: chord.bass_pitch + interval as u8,
                start: start + offset,
                length,
                velocity,
            });
        }
        for (offset, length, interval, velocity) in BELL_PATTERN {
            bell.push(NoteSpec {
                pitch: chord.bass_pitch + interval as u8,
                start: start + offset,
                length,
                velocity,
            });
        }
        for pitch in chord.pad_pitches {
            pad.push(NoteSpec {
                pitch,
                start,
                length: 16,
                velocity: 70,
            });
        }
        for bar in 0..2 {
            for (step, tone) in ARP_PATTERN.iter().enumerate() {
                let Some(tone) = tone else {
                    continue;
                };
                arp.push(NoteSpec {
                    pitch: chord.arp_pitches[*tone],
                    start: start + bar * 8 + step as i64,
                    length: 1,
                    velocity: if step % 3 == 0 { 78 } else { 68 },
                });
            }
        }
    }

    vec![
        TrackSpec {
            name: "Lead",
            color: 0x4c8bf5,
            program: 3, // PHAROH
            gain_db: -5.0,
            pan: 0.0,
            reverb: Some(EffectChoice::Hall),
            reverb_wet: 0.28,
            notes: melody,
        },
        TrackSpec {
            name: "Harmony",
            color: 0x53b88b,
            program: 5, // Lavitar
            gain_db: -14.0,
            pan: -0.22,
            reverb: None,
            reverb_wet: 0.0,
            notes: harmony,
        },
        TrackSpec {
            name: "Bass",
            color: 0xe0a458,
            program: 21, // RUMBLE 1
            gain_db: 6.0,
            pan: 0.0,
            reverb: None,
            reverb_wet: 0.0,
            notes: bass,
        },
        TrackSpec {
            name: "Bass Bell",
            color: 0xc9a14c,
            program: 23, // C,D,Eb,F
            gain_db: -20.0,
            pan: -0.3,
            reverb: None,
            reverb_wet: 0.0,
            notes: bell,
        },
        TrackSpec {
            name: "Pad",
            color: 0x9a7bd8,
            program: 30, // Slow3D Pad
            gain_db: -16.0,
            pan: 0.16,
            reverb: Some(EffectChoice::Plate),
            reverb_wet: 0.3,
            notes: pad,
        },
        TrackSpec {
            name: "Arp",
            color: 0x5ab0c9,
            program: 15, // ARP 2600
            gain_db: -13.0,
            pan: 0.24,
            reverb: None,
            reverb_wet: 0.0,
            notes: arp,
        },
    ]
}

fn melody() -> Vec<NoteSpec> {
    let phrase = [
        (76, 0, 1, 100),
        (76, 1, 1, 96),
        (77, 2, 1, 100),
        (79, 3, 1, 104),
        (79, 4, 1, 100),
        (77, 5, 1, 96),
        (76, 6, 1, 100),
        (74, 7, 1, 92),
        (72, 8, 1, 96),
        (72, 9, 1, 92),
        (74, 10, 1, 96),
        (76, 11, 1, 100),
        (76, 12, 1, 100),
        (74, 13, 1, 92),
        (74, 14, 2, 88),
        (76, 16, 1, 100),
        (76, 17, 1, 96),
        (77, 18, 1, 100),
        (79, 19, 1, 104),
        (79, 20, 1, 100),
        (77, 21, 1, 96),
        (76, 22, 1, 100),
        (74, 23, 1, 92),
        (72, 24, 1, 96),
        (72, 25, 1, 92),
        (74, 26, 1, 96),
        (76, 27, 1, 100),
        (74, 28, 1, 92),
        (72, 29, 1, 88),
        (72, 30, 2, 84),
    ];
    let mut notes: Vec<NoteSpec> = phrase
        .iter()
        .map(|(pitch, start, length, velocity)| NoteSpec {
            pitch: *pitch as u8,
            start: *start * 2,
            length: *length * 2,
            velocity: *velocity as u8,
        })
        .collect();
    let repeat: Vec<NoteSpec> = phrase
        .iter()
        .map(|(pitch, start, length, velocity)| NoteSpec {
            pitch: *pitch as u8,
            start: *start * 2 + 64,
            length: *length * 2,
            velocity: *velocity as u8,
        })
        .collect();
    notes.extend(repeat);
    notes
}

fn diatonic_third_below(pitch: u8) -> u8 {
    let interval = match pitch % 12 {
        0 => 3,  // C -> A
        2 => 3,  // D -> B
        4 => 4,  // E -> C
        5 => 3,  // F -> D
        7 => 3,  // G -> E
        9 => 3,  // A -> F
        11 => 4, // B -> G
        _ => 3,
    };
    pitch.saturating_sub(interval)
}

struct Chord {
    bass_pitch: u8,
    pad_pitches: [u8; 3],
    arp_pitches: [u8; 4],
}

/// Syncopated bass hits in eighth notes: `(offset, length, semitone, velocity)`.
const BASS_PATTERN: [(i64, i64, i32, u8); 10] = [
    (0, 3, 0, 102),
    (3, 1, 12, 88),
    (5, 1, 7, 90),
    (7, 1, 0, 96),
    (8, 2, 0, 100),
    (10, 1, 12, 88),
    (11, 1, 7, 90),
    (13, 1, 0, 96),
    (14, 1, 12, 86),
    (15, 1, 7, 88),
];

/// Offbeat bell figures in eighth notes, relative to the chord root.
const BELL_PATTERN: [(i64, i64, i32, u8); 6] = [
    (2, 1, 24, 80),
    (4, 1, 12, 74),
    (7, 1, 19, 78),
    (10, 1, 24, 82),
    (12, 1, 19, 76),
    (15, 1, 12, 72),
];

const ARP_PATTERN: [Option<usize>; 8] = [
    Some(0),
    None,
    Some(2),
    Some(1),
    None,
    Some(3),
    Some(0),
    Some(2),
];

const PROGRESSION: [Chord; 4] = [
    Chord {
        bass_pitch: 36,
        pad_pitches: [60, 64, 67],
        arp_pitches: [60, 64, 67, 72],
    },
    Chord {
        bass_pitch: 43,
        pad_pitches: [55, 59, 62],
        arp_pitches: [55, 59, 62, 67],
    },
    Chord {
        bass_pitch: 45,
        pad_pitches: [57, 60, 64],
        arp_pitches: [57, 60, 64, 69],
    },
    Chord {
        bass_pitch: 41,
        pad_pitches: [53, 57, 60],
        arp_pitches: [53, 57, 60, 65],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_has_sixteen_bars_of_melody() {
        let notes = melody();
        assert_eq!(notes.iter().map(|note| note.start).max(), Some(124));
        assert_eq!(
            notes.iter().map(|note| note.start + note.length).max(),
            Some(128)
        );
        assert!(notes.iter().any(|note| note.start >= 64));
    }

    #[test]
    fn harmony_tracks_the_melody() {
        assert_eq!(diatonic_third_below(76), 72);
        assert_eq!(diatonic_third_below(79), 76);
        assert_eq!(diatonic_third_below(72), 69);
    }

    #[test]
    fn the_demo_has_six_layered_tracks() {
        let specs = track_specs();
        assert_eq!(
            specs.iter().map(|spec| spec.name).collect::<Vec<_>>(),
            vec!["Lead", "Harmony", "Bass", "Bass Bell", "Pad", "Arp"]
        );
        assert!(specs.iter().filter(|spec| spec.reverb.is_some()).count() >= 2);
        assert!(specs.iter().all(|spec| !spec.notes.is_empty()));
        let last_start = specs
            .iter()
            .flat_map(|spec| spec.notes.iter())
            .map(|note| note.start)
            .max();
        let last_end = specs
            .iter()
            .flat_map(|spec| spec.notes.iter())
            .map(|note| note.start + note.length)
            .max();
        assert!(last_start.is_some_and(|start| start >= 124));
        assert_eq!(last_end, Some(128));
    }
}

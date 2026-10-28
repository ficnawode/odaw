use std::path::Path;

use odaw_core::{
    ChannelLayout, Clip, ClipContent, ClipId, Edge, EdgeId, LoopRegion, Marker, MarkerId, Node,
    NodeId, NodeKind, NodePort, NodeRole, Note, NoteId, ParamId, ParamValue, Port, PortId, Project,
    Tick,
};
use odaw_engine::{DeviceContext, DeviceFactory};
use odaw_format::Container;
use odaw_vst3::{Catalog, PluginEntry, Vst3Backend};

const DEXED: &str = "ABCDEF019182FAEB4447534244657864";
const BARS: i64 = 16;
const BEATS_PER_BAR: i64 = 4;
const TOTAL_BEATS: i64 = BARS * BEATS_PER_BAR;

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

    let mut project = Project::empty("Ode to Joy — Dexed Layers", 48_000, 1)
        .map_err(|error| error.to_string())?;
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
            .with_param(ParamId::new("gain_db"), ParamValue::Float(0.0)),
    );

    let mut next_node = 2u64;
    let mut next_note = 1u64;
    for (track_index, spec) in track_specs().into_iter().enumerate() {
        let effect = spec.reverb.map(|choice| match choice {
            EffectChoice::Hall => &hall,
            EffectChoice::Plate => &plate,
        });
        let track_id = NodeId::from_raw(next_node);
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
                                start: Tick::from_raw(note.start * odaw_core::PPQ),
                                length: Tick::from_raw(note.length * odaw_core::PPQ),
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

    Container::new(path)
        .save(&project)
        .map_err(|error| error.to_string())?;
    println!(
        "created {} ({} tracks, {} beats at 120 BPM)",
        path.display(),
        project
            .nodes
            .values()
            .filter(|node| node.role == NodeRole::Track)
            .count(),
        TOTAL_BEATS
    );
    Ok(())
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
    let mut pad = Vec::new();
    let mut arp = Vec::new();
    for (chord_index, chord) in PROGRESSION.iter().cycle().take(8).enumerate() {
        let start = chord_index as i64 * 8;
        bass.push(NoteSpec {
            pitch: chord.bass_pitch,
            start,
            length: 8,
            velocity: 96,
        });
        for pitch in chord.pad_pitches {
            pad.push(NoteSpec {
                pitch,
                start,
                length: 8,
                velocity: 70,
            });
        }
        for bar in 0..2 {
            for (step, &tone) in ARP_PATTERN.iter().enumerate() {
                arp.push(NoteSpec {
                    pitch: chord.arp_pitches[tone],
                    start: start + bar * 4 + step as i64 / 2,
                    length: 1,
                    velocity: 72,
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
            gain_db: -10.0,
            pan: -0.22,
            reverb: Some(EffectChoice::Hall),
            reverb_wet: 0.24,
            notes: harmony,
        },
        TrackSpec {
            name: "Bass",
            color: 0xe0a458,
            program: 21, // RUMBLE 1
            gain_db: -3.0,
            pan: 0.0,
            reverb: None,
            reverb_wet: 0.0,
            notes: bass,
        },
        TrackSpec {
            name: "Pad",
            color: 0x9a7bd8,
            program: 30, // Slow3D Pad
            gain_db: -12.0,
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
            reverb: Some(EffectChoice::Plate),
            reverb_wet: 0.32,
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
            start: *start,
            length: *length,
            velocity: *velocity as u8,
        })
        .collect();
    let repeat: Vec<NoteSpec> = phrase
        .iter()
        .map(|(pitch, start, length, velocity)| NoteSpec {
            pitch: *pitch as u8,
            start: *start + 32,
            length: *length,
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

const ARP_PATTERN: [usize; 8] = [0, 1, 2, 3, 2, 1, 0, 1];

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
        assert_eq!(notes.iter().map(|note| note.start).max(), Some(62));
        assert_eq!(
            notes.iter().map(|note| note.start + note.length).max(),
            Some(64)
        );
        assert!(notes.iter().any(|note| note.start >= 32));
    }

    #[test]
    fn harmony_tracks_the_melody() {
        assert_eq!(diatonic_third_below(76), 72);
        assert_eq!(diatonic_third_below(79), 76);
        assert_eq!(diatonic_third_below(72), 69);
    }

    #[test]
    fn the_demo_has_five_layered_tracks() {
        let specs = track_specs();
        assert_eq!(
            specs.iter().map(|spec| spec.name).collect::<Vec<_>>(),
            vec!["Lead", "Harmony", "Bass", "Pad", "Arp"]
        );
        assert!(specs.iter().filter(|spec| spec.reverb.is_some()).count() >= 3);
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
        assert!(last_start.is_some_and(|start| start >= 62));
        assert_eq!(last_end, Some(64));
    }
}

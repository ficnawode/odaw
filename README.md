# odaw

A small, deterministic, scriptable music application. The engine is portable
and runs without a UI; the UI is one client among many.

The central model is:

```text
Project -> RenderPlan -> WAV (offline) / audio device (realtime)
```

`Project` is plain, versioned data. A compiler turns its signal graph into a
`RenderPlan`, and one engine renders that plan offline or in realtime.

## Status

This is the **first usable app shell**: the project compiles into a render plan,
schedules sample-accurate notes, renders deterministic audio offline, plays
through the sound card, and can be edited in a native track/piano-roll UI.

The UI is deliberately Ableton-like: tracks live in the timeline header, the
arrangement is the large central view, and the piano roll is a collapsible,
vertically resizable panel below it.

![odaw UI](docs/odaw-ui.png)

What works today:

- `odaw-core`: typed ids, integer `Tick`/`Frames` time with exact tempo-map
  conversion, graph/port/parameter/automation types, inline MIDI notes, the
  persistent `Project`, and structural validation.
- `odaw-control`: versioned commands, the `Tx` staging area, the single
  dispatcher, `ChangeSet` tracking, bounded `History` with undo/redo, operators
  and their registry, and a `Session` that owns the document, history, selection,
  and effects hook.
- `odaw-engine`: the immutable `RenderPlan`, a typed parameter buffer, a block
  executor, note events, a sample-accurate scheduler, and a lock-free realtime
  handoff.
- `odaw-dsp`: primitive in-tree devices (`builtin.sine`, `builtin.poly`,
  `builtin.gain`, `builtin.pass`) and a demo song.
- `odaw-compile`: rack flattening, deterministic DAG ordering, cycle detection,
  port/channel checks, and plan construction.
- `odaw-render`: deterministic offline rendering.
- `odaw-io`: a 16-bit PCM WAV writer.
- `odaw-platform`: `cpal` output with f32/i16/u16 conversion.
- `odaw-format`: versioned DTOs, postcard payloads, JSON manifests, and v1/v2 → v3
  migration. Track names, colors, mute, and solo are persisted.
- `odaw-ui` / `odaw-app`: Ableton-like timeline with track headers, colors,
  names, mute/solo, clip move/resize/split/duplicate, grid zoom and snapping,
  MIDI note and audio waveform previews, and a collapsible/resizable piano roll
  with drag-to-create, move, resize, right-click menus, snap, trackpad zoom,
  undo, and redo. Audio files can be dropped onto the timeline as clips with
  waveform previews. Edits go through `Session`; structural edits are recompiled
  and published to the realtime engine.
- `odaw`: a CLI for create, inspect, rename, demo, validate, render, and play.
- CI: formatting, clippy with warnings denied, and the workspace test suite.

Current limits: mono/stereo only, in-tree devices must live in a rack, devices
with latency are rejected until compensation lands, the output device must
support the project sample rate directly, imported audio clips are not played
yet, MIDI notes are not recorded from hardware, track grouping is not
implemented, there is no file dialog, no velocity lane, no loop/seek transport,
and no automation or plugin UI.

See [ROADMAP.md](ROADMAP.md) for the product path and
[.opencode/DESIGN.md](.opencode/DESIGN.md) for the architecture.

## Build and test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Try it

```sh
cargo run -p odaw-cli -- new song.odaw
cargo run -p odaw-cli -- inspect song.odaw
cargo run -p odaw-cli -- inspect song.odaw --json
cargo run -p odaw-cli -- rename song.odaw "New name"
cargo run -p odaw-cli -- demo demo.odaw
cargo run -p odaw-cli -- render demo.odaw -o demo.wav --duration 2
cargo run -p odaw-cli -- play demo.odaw
cargo run -p odaw-app -- demo.odaw
cargo run -p odaw-cli -- validate song.odaw
```

A container is a directory:

```text
song.odaw/
├── manifest.json
├── project.postcard
├── assets/          # reserved for content-addressed blobs
└── cache/           # disposable
```

The asset directory is created but asset blobs are not stored yet; content
addressing and the `AssetStore` arrive with the render/asset work.

## Layout

```text
core/             pure document types and validation
engine/           RenderPlan, parameter buffer, block executor
dsp/              primitive in-tree devices and the demo project
compile/          Project -> RenderPlan
io/               decoders and encoders
render/           offline renderer
platform/         cpal audio backend
format/           versioned DTOs, migrations, container I/O
control/          commands, staging, dispatch, history, operators, selection
ui/               egui application shell, timeline, piano roll
apps/odaw-cli/    headless CLI
apps/odaw-app/    native app
```

The remaining crates from `DESIGN.md` (`plugins`, `script`, `protocol`, ...) are
added when their phase starts, so the tree never carries empty placeholders.

## Design documents

- [.opencode/DESIGN.md](.opencode/DESIGN.md) — decisions and vocabulary.
- [.opencode/LTPLAN.md](.opencode/LTPLAN.md) — phase sequencing.
- [.opencode/CODING_GUIDELINES.md](.opencode/CODING_GUIDELINES.md) — how code
  is written.

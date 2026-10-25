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

This is the **Phase 2 first-sound milestone plus the inline MIDI note path**:
the project compiles into a render plan, schedules sample-accurate notes, and
renders deterministic audio offline to a WAV file. Realtime playback is next.

What works today:

- `odaw-core`: typed ids, integer `Tick`/`Frames` time with exact tempo-map
  conversion, graph/port/parameter/automation types, inline MIDI notes, the
  persistent `Project`, and structural validation.
- `odaw-control`: versioned commands, the `Tx` staging area, the single
  dispatcher, `ChangeSet` tracking, bounded `History` with undo/redo, operators
  and their registry, and a `Session` that owns the document, history, selection,
  and effects hook. Notes have add/remove/replace commands and participate in
  selection pruning.
- `odaw-engine`: the immutable `RenderPlan`, a typed parameter buffer, a block
  executor that sums inputs in declared order, note events, and a sample-accurate
  scheduler.
- `odaw-dsp`: primitive in-tree devices (`builtin.sine`, `builtin.poly`,
  `builtin.gain`, `builtin.pass`) and a demo song with a MIDI melody.
- `odaw-compile`: rack flattening, DAG topological sort, cycle detection,
  port/channel checks, and plan construction.
- `odaw-render`: deterministic offline rendering with fixed block size.
- `odaw-io`: a 16-bit PCM WAV writer.
- `odaw-format`: versioned DTOs separate from the live types, a postcard project
  payload, a JSON manifest, and format v1 → v2 migration. MIDI notes are part
  of the versioned project format.
- `odaw`: a CLI that creates, inspects, renames, demos, validates, and renders
  projects.
- CI: formatting, clippy with warnings denied, and the workspace test suite.

Current limits: mono/stereo only, in-tree devices must live in a rack, devices
with latency are rejected until compensation lands, MIDI notes are not recorded
from hardware yet, audio clips are not rendered yet, and realtime playback is not
wired.

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
format/           versioned DTOs, migrations, container I/O
control/          commands, staging, dispatch, history, operators, selection
apps/odaw-cli/    headless CLI
```

The remaining crates from `DESIGN.md` (`platform`, `plugins`, `ui`, ...) are
added when their phase starts, so the tree never carries empty placeholders.

## Design documents

- [.opencode/DESIGN.md](.opencode/DESIGN.md) — decisions and vocabulary.
- [.opencode/LTPLAN.md](.opencode/LTPLAN.md) — phase sequencing.
- [.opencode/CODING_GUIDELINES.md](.opencode/CODING_GUIDELINES.md) — how code
  is written.

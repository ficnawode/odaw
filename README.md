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

This is the **Phase 0 foundation**: the workspace, core data types, exact
musical time, the persistent project model, the versioned container format, and
CI. Nothing is audible yet.

What works today:

- `odaw-core`: typed ids, integer `Tick`/`Frames` time with exact tempo-map
  conversion, graph/port/parameter/automation types, persistent `Project`, and
  structural validation.
- `odaw-format`: versioned DTOs separate from the live types, a postcard project
  payload, a JSON manifest, and crash-conscious container save/load with
  interrupted-save recovery.
- `odaw`: a small CLI that creates, inspects, and validates project containers.
- CI: formatting, clippy with warnings denied, and the workspace test suite.

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
addressing and the `AssetStore` arrive with the control side.

## Layout

```text
core/             pure document types and validation
format/           versioned DTOs, migrations, container I/O
apps/odaw-cli/    headless CLI
```

The crates from `DESIGN.md` (`compile`, `engine`, `control`, `dsp`, `platform`,
`plugins`, `render`, `ui`, ...) are added when their phase starts, so the tree
never carries empty placeholders.

## Design documents

- [.opencode/DESIGN.md](.opencode/DESIGN.md) — decisions and vocabulary.
- [.opencode/LTPLAN.md](.opencode/LTPLAN.md) — phase sequencing.
- [.opencode/CODING_GUIDELINES.md](.opencode/CODING_GUIDELINES.md) — how code
  is written.

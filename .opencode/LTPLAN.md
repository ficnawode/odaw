# LTPLAN

Long-term implementation plan for `odaw`. `DESIGN.md` is the source of
truth for decisions; this file is only phase-level sequencing.

## Conventions

- A **phase** is a coherent chunk of work with a single goal. This file
  fixes the goals and their order. The work breakdown for a phase lives in
  its own plan file (for example `PHASE0.md`) written when the phase starts.
- Every commit must leave the tree green: it builds, `cargo test` passes,
  and whatever already ran still runs. Later commits may show less, never
  broken.
- Capability comes from three extension kinds: in-tree devices, plugin
  backends, and control/compile-time Lua (never on the audio thread). The
  core stays small. VST3 is the first plugin backend; CLAP can follow behind
  the same `DeviceFactory`/`DeviceProcessor` boundary.
- New capability starts as an extension. It only enters the core if the
  extension API itself needs it. Prefer composing primitives over adding a
  core type.
- The core ships only primitive devices; packs ship separately. Startup
  never blocks on plugin scanning or network.
- Phases 0–6 produce the first working product: launch the app, create or
  open a project, add tracks/devices, arrange sections and loops, hear it
  play, edit params and automation, save and reopen, and render offline —
  deterministically.
- Phases 7+ are features and are intentionally coarser. Work can be
  re-scoped between phases.
- Every phase that touches the render path must keep the `DESIGN.md`
  contracts: deterministic output for a snapshot, block-size independence,
  realtime cleanliness, and incremental compilation.

## Phase 0 — Foundation

Goal: the workspace, core data types, and persistence, with CI in place.
Nothing audible yet. Covers crates, ids, time, graph and schema types, the
persistent `Project`, versioned format DTOs, and the container.

## Phase 1 — Control side

Goal: the command stream — commands, staging, dispatch, history, undo, and
operators. Still nothing audible.

## Phase 2 — Compiler and offline render

Goal: `odaw render song.odaw -o out.wav` produces a correct, deterministic
WAV. Covers flattening, the topologically sorted plan, the executor, the
parameter buffer, the scheduler, and incremental recompile.

## Phase 3 — Realtime

Goal: hear the project through the sound card, safely. Covers the audio
backend, atomic plan handoff, the render-to-document feedback queue,
realtime-safety enforcement, and the master rate bridge.

## Phase 4 — Arrangement

Goal: reusable, hideable, loopable song structure on the timeline.

## Phase 5 — CLI product and robustness

Goal: a usable headless product — create, edit, inspect, import, save, and
render from the command line.

## Phase 6 — UI (first working product)

Goal: the native app — channels, timeline, piano roll, mixer, and
automation editing.

**Milestone: working product.**

---

## Backlog

Coarser phases. Each respects the conventions above and gets its own plan
file when work starts.

## Phase 7 — MIDI and automation depth

Goal: MIDI I/O, recording, modulators, and richer automation editing.

## Phase 8 — Plugin hosting: VST3 first, then CLAP

Goal: support external instruments, effects, state, and parameters on a
simple core. VST3 lands first as `odaw-vst3`; CLAP reuses the same device
boundary without forking the document or engine.

## Phase 9 — Lua scripting

Goal: control/compile-time Lua for building document data, operators, and
device specs, never on the audio thread.

## Phase 10 — Performance and parallel execution

Goal: scale to hundreds of tracks through profiling, a parallel executor,
and DSP optimization.

## Phase 11 — Remote, collaboration, release

Goal: remote clients, multi-client events, history checkpoints, and
packaging.

## Unassigned backlog

- Optional bundled pack (instruments/effects/presets), shipped apart from
  the core.
- Plugin scan cache with lazy/async scanning.
- Crash-safe autosave and recovery UI.
- Plugin sandboxing.
- Audio clip fades, gain, time-stretch/warp.
- Themes and layout persistence.
- File dialogs and recent projects.
- Metering/analysis beyond a peak cache.

## Ideas (exploratory, not committed)

These are directions, not requirements. They may never happen; do not let
them shape early decisions unless a decision would block them.

- **Scriptable instrument primitives (Max-for-Live-like).** Expose a set of
  core DSP primitives — oscillator, envelope, filter, amp, mixer, note
  source — so Lua can assemble full instruments by composing them. Blocked
  on: audio-rate modulation inputs, a composite/container node with exposed
  parameters, and a voice/polyphony model (note routing, per-voice state,
  glide, unison, MPE). Needs its own design pass before it enters a phase.
  Until then, polyphonic instruments come from a plugin backend.
- **AU adapter** behind the same `Device` abstraction on macOS.

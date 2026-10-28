# Roadmap to a usable DAW

`.opencode/LTPLAN.md` is the authoritative phase sequence; this file is the
product view of it, with acceptance criteria for the features the first release
must have. Each phase starts with its own work breakdown.

The order is deliberately bottom-up. A piano roll is cheap once notes, a
scheduler, and a sound source exist, and expensive before. Track selection is
part of the control surface, not the document. Automation and plugins both
depend on the render-plan and parameter contracts.

## Milestones

| Milestone | Phase | Outcome |
| --- | --- | --- |
| M0 Foundation | 0 | Green workspace, core data, versioned container. **Done.** |
| M1 Control core | 1 | Commands, transactions, undo, operators, selection. **Done.** |
| M2 First sound | 2-3 | Deterministic offline WAV and realtime playback. **Done.** |
| M3 Arrangement and headless product | 4-5 | Sections, loops, usable CLI. |
| M4 UI shell and track selection | 6 | Native window, transport, track headers, timeline. **Done.** |
| M5 Piano roll | 6-7 | MIDI editing and sample-accurate playback. **Done; depth work remains.** |
| M6 Automation | 7 | Draw, record, and render parameter automation. |
| M7 Plugin backends | 8 | VST3 first, then CLAP behind one device factory. |
| M8 VST3 compatibility | 8 | VST3 instruments, effects, state, parameters, and editors. **First pass done.** |
| M9 Release hardening | 9-11 | Scripting, performance, packaging. |

---

## M1 — Control core

Deliver `control` with the full mutation path from `DESIGN.md`:

- deterministic `IdGenerator`;
- `Command` and `CommandKind`, versioned;
- `Tx` staging with structural/value change tracking;
- `dispatch`, the only writer through `Tx`;
- composable, directionless `ChangeSet`;
- bounded `History` with a cursor and single-step undo/redo;
- `Operator`, `TargetRef`, operator registry, and readable history entries;
- `Session::apply` and `Session::apply_command`.

Add a non-document `Selection` (control or UI state):

```text
Selection {
    nodes: set<NodeId>,
    clips: set<ClipId>,
    notes: set<NoteId>,       // once notes exist
    range: option<TickRange>,
    focused_node: option<NodeId>,
}
```

Selection is not serialized in the project and is not an undo step. Commands
always receive concrete ids; the front-end resolves the current selection at
invocation. Keep "focused track" separate from "selected tracks".

Acceptance:

- every edit is a command, and replaying the command log reproduces the project;
- undo and redo restore both the document and the render-side effects;
- a failed command leaves no partial edit;
- selection survives undo if its ids still exist, and drops stale ids.

Implemented in this commit. Remaining follow-up: the versioned command-log DTOs
and the on-disk operation log. Commands are versioned data today; their log
encoding is not written yet.

## M2 — First sound

Add `engine`, `compile`, `dsp`, and `render`.

- Flatten racks and edges into a topologically sorted `RenderPlan`.
- Insert latency compensation on parallel paths.
- Preallocate buffers; make the executor allocation-free.
- Add a stable `ParamAddress` slot map and typed parameter buffer.
- Add a minimal built-in signal path: test instrument -> gain -> master.
- Render offline to WAV with a fixed block size.
- Add golden-WAV tests and block-size-independence tests.
- Add `cpal` realtime playback with atomic plan handoff.

Acceptance:

- `odaw render song.odaw -o out.wav` is byte-identical across runs for the same
  project, platform, and block size;
- core devices render identically across block sizes;
- the audio callback does not allocate, lock, do I/O, or parse;
- the app plays the same project through the sound card.

Implemented in this commit: rack flattening, deterministic DAG ordering, cycle
and channel checks, the parameter buffer, the block executor, primitive
sine/poly/gain/pass devices, inline MIDI notes, the note scheduler, deterministic
offline WAV rendering, `cpal` realtime playback, lock-free engine handoff,
block-size-independence tests, and `odaw demo` / `odaw render` / `odaw play`.

Remaining in M2:

- latency compensation and delay nodes (non-zero latency is rejected for now);
- devices used outside a rack or in parallel/sidechain device paths;
- the master sample-rate bridge for devices that cannot run at the project rate;
- incremental recompile and parameter updates on a live engine;
- a general feedback queue for meters/transport beyond position and finished;
- golden WAV fixtures in CI.

## M3 — Arrangement and headless product

- Sections, arrangement refs, inactive sections, and loops.
- Flatten arrangement for render; keep the document reusable.
- CLI: create tracks, add primitive devices, set parameters, import audio,
  create sections, inspect, render, save.

Acceptance: a complete song can be built, saved, reopened, and rendered from the
command line without a UI.

## M4 — UI shell and track selection

Use `egui` + `winit` native. The UI holds no document state.

- Transport, playhead, tempo display.
- Track list with add/rename/delete/reorder, mute/solo, arm.
- Track selection, multi-select, focus, keyboard navigation.
- Timeline with clips, zoom, snap, and horizontal/vertical scroll.
- Inspector for the selected node and its parameters.
- Mixer strips.

Acceptance: select one or more tracks, edit them through operators, undo, and see
the UI and audio follow. Selection survives a reload only as an in-memory
workspace preference, never in the project document.

Implemented: native window, transport with editable tempo and bar/beat display,
Ableton-like timeline with track headers inside the arrangement, track colors,
rename, mute/solo, add/delete track, add MIDI clip, clip move/resize/split/
duplicate, marquee multi-selection, zoom-following snap divisions from 1/1 to
1/128 with triplets and faint guides, zoom-to-fit plus Ctrl+wheel/pinch,
C0–C7 piano-roll range, key reorder of tracks and notes, row-aligned and
resizable track headers, MIDI and audio previews,
drag/drop WAV/MP3 import with waveform previews and realtime audio-clip
playback through the track strip, a contextual bottom panel (piano roll for
MIDI, audio clip waveform/details, and track options), a Time dialog for
bar/beat position, tempo, and time signature, click/drag ruler seeking,
split-at-cursor, per-track gain/pan faders with post-fader stereo level meters,
a single stereo master out strip in the timeline, inspector, and realtime
publish on structural edits. Scrubbing uses the realtime seek command, fader
gestures commit one value-only command on release, and meters travel from the
audio thread to the UI through atomics. Saved projects copy their audio blobs
into the container so waveforms and playback survive a reload.

Remaining: drag-to-reorder tracks, grouping/folders, arm, track multi-select,
parameter automation UI, file dialogs, loop transport, audio clip fades, and
time-stretch/warping.

## M5 — Piano roll

The data decision is made: MIDI notes live inline in the clip, not only in an
immutable MIDI asset. Inline notes are what make per-note editing, undo, and
copy/paste tractable. Import/export remains a separate path through `AssetId`.

Implemented: `Note` / `NoteId`, note add/remove/replace commands, note
selection, the sample-accurate `Scheduler`, the `builtin.poly` instrument, and a
collapsible/resizable piano roll with grid, snap, zoom, tap-to-create,
drag-to-marquee, group move, group transpose, copy/paste, duplicate, resize,
right-click menus, per-note velocity/length/transpose actions, undo, and redo.

Remaining:

- velocity lane;
- quantize and note preview;
- playhead follow while playing;
- MIDI recording and quantization depth in M7.

## M6 — Automation

The document already stores lanes and keyframes.

- Lane view per parameter, with absolute and trim modes.
- Draw, move, delete, and curve-edit keyframes; snap and zoom.
- Automation recording that thins incoming values into keyframes as one
  transaction.
- Compiler folds lanes into the DAG; executor evaluates at block boundaries
  (or per sample for audio-rate parameters).
- Smoothing policy decision from `DESIGN.md` open item 2.
- Unmatched lanes are dropped with an event when a device kind changes.

Acceptance: automate gain and a filter cutoff, hear it, undo a recording pass in
one step, save/reopen, and render deterministically.

## M7 — Plugin backends

VST3 landed first (see M8) because it was the requested backend. CLAP can
follow behind the same `DeviceFactory` boundary and must not fork the device
interface.

- Lazy, cached, deferrable plugin scan.
- Load/prepare off the audio thread; hand off live instances atomically.
- Map parameters and state to `NodeKind`, `ParamSchema`, and `Node.state`.
- Sample-accurate parameter changes and note events.
- Latency reporting and compensation.
- Floating plugin windows first; embedding later.
- Process isolation is an implementation of the same `DeviceProcessor` boundary.

Acceptance: load a CLAP instrument and effect, play MIDI, automate a parameter,
save/reopen the patch state, and render offline.

## M8 — VST3 compatibility

VST3 is a compatibility adapter, not a second core model. It uses the same
`DeviceProcessor` and parameter buffer as CLAP.

Two constraints shape the plan:

- **Licensing.** The official VST3 SDK is dual-licensed GPLv3 or Steinberg
  proprietary. Do not link it into a permissively licensed core by accident. A
  clean-room implementation of the public VST3 interfaces, or a separately
  distributed host process, keeps the license boundary explicit. Get legal
  review before shipping.
- **Windows/macOS/Linux bundles.** `.vst3` is a bundle with a component and an
  edit controller; scanning, class ids, and state are host responsibilities.

Technical work:

- COM-style interfaces: `IComponent`, `IEditController`, `IParameterChanges`,
  `IEventList`, `IComponentHandler`, `IPlugView`.
- Parameter normalization, state save/restore, latency, note events.
- Scan cache keyed by bundle path plus mtime/hash.
- Audio-thread rules identical to CLAP.
- Floating plugin windows first; Wayland embedding stays out of scope.

Acceptance: the CLAP acceptance test passes with a VST3 plugin, including state
restore and sample-accurate automation. If the plugin pool demands it, VST3 can
move ahead of CLAP, but the device interface must not fork.

Implemented in this commit: `odaw-vst3` (MIT `vst3-host` behind
`DeviceFactory`), filesystem scan with `moduleinfo.json` fast path and
`~/.cache/odaw/vst3-plugins.json`, `odaw plugins`, normalized `param.<id>`
parameters with change-tracked parameter-buffer updates, component/controller
state capture and restore, note events, latency reporting plus per-edge PDC,
missing-plugin pass-through placeholders, a generic parameter panel, a
background plugin browser, instrument/effect insertion, and floating editor
windows verified under XWayland.

Rack effects stack in serial order and can be reordered/removed in the UI.
Plugin state and current normalized parameters are captured into `Node.state`
and `Node.params` on save, editor close, and explicit "Save state"; undo/redo
restores the document snapshot rather than the live patch.

Remaining: instance retention across structural edits (currently state is
re-instantiated), sample-accurate automation, multi-out/sidechain buses,
process isolation, Windows/macOS bundles, a bundled MIT test effect for CI, and
`.vstpreset` import/export.

## M9 — Release hardening

- Lua control/compile-time scripting.
- Parallel execution for large sessions.
- Autosave, crash recovery, recent projects, file dialogs.
- Plugin scan cache UX and sandboxing.
- Audio clip fades, gain, time-stretch, and warping.
- Packaging, installer, documentation, and a stable project-format policy.
- Choose and add the project license.
- Fuzz the container parser; add golden renders to CI.
- Durable container save: directory fsync on Unix, platform-specific atomic
  directory replacement, and a save crash test.

## Immediate next steps after this commit

1. Commit this tree as the audio-clip playback commit.
2. Add loop playback and a loop region editor.
3. Add audio clip fades, gain, time-stretch, and warping.
4. Add a velocity lane, quantize, and playhead follow to the piano roll.
5. Add track grouping/folders and drag-to-reorder.
6. Add automation editing.
7. Add CLAP behind the same device factory, and instance retention for plugins.
8. Add latency compensation, the master rate bridge, and support for
   parallel/sidechain device paths.

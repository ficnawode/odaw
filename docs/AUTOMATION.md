# Automation

Automation is the first feature that touches every layer of odaw: the document,
the command stream, the compiler, the realtime engine, offline rendering, and
the editor. The data model and format already contain most of the shape, so the
work is mostly semantics, evaluation, and UI.

This document describes the implemented design and the remaining depth work. The
parts that are expensive to reverse are called out at the end.

## Status

Implemented: lane enablement and format v5 migration, the pure evaluator and
validation, the non-structural automation change channel, `AutomationPlan`
compilation and live swap, base/effective `ParamBuffer` values with ramps and
touch overrides, sample-accurate gain/pan automation, block-size independent
offline rendering, the Notes/Automation/Clip bottom panel, lane editing,
Absolute/Trim, right-click discovery, expandable arrangement lanes, parameter
descriptors, and touch recording.

Still open: additional curve shapes, write/latch recording, punch ranges,
sample-accurate VST3 parameter changes, copy/paste across different parameters,
and dropping unmatched lanes on a device-kind change. The CLI exposes
`automation list/set/clear`.

## Demo

`examples/automation-demo.odaw` is a playable demonstration built from the
Dexed Ode to Joy arrangement. It has 14 lanes across plugin and host
parameters:

- Dexed lead: cutoff, resonance, feedback;
- Dexed pad: cutoff, resonance;
- Dexed bass: operator 1 output level;
- Dragonfly Hall: late level, size;
- Dragonfly Plate: late level;
- track pan on lead, bass bell, and arp;
- bass gain;
- master gain fade-out.

Run it with:

```sh
cargo run -p odaw-cli -- demo-automation automation.odaw
cargo run -p odaw-cli -- render automation.odaw -o automation.wav
cargo run -p odaw-app -- automation.odaw
```

`demo-automation` requires the same installed Dexed and Dragonfly Hall/Plate
plugins as `demo-dexed`.

---

## 1. What already exists

- `core::AutomationLane { mode, keyframes }`, `Keyframe { tick, value, curve }`,
  `Curve::{Hold, Linear}`, and `AutomationMode::{Absolute, Trim}` live in
  `core/src/graph.rs`.
- A lane is stored on the node that owns the parameter:
  `Node.automation: ParamMap<ParamId, AutomationLane>`. Its target is implicit,
  so it cannot dangle and deleting the node deletes its lanes. This is the right
  ownership rule; do not introduce a global lane map.
- `AutomationCommand::Set { node, param, lane }` exists, and `Tx` has
  `set_automation`.
- The format DTOs and v2/v3/v4 migrations already round-trip lanes.
- `DeviceParameter` already carries `label`, `min`, `max`, `step_count`, `unit`,
  and `read_only`, and VST3 reports it at load time.

## 2. What is missing

This is the actual gap, and it explains the ordering below:

1. **Semantics.** There is no pure evaluator and no definition for values before
   the first keyframe, after the last, discrete parameters, trim mode, boundary
   rules, or duplicate keyframes.
2. **Validation.** `core::validation` says nothing about lanes.
3. **Change tracking.** `Tx::set_automation` marks the change structural. That
   is wrong: every point drag would rebuild the graph and, today, re-instantiate
   every plugin in the project. Automation edits must be a separate, cheap
   channel.
4. **Render data.** The compiler ignores `Node.automation` and builds a
   `ParamBuffer` from base values only. `ParamBuffer` has no notion of a base
   value versus an automated value, and no ramps.
5. **Engine.** Nothing evaluates automation on the audio or offline path.
6. **UI.** There is no lane list, no editor, and the bottom panel is a single
   `piano_roll_open` / `track_options_open` boolean rather than a view switch.

The good news: automation can be added without changing the signal graph. Lanes
are data on existing nodes, and a lane targets an existing `ParamAddress`.

---

## 3. Product shape

Use two complementary views, both reaching the same lane data:

1. **A tab in the bottom panel**, beside Notes and Clip/Track. This is the
   detailed editor: parameter list on the left, large curve canvas on the right.
2. **Expandable lane rows under a track in the arrangement.** This is the
   overview: see several parameters at once against the song, and make coarse
   edits without leaving the timeline.

The bottom panel is the primary editor; arrangement lanes are the overview.
That matches how the existing timeline and piano roll divide responsibility, and
it keeps the piano roll available.

```text
┌──────────────────────────────────────────────────────────────────────┐
│ odaw   Play  Start  1.1  4/4  Time  Tempo 120.0 BPM   audio ready     │
├──────────────────────────────────────────────────────────────────────┤
│ ... timeline ...                                                     │
│  Track 3  [A]  Volume  ────────╲───────╱────  -6.0 dB                │
│                 Pan     ────────────────     C                        │
│  Track 4  [A]                                                        │
├──────────────────────────────────────────────────────────────────────┤
│ [Notes] [Automation] [Clip/Track]                         Hide panel  │
│ Target: Track 3 / Dexed / Cutoff   [Absolute ▾] [Enabled] [Arm ●]    │
│ ┌──────────────┬───────────────────────────────────────────────────┐ │
│ │ Track 3      │ 1.1        1.2        1.3        1.4        2.1  │ │
│ │  • Volume  A │ ─────────●────────────────●─────────────●─────── │ │
│ │  • Pan       │                 ╱─────────────────────────          │ │
│ │ ▾ Dexed      │                                              0.72 │ │
│ │  • Cutoff  A │                                              0.31 │ │
│ │  • Resonance │                                              0.00 │ │
│ └──────────────┴───────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────┘
```

### 3.1 Bottom panel tabs

Replace the current `piano_roll_open` / `track_options_open` booleans with:

```rust
enum BottomTab {
    Notes,      // piano roll; disabled when the selection is not MIDI
    Automation, // parameter list + large lane editor
    Clip,       // existing audio detail + track options
}
```

- `Notes` keeps the existing behavior and remains the default for a MIDI clip.
- `Clip` is the existing track options plus audio clip detail.
- `Automation` is always available when a track is selected.
- The previous tab is remembered. Selecting a MIDI clip does not steal focus
  from Automation if the user is working there; it only does so when the current
  tab is not valid for the new selection.
- The lane list is scoped to the focused track (its channel strip, its rack
  devices, and the master when the master is focused). A search field handles
  plugins with hundreds of parameters.

### 3.2 Arrangement lanes

- Each track header gets a compact **A** toggle. Tracks with automation show a
  small dot on the toggle even when collapsed.
- Expanding a track adds one row per visible lane beneath its clip row. Track
  rows become variable height; the header column and grid column must share the
  same row offsets.
- A lane row has a compact header (param name, mode, enable, arm, value at the
  playhead) and a curve strip aligned to the timeline.
- Limit the initial visible stack to a handful of lanes; the rest are a
  scrollable/expandable count. Vertical scrolling already exists.
- Lane expansion and per-lane visibility are **view state**, not document
  state. Store them in `TimelineState`, not in the project.

### 3.3 Discovery from parameter controls

This is the detail that makes automation feel native rather than bolted on.
Every parameter control (mixer gain/pan, device parameter slider, track
inspector) gets:

- a small **A** badge when the parameter has a lane;
- a right-click menu with **Show automation**, **Add automation lane**,
  **Clear automation**, and later **Arm for recording**;
- consistent value formatting from the same parameter descriptor the automation
  editor uses.

`Show automation` selects the parameter and switches the bottom panel to the
Automation tab. `Add automation lane` creates it at the current cursor with the
current value as its first point.

---

## 4. Document semantics

### 4.1 Lane shape

Keep `Node.automation` keyed by `ParamId`. Add one field and keep the rest
stable:

```rust
pub struct AutomationLane {
    pub mode: AutomationMode,   // Absolute or Trim
    pub enabled: bool,          // new; persisted
    pub keyframes: PersistentVector<Keyframe>,
}
```

Why `enabled`: an automation lane is a mix decision. Turning it off must survive
save/reopen and must be undoable. A UI-only mute would make render and playback
disagree after a reload.

`enabled` is a small additive format change. Bump `FORMAT_VERSION` to 5, add a
v4 DTO snapshot, and migrate old lanes with `enabled = true`. This is cheaper
and safer than adding fields to postcard without a version bump.

Do **not** add a `KeyframeId`. A keyframe is only ever addressed within its own
lane; the UI can select by index, and every edit can replace the whole lane as
one command. Ids would add format churn, migration work, and selection
bookkeeping for no functional gain. If collaborative editing ever needs stable
per-point identity, that is a later opt-in.

Do not add a lane color. Use the owning track's color with a brighter/darker
variant. It keeps the document smaller and the arrangement visually coherent.

### 4.2 Value rules

A lane evaluates to a `ParamValue` of the same variant as its points.

- **Before the first keyframe:** hold the first value.
- **After the last keyframe:** hold the last value.
- **On a keyframe:** exactly that value, regardless of the previous segment's
  curve.
- **`Hold`:** the value holds until the next keyframe, then steps.
- **`Linear`:** linearly interpolate between the two values.
- **Float:** interpolates.
- **Int:** interpolates then rounds to the nearest integer. Stored points may
  be integers; the rounded result is what the device sees.
- **Bool / Enum:** interpolation is meaningless. These use `Hold` regardless of
  the stored curve; the value steps at the keyframe. The UI hides the curve
  choice for them.
- **Empty lane:** no effect. The base value is used.

`Keyframe.curve` describes the segment that starts at that keyframe and ends at
the next one. The last keyframe's curve is unused.

### 4.3 Absolute and trim

The parameter value is assembled in a fixed order:

```text
base value
  -> automation (absolute, or trim offset)
  -> modulation offsets (later)
  -> clamp
```

- **Absolute:** the lane replaces the base value for playback. The base value is
  still stored and is what the fader edits when automation is not writing.
- **Trim:** the lane value is added to the base value. On a Float lane that
  means `base + offset`; on an Int lane it means `round(base + offset)`. Trim is
  disabled for Bool and Enum.
- Trim values are stored in the parameter's native units (dB for gain, -1..1 for
  pan, 0..1 for normalized plugin parameters). The UI draws a zero line.
- Switching modes converts the existing points so the curve does not jump:
  Absolute -> Trim subtracts the current base value, Trim -> Absolute adds it.
  For Float and Int this is exact; for other types the switch is disallowed.

This resolves the requirement in `DESIGN.md` §4.3 that a bulk change still works
when a parameter already has automation.

### 4.4 Keyframe ordering and duplicates

The dispatcher normalizes every lane before storing it:

1. drop non-finite Float values (reject the command with a diagnostic instead of
   silently writing a NaN);
2. sort by tick;
3. for equal ticks, keep the last one in the input order;
4. clamp ticks to `>= 0`.

The UI should never create duplicates, but paste and recording can. Last-wins is
deterministic and avoids a validation failure during a normal editing gesture.

### 4.5 Validation

Extend `core::validation`:

- keyframes are strictly increasing by tick after normalization;
- Float values are finite;
- `Trim` is not used on Bool or Enum lanes;
- at most one lane per `(node, param)` — already guaranteed by the map.

The last rule is structural, not a scan: the map enforces it.

### 4.6 Undo and replay

`AutomationCommand::Set` replaces the whole lane, so every point edit is one
ordinary command and one history entry. A drag previews locally and commits once
on release, exactly like notes and clips.

Recording is a whole-lane replacement (or a range merge) produced by one
command, so a recorded pass is one undo step.

---

## 5. Render-side design

### 5.1 Automation is not structural

Add a separate change channel:

```rust
pub struct ChangeSet {
    // existing fields...
    pub automation: BTreeSet<ParamAddress>,
}

impl ChangeSet {
    pub fn is_automation_only(&self) -> bool { /* no structural, no layout,
        no meta/tempo, no arrangement, only automation */ }
}
```

`Tx::set_automation` marks `automation` and **not** `structural`. The node is
still touched for UI invalidation if desired, but no graph rebuild follows.

Why this matters: a structural rebuild currently constructs new device
processors. Dragging an automation point must not reload a VST3 instrument,
reset its voices, or drop its tails. Automation updates take a dedicated path.

### 5.2 ParamBuffer: base versus effective

`ParamBuffer` needs three concepts per stable slot:

```rust
struct Slot {
    base: ParamValue,      // written by the document / a fader
    effective: ParamValue, // what devices read
    ramp: Option<(f64, f64)>, // optional linear ramp for this block
    version: u64,
}
```

- `ensure` initializes both base and effective.
- `set` writes base and effective (this is what a fader or a value command does).
- `set_effective` writes only the automated value.
- `set_ramp` stores start and end and bumps the version.
- `get`/`float` return the effective value.
- `base` returns the base value; trim evaluation needs it.
- `float_at(address, sample, frames)` returns the interpolated effective value;
  devices that care about smoothness use it, devices that do not keep using
  `float`.

The version moves only when the effective value or ramp changes, so VST3 still
sees one change per block instead of every sample.

This also solves the live-fader conflict. Moving a fader writes the base and the
current effective value. On the next block, trim automation rebuilds the
effective value from the new base; absolute automation overwrites it. No lane
recompile is needed for a base change.

### 5.3 AutomationPlan

The compiler produces an immutable, allocation-free-to-read plan:

```rust
pub struct AutomationPlan {
    pub lanes: Vec<LanePlan>,
}

pub struct LanePlan {
    pub address: ParamAddress,
    pub slot: usize,             // resolved on the control side
    pub mode: AutomationMode,
    pub points: Vec<PointPlan>,  // frame domain, sorted
}

pub struct PointPlan {
    pub frame: u64,
    pub value: ParamValue,
    pub curve: Curve,
}
```

Keyframes are musical, so they convert to frames through the current tempo map
at compile time. A tempo change already forces a recompile, so the plan always
matches the project. This keeps the audio thread free of tempo conversion and
i128 division.

Slots are resolved before handoff. Adding a lane can only target a parameter
that already has a slot; the compiler ensures slots for every document
parameter and every device-reported parameter.

### 5.4 Evaluation and block-size independence

The plan evaluates at absolute frames, not at block-relative indices.

- `value_at(frame)` is a pure function used by the engine, tests, and the UI
  (the UI converts its tick to a frame or evaluates in the tick domain).
- The engine splits a processing block at the next automation boundary:
  `min(block_size, next_keyframe_frame)`. Note events are scheduled per chunk by
  the existing `Scheduler`; realtime and offline both already have a chunk loop.
- Within a chunk, a linear segment is a single ramp; a hold segment is a single
  value. The keyframe corner is never smeared across a block boundary.
- Core devices that want sample-accurate automation read `float_at`; the channel
  strip uses it for gain and pan. `Sine`, `Gain`, and future built-ins can adopt
  it incrementally.
- VST3 is block-quantized today. It can add a start and end parameter point per
  chunk, which is block-accurate with linear ramps. Truly sample-accurate VST3
  parameter changes remain a separate depth step; the plan does not pretend
  otherwise.
- For core devices, the combination of chunk splitting and absolute-frame ramps
  must be **byte-identical across block sizes**. Add a golden test that renders
  the same automated project at several block sizes and compares samples.
  Curves added later must preserve this; do not approximate a curve from block
  endpoints.

### 5.5 Applying a plan

At the start of each chunk the engine:

1. for each lane, reads the base value from the slot (or the plan's fallback);
2. evaluates the lane at the chunk start and end;
3. applies Absolute or Trim;
4. writes a value or a ramp with `set_effective` / `set_ramp`.

Discrete values (Bool/Enum, and Int with a step) get `set_effective` and no
ramp. Float values get a ramp when start and end differ.

An empty or disabled lane is skipped at compile time; an unresolved slot is
reported as a compile diagnostic rather than silently ignored.

### 5.6 Swapping automation without rebuilding the graph

Add a realtime command:

```rust
enum ToAudio {
    // existing...
    SetAutomation(Box<AutomationPlan>),
}
```

- The control side compiles a new plan from the current project plus the stable
  slot map.
- The audio thread swaps it at a block boundary and resets its per-lane cursors
  by binary-searching the current frame.
- The old plan goes back on the retirement queue and is dropped by the control
  thread. Do not drop a `Box` on the audio thread; this is the same rule as
  `RealtimeState`.
- If the queue is full, the edit still committed to the document but the live
  swap failed. Surface it as a status/error and retry on the next frame; do not
  silently desync playback.

The existing retirement queue carries `RealtimeState`. Generalize it to an enum
or add a parallel automation queue, so both kinds are freed on the control
thread.

### 5.7 Offline render

The offline renderer uses the same `Engine`, so it gets automation for free once
the plan exists. Its chunk loop must split at automation boundaries exactly like
the realtime loop. `odaw render` then produces the automated result
deterministically and byte-identically for a fixed block size.

---

## 6. UI design in detail

### 6.1 Automation tab layout

A left sidebar and a canvas:

**Sidebar: targets**

```text
[Search parameters...]

Track 3
  ● Volume          A   (lane)
  ○ Pan                 (no lane)
  ▾ Dexed
      Cutoff        A
      Resonance
      ...
  ▾ Hall
      Mix
```

- `A` marks a parameter with a lane; clicking the row selects it.
- A `+` appears on hover for parameters without a lane.
- Eye/lane visibility controls live in the arrangement, not here, to keep this
  list compact.
- Read-only parameters are hidden or disabled with a tooltip.
- VST3 parameters are only available once the plugin is live. Show a clear
  "open the plugin once to enumerate parameters" message otherwise.

**Canvas: the lane**

- Ruler: bars, beats, snap subdivisions, matching the timeline. Horizontal zoom
  and scroll should feel like the timeline; a `Fit` button frames the lane's
  points and the song.
- Y axis: min/mid/max labels plus the value at the cursor. Use the parameter
  descriptor's `format`, so gain reads `-6.0 dB`, pan reads `L35`, and a plugin
  parameter uses the plugin's own string.
- Curve: line plus a subtle area fill under it, in the track color. Hold
  segments are horizontal-then-vertical steps. Linear segments are straight
  lines.
- Points: 7 px circles; selected points use the accent color with a white ring;
  hovered points get a larger hit area. Draw only the visible range.
- Playhead: the red transport cursor. A separate edit cursor shows the hovered
  tick and value as a crosshair and tooltip.
- Grid: bar lines stronger, beat lines medium, snap substeps faint. Value grid
  at 25/50/75% of the range; a stronger zero line for Trim and pan.

**Toolbar above the canvas**

- Target name and device path.
- `Absolute | Trim` selector (disabled for Bool/Enum).
- `Enabled` toggle.
- `Arm` record toggle.
- Snap controls: re-use `SnapSettings` (on/off, triplets, division) with its own
  state, just like the piano roll.
- `Clear lane`, `Copy lane`, `Paste lane`.
- Selected-point count and value.

### 6.2 Gestures

Follow the piano roll's pointer model so the app feels consistent:

| Gesture | Result |
| --- | --- |
| Click empty grid | Add a point at the snapped tick and current pointer value |
| Drag empty grid | Marquee-select points |
| Drag a point / selection | Move in time and value; snap time, free value; Shift constrains to one axis |
| Ctrl/Cmd + drag a point | Duplicate the selection and drag the copies |
| Double-click a point | Numeric value entry |
| Right-click a point | Curve, value, duplicate, delete |
| Right-click empty grid | Add point here, paste, clear selection |
| Delete / Backspace | Delete selected points |
| Left/Right | Nudge tick by snap; Shift by a larger step |
| Up/Down | Nudge value by 1% of range; Shift by 10% |
| Ctrl/Cmd+C / V | Copy/paste points at the edit cursor |
| Alt + wheel / pinch | Horizontal zoom |
| Ctrl/Cmd + wheel | Horizontal zoom |

A `Pointer | Draw` tool toggle is worth adding for freehand automation. Draw
mode samples the drag, then thins and commits one lane on release. Keep it a
small pair of buttons, not a hidden modal.

### 6.3 Adding and selecting lanes

- Selecting a parameter with no lane shows a centered "Add automation lane"
  button and the base value.
- Adding a lane inserts one point at the edit cursor with the current base
  value. One point holds everywhere, so the lane is immediately visible and
  meaningful.
- Selecting a different track updates the sidebar. The current lane stays
  selected if its node still exists; otherwise fall back to the track's volume
  lane or the first lane on the track.
- The arrangement lane row and the bottom editor are two views of the same
  selection. Clicking a lane header in the arrangement switches the bottom tab
  to that lane. Editing one redraws the other next frame because both read the
  same document.

### 6.4 Parameter descriptors

The editor needs `label`, `min`, `max`, `step_count`, `unit`, `read_only`, and a
formatter. `DeviceParameter` already has all of it, but it is only exposed for
VST3 today.

Add to `DeviceProcessor`:

```rust
fn parameter_info(&self) -> Vec<DeviceParameter> { Vec::new() }
```

- VST3 returns its live parameters (and bypass).
- Built-in devices return their ranges, e.g. `gain` 0..0.2? No: use the natural
  unit, and let the device clamp.
- The compiler adds the channel-strip descriptors:
  - `gain_db`: -60..+12 dB, default 0, continuous;
  - `pan`: -1..+1, default 0, continuous, formatted as `L/C/R`.

The compiler collects this into:

```rust
pub struct CompiledProject {
    pub plan: RenderPlan,
    pub params: ParamBuffer,
    pub meters: MeterBank,
    pub automation: AutomationPlan,
    pub parameters: BTreeMap<ParamAddress, DeviceParameter>,
}
```

The UI caches `parameters` as derived metadata. It is not document state and is
rebuilt with the project. A missing descriptor falls back to 0..1 and a plain
decimal formatter, so the UI never blocks on metadata.

### 6.5 Faders, automation, and recording

When a parameter has an enabled lane and the transport is playing:

- the control displays the lane's value at the playhead, not the base value;
- it is marked with `A` and is read-only by default;
- attempting to drag shows "Arm this lane to write automation" rather than
  silently changing a value that playback ignores.

When stopped, dragging edits the base value. This is how the user sets the level
that Trim offsets from, and how Absolute mode's fallback changes.

When the lane is armed and the transport is playing, dragging:

- sends a touch/override to the engine so the live value is audible instead of
  being overwritten by the lane;
- records `(playhead_tick, value)` at each UI update;
- on release (or stop), thins the points and commits one lane command.

This is touch-style recording. Write and latch modes can follow once the core
is trustworthy; do not ship five recording modes before one works.

---

## 7. Recording pass

### 7.1 Capture

`Editor::preview_param` already receives every live gesture value. Add a
recording buffer:

```rust
struct RecordPass {
    target: ParamAddress,
    points: Vec<(Tick, ParamValue)>,
    mode: RecordMode, // Touch for now
}
```

- Start on the first touched value while playing with the lane armed.
- Capture the playhead tick at the same instant, not the UI frame start. The
  playhead frame is available; convert with the tempo map.
- Ignore duplicate values that differ by less than the tolerance.
- Stop on release or transport stop.

### 7.2 Thinning

Use a distance threshold over `(tick, normalized_value)`; a full RDP pass is a
future refinement:

- tolerance: 0.5% of the parameter range;
- minimum spacing: 1/64 note or 5 ms, whichever is larger;
- always keep the first and last point;
- discrete parameters record on value changes only.

This resolves the open item in `DESIGN.md` §8.3 with a concrete default; expose
it as a per-project/UI setting later if users ask.

### 7.3 Commit

- Touch mode: replace points inside the touched range, keep points outside.
- If the lane is empty, creating the first point at the first captured time is
  enough; its value holds before and after.
- One `AutomationCommand::Set` (or a range-specific command if lanes grow very
  large) and one history entry.
- On release, send the touch-off command so the lane takes over again.
- If the user undoes during a pass, cancel the pass rather than committing a
  lane built against a stale base.

### 7.4 Arm and record UI

- A global `Rec` button in the transport, next to Play.
- A per-lane arm dot. Arming a lane on a track should not implicitly arm MIDI
  recording; those are separate futures.
- A red outline on the lane while recording, and a "recording" status.

---

## 8. Implementation order

Each phase leaves the tree green and usable. Do not start the next phase until
its predecessor's acceptance criteria pass.

### Phase A — Semantics, validation, change tracking

Scope:

- Add `AutomationLane::enabled`; bump format to v5 with a v4 migration.
- Add a pure evaluator in `core`:
  - `value_at(tick)`, `value_at_frame` only if needed;
  - before/after hold, exact-keyframe behavior, Float/Int/Bool/Enum rules;
  - trim combination and mode conversion;
  - keyframe normalization (sort, dedupe, finite check).
- Extend `core::validation`.
- Add `ChangeSet.automation`, `is_automation_only`, and fix
  `Tx::set_automation` to stop marking structural.
- Dispatcher: normalize lanes, emit a diagnostic for invalid input, and remap
  lanes by `ParamId` when a device kind changes (keep matches, drop the rest
  with an event).
- Tests: evaluator, trim, discrete, normalization, undo.

Acceptance: lane edits are value-channel changes; a 100-point lane saves,
reloads, undoes, and redoes perfectly; no graph rebuild is triggered.

### Phase B — Render playback

Scope:

- Extend `ParamBuffer` with base/effective/ramp/`float_at`.
- Add `AutomationPlan`, `LanePlan`, `PointPlan`; compile from the project and
  the tempo map.
- Add `CompiledProject.automation` and `parameters`; add
  `DeviceProcessor::parameter_info`.
- Engine: per-chunk automation application, cursors, `set_effective`/`set_ramp`,
  boundary splitting.
- Realtime: `SetAutomation` command, retirement handling, automation-only edit
  path in the editor.
- Channel strip: use `float_at` for gain and pan.
- Offline render: same chunk splitting.
- Tests: gain ramp sample output; block-size independence; trim; hold; disabled;
  automation-only swap does not rebuild devices; golden WAV.

Acceptance: automate gain and pan, hear smooth changes, render offline
identically across block sizes, and drag points while a VST3 instrument plays
without resetting its voices.

### Phase C — Bottom-panel editor

Scope:

- Rename the panel state and add `BottomTab::{Notes, Automation, Clip}`.
- Parameter catalog from `CompiledProject.parameters`.
- Sidebar with track/device grouping and search.
- Large canvas: grid, ruler, value axis, curve/points, playhead, cursor,
  snap/zoom/fit, context menus, marquee, move, add, delete, copy/paste, undo.
- Mode/enable/clear/copy/paste lane controls.
- Right-click "Show automation" on existing parameter controls and the `A`
  badge.
- Pure geometry/value helpers with unit tests.

Acceptance: create, edit, and delete lanes for a track fader and a VST3
parameter; undo each edit as one step; save/reopen; the piano roll and audio clip
views still work.

### Phase D — Arrangement lanes

Scope:

- Variable-height track rows in the timeline, with matching header and grid
  offsets.
- `A` toggle per track, per-lane visibility, compact lane rendering.
- Selection sync with the bottom editor; click a lane to open it; coarse
  editing in the compact view (or read-only first if schedule demands it).
- Keep clip dragging, marquee, split, drop targets, and scrolling intact.
- Tests for row offsets and hit-testing helpers.

Acceptance: several lanes under a track align with the ruler and clips at every
zoom; no regressions in clip editing; collapsing restores the original layout.

### Phase E — Recording

Scope:

- `RecordPass` capture from live parameter gestures.
- Engine touch override so the live value is audible during a pass.
- Distance thinning with the proposed defaults.
- Touch-mode arm/record UI, global `Rec`, one-transaction commit, touch-off.
- Fader read/follow behavior for automated parameters.

Acceptance: arm a gain lane, play, move the fader, release, and hear the pass;
one undo removes it; the saved project renders the same pass.

### Phase F — Depth

- Curve shapes beyond Hold/Linear (converted to fixed-resolution segments or
  evaluated per absolute sample so block-size independence survives).
- Write and latch modes, punch ranges.
- Lane copy/paste across parameters with range scaling.
- Trim/Absolute conversion UI and lane duplication.
- Automation for built-in device parameters beyond the strip.
- CLI: `odaw automation list/set/clear`.
- Performance: visible-range culling, boundary index for many lanes.

---

## 9. Decisions worth confirming before Phase A

These are the choices that are expensive to change later:

1. **Format v5 for `enabled`.** Recommended. The alternative is to treat lane
   presence as enablement and delete lanes to disable them, which loses the
   lane's points and makes the toggle destructive.
2. **No `KeyframeId`.** Recommended. Whole-lane replacement is sufficient and
   keeps the format small.
3. **Two views, bottom editor primary.** Recommended. A timeline-only design
   makes multi-parameter editing cramped; a bottom-panel-only design hides
   automation against the song.
4. **Arrangement lanes editable or display-only in Phase D.** Start read-only
   if timeline refactoring is already large; the bottom editor is the source of
   truth.
5. **Touch recording first.** Recommended. Do not build write/latch semantics
   before the touch path is proven.
6. **Curves later.** Hold and Linear cover the first release. Curve expansion is
   a separate format/version decision because it changes evaluation and
   block-size determinism.
7. **Thinning defaults.** 0.5% value tolerance and 1/64 note / 5 ms minimum
   spacing. This can be a setting later.
8. **Smoothing.** Prefer automation ramps over an implicit per-device smoothing
   filter. If a device needs extra smoothing, declare it in its parameter
   descriptor rather than hiding it in the render path. This is the natural
   answer to `DESIGN.md` §8.1.

---

## 10. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Point drags rebuild the graph and reset plugins | Phase A's `ChangeSet.automation` and Phase B's dedicated plan swap; test with a live VST3 instrument. |
| Automation steps are audible (zipper) | Absolute-frame ramps and `float_at` in the channel strip; test a slow gain ramp. |
| Output depends on block size | Split chunks at keyframes; evaluate from absolute frames; golden test across block sizes. |
| Audio thread frees the old plan | Retirement queue for `AutomationPlan`, same rule as `RealtimeState`. |
| Huge plugin parameter lists overwhelm the UI | Search, grouping, scroll, and visible-range drawing. |
| Timeline row refactor breaks clip editing | Isolate variable-height rows behind a `TrackRow` layout helper; test offsets and hit tests. |
| Trim and Absolute are confusing | Mode badge, zero line, mode conversion, and fader read-only behavior. |
| Recorded passes are noisy | Distance thinning, minimum spacing, and one-transaction undo. |
| Missing plugin parameters at load | Show an explicit "instantiate the plugin to enumerate parameters" state; never create a lane for an unknown parameter. |

---

## 11. Test plan

- **Core:** table tests for every interpolation and boundary case; property
  tests for clamp/range and monotonic linear lanes; normalization tests.
- **Control:** lane command round-trips, undo/redo, invalid input, device-kind
  remap, automation-only change classification.
- **Compile:** tick-to-frame conversion, disabled/empty lane omission, slot
  resolution, multiple lanes, tempo change recompiles.
- **Engine/render:** exact sample values for a simple ramp, hold steps at
  keyframes, trim with a moving base, block-size independence, deterministic
  golden WAV, no device rebuild on a plan swap.
- **Realtime:** plan swap at a block boundary, queue-full path, retirement.
- **UI:** pure helpers for y mapping, snapping, hit testing, thinning, and mode
  conversion; manual smoke tests for the tab and arrangement layouts.

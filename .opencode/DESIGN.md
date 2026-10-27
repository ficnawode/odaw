# DESIGN

Status legend:

- `[CONFIRMED]` — decided; changing it later is expensive.
- `[PROPOSED]` — current best guess; still open.
- `[OPEN]` — no decision yet.

This document is a living sketch. It fixes the vocabulary and ownership
boundaries first, because later decisions depend on them.

Only the boundaries are expensive to reverse: the control/render split, the
ownership and trust boundaries (§6), the device ABI, the time and
determinism invariants, and the extension model. Everything else — concrete
types, field layouts, and named crates — is a working sketch that shows
intent and constrains the design, but is not frozen until code depends on
it. `[CONFIRMED]` means decided and costly to reverse for now, not a
promise. Fill the rest in as we build.

---

## 1. What this program is

A small, deterministic, scriptable music application. The core is a
portable engine that can run without any UI. The UI is one client among
many (CLI, scripts, future remote clients).

The central idea:

> A `Project` (plain data) is compiled into a `RenderPlan`, and the plan is
> rendered either to a sound card in real time or to a file offline, using
> the same engine.

`Project` -> `RenderPlan` -> WAV (offline) / audio device (realtime).

This is not a clone of an existing DAW. The core stays small; capability
comes from three extension kinds:

- in-tree devices (`dsp`),
- CLAP plugins behind one `Device` abstraction,
- control/compile-time Lua that builds document data, operators, and device
  specs, but never runs on the audio thread.

An Ableton-like workflow (looping, reusable and hidden sections, arrangement)
is expressed on top of this core, not baked into it. Live session view and
the clip-launcher grid are out of scope; playing and jamming happen on the
timeline.

### 1.1 Principles

- One coherent core, not a suite. A feature lives in an extension unless
  the extension API itself needs it.
- Batteries optional. The core ships a minimal set of primitive devices;
  instruments, effects, and presets are packs, never bundled bloat.
- Fast, quiet startup. No eager plugin scan, no background services, no
  network on launch. Scanning is lazy, cached, and deferrable.
- Extensible by design. Anything the UI can do, a script can do through the
  same command/operator API. That API and the device ABI are the product
  surface and are versioned.
- Composability over special cases. Prefer combining existing primitives to
  adding a new core type. A new core concept is a last resort.
- Deterministic and inspectable. The document is plain, versioned data, and
  renders are reproducible.

The cost of this stance is that the extension surface must stay stable. If
it churns, extensibility fails and the core grows back into a bundle.

---

## 2. Two machines `[CONFIRMED]`

There are two execution models in this program. They must stay separate.

### 2.1 The control side: the command stream

The control side is linear. A `Session` owns a `Project` and applies a
totally ordered stream of `Command`s. Each command changes the state left
by the previous command, so the order matters. This is not a graph and is
not sorted. Undo, macros, and future collaboration all need this single
linear history.

`Session` is single-writer. At first, command sources call it directly;
later, they feed a queue that the `Session` thread drains. Either way only
one thread mutates the document, so the history stays linear. The model
must scale to hundreds of tracks and many clips, so edits are incremental
and the compiler only touches the affected subgraph.

### 2.2 The render side: the signal graph

Inside the `Project` there is a signal `Graph` (nodes and edges). It is a
directed acyclic graph. The `Compiler` sorts it, resolves delays and
channel layouts, and produces a `RenderPlan`. The realtime executor walks
the plan block by block.

```
 command sources: UI, CLI, Lua tools, remote, macros
        |
        v   (Command stream, total order)
   Session  ------owns------>  Project  (source of truth, serializable)
        |                          |
        |                      Compiler  ---->  RenderPlan  (DAG, sorted)
        |                          |                  |
        +--> Events (out)          +-- offline --->   WAV
                                              realtime -> audio device
```

### 2.3 Consequence: commands are not DSP

A command is a change to the document. A DSP unit is a node in the signal
graph. These are different things and use different words (see vocabulary).
A command never runs on the audio thread. A DSP unit never changes the
document.

Command sources (UI, CLI, scripts, remote, macros) are interchangeable:
they all go through one command API. DSP units are not command sources.

### 2.4 Mutation and undo `[CONFIRMED]`

The `Project` is an immutable value. A command does not edit it in place;
it produces a new value that shares structure with the previous one.
Undo and redo move a cursor over the saved values.

- The `Project` is built from persistent collections: persistent maps for
  the id-keyed sets (nodes, clips, edges) and persistent vectors for
  ordered lists (rack contents, automation points, clip order). Cloning a
  `Project` is O(1), because the collections share their internal nodes.
- Media and plugin state are immutable blobs behind a shared pointer, so
  cloning a `Project` never copies audio.
- A command is applied by the dispatcher through a staging handle `Tx`
  that holds a working copy of the `Project`. On commit the working copy
  becomes the new immutable root.
- `Session::apply(operator) -> Result<Vec<Event>, Error>` stages, commits,
  and appends a `History` entry. A `History` entry is the new root
  snapshot, the transition's `ChangeSet`, and the operator invocation
  `(id, version, params)` when the entry came from an operator.
- Undo and redo move the cursor, then replay the same effects as a forward
  commit, using the transition's `ChangeSet` and the destination snapshot;
  see 2.8. ChangeSets are directionless, so one path handles both
  directions. No command writes its own inverse; the old root is the
  inverse.
- `Tx` tags changes as structural or value, which is how the session knows
  whether to recompile the plan or to write the parameter buffer.
- A value gesture (a knob drag) writes the base value. Intermediate values
  go to the realtime parameter buffer for immediate feedback; the document
  only receives the final value. The whole gesture is one transaction and
  one undo step.
- Automation recording is a separate case: incoming values are thinned into
  `Keyframe`s on the target lane, and the whole pass is one transaction.
  See 4.3.
- `Command` is serializable, for macros and logs, and carries an optional
  operator reference (id and version). The on-disk format is a separate set
  of versioned DTOs (see section 3), not the live snapshot.

### 2.5 Command data and the dispatcher `[CONFIRMED]`

`Command` is serializable data, not a function. One dispatcher maps each
command to writes on the staging area. The variants below show the intended
scope of mutation; the exact set is illustrative until the dispatcher
exists.

```
struct Command {
    version: u16,
    operator: Option<OperatorRef>,  // set when an operator produced it
    kind: CommandKind,
}

struct OperatorRef { id: OperatorId, version: u16 }

enum CommandKind {
    Meta(MetaCommand),              // project meta, sample rate, seed
    Asset(AssetCommand),            // import, replace, remove
    Node(NodeCommand),
    Device(DeviceCommand),
    Edge(EdgeCommand),
    Clip(ClipCommand),
    Section(SectionCommand),
    Arrangement(ArrangementCommand),
    Loop(LoopCommand),
    Param(ParamCommand),
    Automation(AutomationCommand),
    Tempo(TempoCommand),            // includes time signatures
    Marker(MarkerCommand),
}
```

- Every mutation is a command, including project metadata and assets. UI,
  CLI, Lua, macros, and the protocol all produce commands, and nothing else
  writes to the project. This is what makes the command log authoritative
  and replayable.
- `fn dispatch(cmd: &Command, tx: &mut Tx) -> Result<(), Error>` reads the
  current state from `Tx`, then performs writes. It is the only code
  allowed to write through `Tx`.
- Complex edits (split a clip, delete a node and rewire its neighbors) are
  still a single command; the dispatcher reads what it needs and issues
  several writes.
- Scripts and plugins never get `Tx`. Scripts get an API that builds
  commands. Plugins are not command sources at all.
- Commands are versioned, because the log and macros must replay.

### 2.6 Operators `[CONFIRMED]`

A `Command` is a primitive mutation with no user-facing identity. An
`Operator` is a user-facing action: a label, typed parameters, an
enabled/disabled check, and a body `run(&Project) -> Vec<Command>` that
produces commands. A `Command` carries an optional `OperatorRef`, so
`apply_command` can dispatch a bare command and `apply` stamps its id and
version.

- Every front-end invokes operators: UI, CLI, Lua, macros. This keeps the
  "the UI is not special" rule true.
- Advanced scripts may emit commands directly.
- `[CONFIRMED]` Operators and the operation log are in-project. They target
  concrete `NodeId`s, and a macro replayed after its targets are deleted
  fails cleanly with an `Event` rather than writing a partial edit.
  Cross-project portability is a later feature and does not shape the type
  now.
- `[CONFIRMED]` Operator parameters use a reference type, `TargetRef`,
  which currently has one variant, `Explicit(NodeId)`. The front-end
  resolves a `TargetRef` against the current project (and selection) at
  invocation, and the `History` entry records the concrete binding used.
  Later variants (`ByPath`, `ByName`) can be added without a breaking
  change.
- `[CONFIRMED]` `Operator` is an additive ABI. It carries an
  `operator_version`; shipped semantics never change, old versions stay
  registered, and parameters are versioned DTOs like commands. History
  entries and macros record `(operator_id, operator_version, params)`.
- `Session` remembers the last operator and its parameters, so "adjust the
  last action" is possible.
- `Operator` is serializable: label, typed parameters, and the `TargetRef`s
  it targets. The log, macros, and "adjust last action" replay the
  operator, not just its commands.
- Each `History` entry is tagged with the operator that produced it, so
  the UI can present readable undo labels.

The alternative would be to store only commands, losing readable history
and "adjust last action".

### 2.7 The staging area (`Tx`) `[CONFIRMED]`

`Tx` is a working copy plus a change log. The `Session` creates it, the
dispatcher writes through it, and it is either committed or dropped.

```
struct Tx {
    project: Project,   // persistent: writes clone only the changed paths
    changes: ChangeSet,
    emitted: Vec<Event>,
}
```

Reads: `tx.node(id)`, `tx.edge(id)`, `tx.param(addr)`, `tx.project()`.
Reads do not mutate and do not allocate.

Writes fall into two categories. The category, not the exact method
surface, is what matters; the API settles with the dispatcher.

- Structural writes: add/remove/reorder nodes and devices, connect and
  disconnect edges, add/remove/move/trim clips, and set a parameter that is
  declared structural.
- Value writes: set an automatable parameter.

`ChangeSet` records what changed, in a form that lets the compiler compute a
dirty set. Its requirements are fixed; its shape is a sketch:

- Composable: several commands in one transaction merge.
- Directionless: undo and redo read the same delta against the destination
  snapshot.
- Distinguishes structural from value changes, and flags when downstream
  layout or latency may be affected (the escape hatch for "rebuild
  everything downstream").
- A `set_param` on an automatable parameter is a value change; on a
  structural parameter (for example a channel-layout switch) it is a
  structural change.

```
// sketch
struct ChangeSet {
    structural: bool,
    nodes: PersistentMap<NodeId, TouchKind>,  // Added, Removed, Reordered,
    edges: PersistentSet<EdgeId>,             // RoleChanged, KindChanged,
    params: PersistentSet<ParamAddress>,      // StateChanged
    layout_or_latency: bool,
    meta_or_tempo: bool,
    arrangement: bool,
}
```

The compiler turns the `ChangeSet` into a dirty set over its cached
dependency graph and rebuilds only dirty fragments in topological order; see
section 2.8.

Commit and abort:

- `Tx::commit() -> (Project, ChangeSet, Vec<Event>)` returns the working
  copy and the log.
- Dropping a `Tx` without commit discards everything, so a failed command
  cannot leave a partial edit. Transactions are atomic by construction.
- `validate()` checks references: no dangling edge, valid ports,
  compatible channel layouts, no cycle in the graph (feedback is expressed
  with explicit delay nodes, so the graph stays a DAG), assets present. It
  runs for structural changes and is skipped for value changes.

### 2.8 What the Session does with a commit `[CONFIRMED]`

```
Session::apply(operator):
    commands = operator.run(&project)   // pure; reads, produces commands
    commit the commands atomically, record one History entry,
    then apply the effects to the render side.

Session::apply_command(command):
    same, with the operator reference taken from the command.

Session::apply_effects(project):
    if the change is structural, rebuild the affected plan fragments;
    write changed parameter values into the parameter buffer.
    Also runs on undo, redo, and snapshot loads, against the destination
    snapshot.
```

- `dispatch` is the only code that writes through `Tx`. Operators return
  commands; the Session dispatches them.
- `apply` and `apply_command` differ only in where the operator reference
  comes from: the operator itself, or the command's optional field.
- `apply_effects` is the single place the realtime side is updated. Undo,
  redo, and snapshot loads call it with the destination snapshot, so audio
  always matches the document.
- `rebuild_plan` consults the `ChangeSet` and recompiles only the affected
  subgraph. Plan fragments and device instances are cached by `NodeId`, so
  unaffected devices are reused as-is and their plugin state, sample data,
  and filter tails are never rebuilt.
- Recompilation reads device metadata only. It never mutates, resets, or
  prepares a live device instance. A replacement instance — a new plugin
  load, a newly added device, or a device whose state must change — is built
  and prepared off the audio thread by the control side, and becomes live
  only at the plan handoff. Only the audio thread touches a live instance
  (see section 6).
- Parameter writes go to the preallocated lock-free buffer. A rebuild never
  moves or invalidates a slot: slots are assigned per `ParamAddress` and
  are stable for the life of the project. See point 4 in section 8.
- The buffer is typed, with accessors per `ParamKind` (`float`, `int`,
  `bool`, `enum`); device code never casts a raw integer, the plan supplies
  the type. Control parameters are read once per block; audio-rate
  parameters come from a per-sample signal path.

### 2.9 Project data layout `[CONFIRMED]`

One id space for nodes. A `Device` is a `Node` whose role is `Device`.

```
Project {
    meta: Meta,                              // name, sample rate, seed
    tempo: PersistentVector<TempoPoint>,
    signatures: PersistentVector<TimeSignature>,
    markers: PersistentMap<MarkerId, Marker>,
    sections: PersistentMap<SectionId, Section>,
    arrangement: PersistentVector<SectionRef>,
    loops: PersistentVector<LoopRegion>,
    nodes: PersistentMap<NodeId, Node>,
    edges: PersistentMap<EdgeId, Edge>,
    clips: PersistentMap<ClipId, Clip>,
    assets: PersistentMap<AssetId, Asset>,
}

Node {
    role: NodeRole,                    // Track, Bus, Master, Send, Folder,
                                       // Input, Output, Modulator, Device
    kind: Option<NodeKind>,            // set when role is Device
    name: Option<String>,              // display name, mostly for tracks
    color: Option<u32>,                // 24-bit RGB, mostly for tracks
    muted: bool,
    soloed: bool,
    rack: PersistentVector<NodeId>,    // device node ids, serial order
    ports: PersistentVector<Port>,
    params: PersistentMap<ParamId, ParamValue>,
    automation: PersistentMap<ParamId, AutomationLane>,  // by target param
    modulations: PersistentMap<ParamId, PersistentVector<ModulationLink>>,
                                                         // by target param
    clips: PersistentVector<ClipId>,   // track nodes only
    state: Option<Arc<[u8]>>,          // opaque plugin state, device nodes only
}
```

- `edges` connect ports of routing nodes. A rack is serial, so its devices
  are ordered by `rack` and are not connected by edges.
- `ParamAddress { node: NodeId, param: ParamId }` addresses track
  parameters and device parameters alike, because devices are nodes.
- `Node.automation` is keyed by the target `ParamId`, so the lane for a
  parameter lives on the node that owns it. Its target is implicit, it
  cannot dangle, and deleting the node deletes its lanes. There is no
  separate global lane map.
- `[CONFIRMED]` Modulation uses the same ownership rule. `Node.modulations`
  is keyed by the target `ParamId`; a link is
  `ModulationLink { source: NodeId, amount }` and `source` is a `Modulator`
  node. It cannot dangle, and deleting the target deletes its inbound
  links. A derived reverse index (`source -> Set<ParamAddress>`), rebuilt
  on load and repaired by the dispatcher, lets a modulator list its targets
  and be deleted without a full scan. The compiler folds each link into the
  DAG as an implicit edge, so ordering, cycle checks, and incremental
  recompile all see it.
- Asset blobs live in the `AssetStore` outside `History`; the `Project`
  holds metadata and the content hash. Evicting history never frees an
  asset that a live snapshot still references. See section 2.10.
- `sections` holds reusable spans of clip placements. `arrangement` is the
  ordered song structure: a list of `SectionRef { section, repeat, active }`.
  Setting `active = false` hides a section without deleting it. `loops`
  holds timeline ranges with looping enabled. Arrangement is data, edited by
  commands; the compiler and scheduler only see the flattened result.
- The compiler flattens each rack into a serial chain and combines these
  with `edges` and `modulations` into one DAG. It inserts delay nodes to
  break feedback and compensates device latency on parallel paths.

### 2.10 History pruning `[CONFIRMED]`

`History` is a bounded deque of snapshots with a cursor. Memory has two
independent budgets: structure and asset bytes.

- Structure is bounded by a transaction count limit. Proposed default: 512
  transactions. Each transaction has a bounded structural delta, so the
  count limit bounds the structure.
- Asset bytes are *not* bounded by the count limit. The `AssetStore` owns
  the immutable blobs outside `History` and refcounts them; evicting a
  snapshot frees a blob only when no live snapshot references its
  `AssetId`. A separate `asset_budget` bounds total asset bytes and
  triggers eviction or an explicit purge when exceeded. There is no
  "count limit is also a memory bound" claim.
- Because snapshots share structure, evicting one frees only the nodes
  that no live snapshot still refers to.
- Saving the project collapses all entries older than the saved snapshot
  into one baseline snapshot (a savepoint). Eviction can then drop the
  prefix while the count bound still holds, and "revert to saved" loads the
  baseline. Saving does not pin the whole chain.
- Single-step undo and redo use the crossed entry's `ChangeSet`. A jump
  across several entries ("revert to saved", click an old entry) simply
  loads the destination snapshot and does a full resync: recompile if any
  structural difference exists and rewrite every parameter value.
  Intermediate entries are not replayed.
- Optional later: named checkpoints, and collapsing old history into one
  coarse baseline independently of save.

---

## 3. Cross-cutting contracts

The invariants that everything else is built against. Items are covered in
more detail where they originate; this is the checklist. Names of crates and
concrete types are illustrative unless a boundary depends on them.

- `[CONFIRMED]` The `Project` is the source of truth and is serializable.
- `[CONFIRMED]` Time is integer only. Musical position is `Tick` (`i64`),
  audio position is `Frames` (`i64`); floating-point seconds are never
  canonical. Tempo is integer microseconds per quarter note at breakpoints.
- `[CONFIRMED]` The signal graph is the document format from the start. It is
  a DAG; feedback uses explicit delay nodes, never implicit cycles. Whether
  the executor is serial or parallel is an implementation detail.
- `[CONFIRMED]` `Device` is the only DSP role. `Instrument` is a `NodeKind`
  (a device with no audio input), not a role.
- `[CONFIRMED]` Every document mutation has a `Command`, including meta,
  signature, and asset changes.
- `[CONFIRMED]` Control API: `Session::apply(Operator)` for front-ends and
  `Session::apply_command(Command)` for low-level callers. Both use one
  dispatcher and one effects path.
- `[CONFIRMED]` The core is deterministic: no wall clock, no unseeded
  randomness, no locale-dependent formatting, no output-affecting unordered
  iteration. Randomness comes from a project-seeded, counter-based generator
  keyed by `NodeId` and absolute sample index. Nondeterminism may enter only
  through plugins or scripts that affect output, and each such item declares
  it.
- `[CONFIRMED]` Determinism is tiered. Core devices render byte-identically
  across block sizes. Plugin output is deterministic for a fixed `(plugins,
  state, block size)`, and the offline renderer fixes and records its block
  size. Realtime is not bit-reproducible. Time-based behavior is a function
  of absolute sample position, never of block count; live `set_param` and
  gesture writes are block-quantized and outside this guarantee.
- `[CONFIRMED]` The engine always runs at `Meta.sample_rate`, which is
  immutable for the life of a project. Assets are resampled on import. If
  the device cannot run at that rate, one master resampler at the platform
  edge converts the final mix. Changing a project's rate is an explicit
  offline conversion.
- `[CONFIRMED]` Every device reports its latency; the compiler inserts
  compensating delays on parallel paths.
- `[CONFIRMED]` Device state blobs change only on structural or preset
  commands, and are content-addressed and deduplicated.
- `[CONFIRMED]` Parallel execution only runs nodes proven independent in the
  DAG, and a node sums its inputs in a fixed, declared order, so parallel
  output equals serial output.
- `[CONFIRMED]` Render-side state crosses to the audio thread as owned state
  over bounded lock-free queues. The audio thread swaps state only at block
  boundaries and never locks, allocates, or frees; outgoing state returns on
  another queue and is destroyed by the control thread.
- `[CONFIRMED]` The realtime side never allocates, locks, does I/O, parses
  text, or calls scripts. Enforced in CI.
- `[CONFIRMED]` Assets are content-addressed by hash and embedded in the
  project container; cached renders and analyses are keyed by input hashes.
- `[CONFIRMED]` The project format is versioned from the first commit, with a
  migration path. On-disk DTOs are separate from the live types.
- `[CONFIRMED]` Parameter values are numeric only: float, integer, boolean,
  or enum. Text and files are document references or plugin state.
- `[CONFIRMED]` Scripting is control/compile-time Lua, never on the audio
  thread.
- `[CONFIRMED]` Plugins are CLAP, behind the same `Device` abstraction as
  in-tree devices. The plan drives devices through a transport-agnostic
  `DeviceProcessor`; in-process is the default and process isolation is
  another implementation of the same interface.
- `[CONFIRMED]` Automation drives plugin parameters through sample-accurate
  CLAP events; odaw modulators map to CLAP parameter modulation where the
  plugin supports it.
- `[CONFIRMED]` Sections are reusable and referenced by the arrangement; an
  instance may be inactive. Loops are explicit timeline ranges. The renderer
  sees only the flattened arrangement.
- `[CONFIRMED]` The core ships only primitive devices; instruments, effects,
  and presets are separate packs. Startup never blocks on plugin scanning or
  network.
- `[CONFIRMED]` The UI is `egui` on `eframe`/`winit` (native window,
  cross-platform). It issues commands and holds no document state.

---

## 4. Vocabulary

Names are chosen to avoid collisions. Several words that sound natural are
banned because they already mean something else here.

### 4.1 Control-side names

| Name | Is | Job |
| --- | --- | --- |
| `Project` | data, serializable | Root document. Holds graph, assets, tempo, markers, metadata. |
| `Session` | runtime object | Owns a `Project`, applies commands, keeps history, emits events, feeds the engine. Non-realtime. |
| `Command` | data, serializable | Describes one mutation. Applied by the dispatcher. |
| `Operator` | data, serializable | A user-facing action: label, typed params, `run(&Project) -> Vec<Command>`. Versioned. |
| `TargetRef` | data | A node reference in operator parameters; resolved to a `NodeId` at invocation. |
| `Transaction` | runtime | One atomic group of commands; one history entry. |
| `History` | runtime | List of transactions plus an undo cursor. |
| `Event` | data | Notification emitted after a change (for UI and other clients). |
| `NodeRegistry` | runtime | Binds `NodeKind` to its `NodeSchema` and constructor. Lives outside `core`. |
| `NodeSchema` / `ParamSchema` | data | A `NodeKind`'s ports and parameters. Lives in `core`; used for validation, automation, and UI. |

### 4.2 Signal-graph names

| Name | Is | Job |
| --- | --- | --- |
| `Graph` | data | Nodes and edges. The signal flow, stored in the `Project`. |
| `Node` | data | A vertex in the graph. Identified by `NodeId`. |
| `NodeRole` | data | What a node is for: `Track`, `Bus`, `Master`, `Send`, `Folder`, `Input`, `Output`, `Modulator`, `Device`. A role, not a separate type. |
| `NodeKind` | data | The concrete implementation a node uses, for example `builtin.gain` or a plugin id. |
| `Device` | role | A `Node` with role `Device`. Processes audio or MIDI, internal or plugin. Reports its latency. |
| `Rack` | data | An ordered serial chain of device node ids owned by a track or bus node. |
| `Port` | data | An input or output on a node, with a channel layout. |
| `Edge` | data | A connection from one node's output port to another node's input port. |
| `ChannelLayout` | data | Mono, stereo, N-channel; used to check port compatibility. |

Notes:

- `[CONFIRMED]` There is no `Track` type. A track is a `Node` whose role
  is `Track`. This keeps the model uniform and makes folders, buses, and
  sends just roles.
- `[CONFIRMED]` Devices are nodes too. A device is a `Node` whose role is
  `Device`. There is one id space, so `ParamAddress` is uniform.
- `[CONFIRMED]` `Instrument` is not a role. It is a `NodeKind`: a device
  with no audio input that produces audio (or MIDI) from its state.
- `[CONFIRMED]` A `Rack` is a serial chain of device node ids. Parallel and
  sidechain paths use `Edge`s. Racks do not contain subgraphs. `Node.rack`
  is meaningful only on `Track`, `Bus`, and `Folder` nodes; there is no
  parallel chain inside one rack.
- `[PROPOSED]` The `Compiler` expands racks and edges into one flat DAG.

### 4.3 Clips, assets, automation, parameters

| Name | Is | Job |
| --- | --- | --- |
| `Clip` | data | A region on a timeline belonging to a node. |
| `ClipContent` | data | What the clip holds: `Audio` (source range in `Frames`), `Midi` (inline notes plus source range in `Tick`), or `Empty`. |
| `Note` | data | A MIDI note inside a clip: `NoteId`, pitch, clip-local `Tick` start and length, velocity, channel. |
| `NoteId` | data | Identity of a note within the project, used by selection and editing commands. |
| `Section` | data | A reusable named span of clip placements, referenced by the arrangement. |
| `SectionRef` | data | `(section, repeat, active)`. One instance of a `Section` in the arrangement; `active = false` hides it. |
| `LoopRegion` | data | A timeline range with looping enabled. |
| `Asset` | data | Immutable media (audio file, MIDI file, impulse response). |
| `AssetId` | data | Content hash of an asset. |
| `AssetStore` | runtime | Maps `AssetId` to metadata and file location. |
| `AutomationLane` | data | Keyframes that drive one parameter over time. Stored on the owning node under its `ParamId`. |
| `Keyframe` | data | A value at a `Tick`, with a curve. |
| `Modulator` | data | A `Node` that outputs a signal. |
| `ModulationLink` | data | `(source: NodeId, amount)`. Stored on the target node under its `ParamId`; a link from a `Modulator` node to a parameter. |
| `ParamId` | data | A parameter name within a node kind. |
| `ParamAddress` | data | `(NodeId, ParamId)`. The single way to name a parameter everywhere: automation, UI, MIDI learn, scripts. |
| `ParamValue` | data | A numeric value: `Float`, `Int`, `Bool`, or `Enum`. Normalized at plugin boundaries. |
| `ParamKind` | data | How a parameter behaves: `Control` (per block, smoothed) or `Audio` (per sample). |

Notes:

- MIDI notes are stored inline in `ClipContent::Midi` so per-note editing,
  undo, and copy/paste are ordinary document commands. A MIDI asset remains an
  import/export path, not the editable source of truth.
- Automation is "what the value should be". Modulation is "what
  temporarily offsets it". They are separate but share one ownership rule:
  both live on the target node. An `AutomationLane` is explicit keyframes
  under its `ParamId`; a `ModulationLink` is a link from a `Modulator` node
  under the same `ParamId`. Neither can dangle. Both are folded into the
  DAG by the compiler.
- Keyframes carry a curve, so a smooth or linear run is stored as a few
  points, not one per sample. Recording thins the incoming stream to the
  smallest set of keyframes that stays within a tolerance. Placing a
  `Modulator` node is the way to generate a signal on the fly instead of
  storing many points.
- At a parameter the value is assembled in a fixed order: base value, then
  automation (absolute or trim), then summed modulation offsets, then
  clamp. Control parameters use one scalar slot per block; audio-rate
  parameters read a per-sample signal path.
- Values are numeric only. Text and file references are document
  references (`AssetId`) or plugin state, handled by structural commands.
  This keeps automation, modulation, and the realtime parameter buffer
  simple and fixed-size.
- An automation lane combines with the parameter's base value. A bulk
  change such as "make this whole track 2 dB louder" must still work when
  the parameter already has automation, so the lane needs an absolute or
  trim (offset) mode. This is required, not optional.
- `[CONFIRMED]` A `ParamAddress` is only meaningful while its node's kind
  is unchanged. Replacing a device with a different `NodeKind` runs an
  explicit remap in the dispatcher: lanes and MIDI-learn bindings are
  matched by `ParamId` and kept, and unmatched ones are dropped with an
  `Event`. Nothing keeps referring to a parameter that no longer exists.

### 4.4 Time

| Name | Is | Job |
| --- | --- | --- |
| `Tick` | `i64` | Musical time. |
| `Frames` | `i64` | Audio time at the project sample rate. |
| `TempoMap` | data | Breakpoints of tempo. |
| `TempoPoint` | data | `(Tick, microseconds per quarter note)`. |
| `TimeSignature` | data | `(Tick, numerator, denominator)`. |
| `Marker` | data | A named position. |

`[CONFIRMED]` `PPQ` is 983040 (= 960 x 1024). It is an exact multiple of
all common MIDI resolutions (96, 120, 192, 240, 384, 480, 960, 1024) and
of 3 and 5, so triplets and quintuplets are exact.

`[CONFIRMED]` `Tick -> Frames` is a pure function of `(TempoMap, Tick)`.
Each tempo segment stores an exact anchor (`anchor_tick`, `anchor_frames`,
and the integer remainder at the anchor) at its `TempoPoint`. Within a
segment the conversion is closed-form, computed with `i128` from the anchor
and the segment's integer microseconds-per-quarter-note. Anchors are
recomputed on the control side whenever the tempo map changes. No stateful
"running remainder" is carried across calls, so seeking and any block size
produce identical results.

### 4.5 Render-side names (realtime)

| Name | Is | Job |
| --- | --- | --- |
| `Compiler` | runtime | `Project` -> `RenderPlan`. Non-realtime. |
| `RenderPlan` | data, immutable | Flat, sorted, preallocated schedule of processing steps. |
| `Engine` | runtime | Owns the realtime executor, the transport, and the device I/O. |
| `DeviceProcessor` | runtime | Plan-driven processing impl for a device; in-process or a shared-memory proxy. |
| `Scheduler` | runtime | Converts `Tick` to `Frames` and dispatches events per block. |
| `Buffer` | data | Preallocated audio memory. |
| `Block` | data | A slice of frames plus the context for one processing step. |
| `Transport` | runtime | Runtime play state (playing, position, loop, record). |

`[PROPOSED]` The playhead and recording state live in `Transport` on the
render side, not in the `Project`. The `Project` stores arrangement,
tempo, markers and loop range.

### 4.6 Platform / edge names

| Name | Is | Job |
| --- | --- | --- |
| `AudioBackend` | runtime | Sound card I/O. Wraps `cpal`. |
| `MidiBackend` | runtime | MIDI I/O. Wraps `midir`. |
| `Window` | runtime | Native window and event loop. `winit` + `egui`. |
| `Clock` | runtime | Monotonic time for non-audio use. Never used in the core. |
| `Decoder` / `Encoder` | runtime | File decode/encode (WAV, FLAC, and so on). |
| `Resampler` | runtime | Sample-rate conversion: assets on import, master output at the platform edge. |
| `Container` | runtime | Reads and writes `song.odaw/`. Crash-safe save: temp dir, fsync, atomic replace. |
| `PluginHost` | runtime | Scans and instantiates CLAP plugins. Non-realtime load; devices cross into render. |
| `ScriptHost` | runtime | Runs control/compile-time Lua. Never on the audio thread. |
| `PeakCache` | runtime | Waveform pyramid for display. Disposable. |

### 4.7 Banned or reserved words

- Bare "Processor" — ambiguous (command processing vs DSP). Use `Session`
  for the command side and `Device` for the DSP side. `DeviceProcessor` is
  the one permitted qualified form, because it names the plan-driven
  implementation of a device unambiguously.
- "Action" — overloaded. Use `Command`.
- "Plugin" — means external loadable code only. An internal DSP unit is a
  `Device`, not a plugin.
- "Track type" — there are no track types, only `NodeRole`s.
- "Editor" — a UI view. Never a document object.

---

## 5. Where things live

`[PROPOSED]` Cargo workspace. Dependencies point inward, toward `core`.
`core` must compile with no audio backend, no FFI, and no UI.

```
odaw/
├── core/       Project, Graph, Node, Clip, Asset, Command, time, params,
│               NodeSchema/ParamSchema. Pure data and logic. No platform
│               code, no FFI, no UI. Does not know device constructors.
├── format/     Versioned DTOs, container read/write, migrations.
├── compile/    Compiler: Project -> RenderPlan. Non-realtime; allocates.
├── engine/     Engine, RenderPlan, Scheduler, executor. Realtime.
├── control/    Session, History, Tx, dispatcher, operators. Non-realtime.
├── dsp/        Devices implemented in-tree. SIMD, resampling. Realtime.
├── io/         Decoders and encoders.
├── platform/   AudioBackend (cpal), MidiBackend (midir), Clock, FS.
│               The only crate that is allowed platform-specific code.
├── plugins/    CLAP adapter behind the Device abstraction. Non-realtime
│               scan/load; devices cross into render.
├── protocol/   Control API trait, IPC/WebSocket adapters, schemas.
├── script/     Control/compile-time Lua host, sandbox, command bridge.
│               Never runs on the audio thread.
├── render/     Offline renderer. Uses the engine without realtime limits.
├── ui/         egui frontend. Replaceable. Holds no document state.
└── apps/
    ├── odaw-cli/    `odaw render`, `odaw analyze`, `odaw run`.
    └── odaw-app/    Native app: UI + control + engine.
```

Dependency rule: `core` depends on nothing in this list. `format` depends
only on `core`. Everything else depends inward. No crate except
`platform` and the plugin-GUI adapter may contain `cfg(target_os)`.

`core` holds `NodeSchema`/`ParamSchema` as data but not device
constructors. `NodeRegistry` binds a `NodeKind` to its schema and its
constructor in `engine`/`dsp`, so `core` never depends on device code. The
control side (`Session`, `History`, `Tx`, dispatcher, operators) lives in
`control`, which depends on `core`, `compile`, and `engine`. The
realtime-safety check covers `engine`/`dsp` only; `control`, `compile`, and
`format` may allocate.

---

## 6. Ownership and trust boundaries

Three worlds. Data crossing a boundary must obey the boundary's rules.

1. Document world (non-realtime): `Project`, `Session`, `Command`,
   `History`, `format`, scripting host.
2. Render world (realtime): `RenderPlan`, `Engine`, `Device`s, `Buffer`.
3. Platform world (edges): `AudioBackend`, `MidiBackend`, `Window`.

Rules:

- The document world never runs on the audio thread.
- The render world never allocates, locks, does I/O, or calls scripts.
- The only objects that cross document -> render are owned render-side state
  (plan, live device instances, parameter snapshot) and parameter values on a
  lock-free command queue.
- Render-side state is handed over as a whole over a bounded lock-free queue.
  The audio thread swaps it at a block boundary; it never locks, waits,
  allocates, or frees. The outgoing state is returned on a second queue and is
  destroyed by the control thread.
- Asset blobs live in the `AssetStore` outside the immutable `Project`;
  snapshots reference them by `AssetId`. Eviction must not free a blob that
  a live snapshot still references. See section 2.10.
- Recompilation may read device metadata but never mutates a live device
  instance. The control side builds and prepares replacement instances off
  the audio thread; a live instance is only touched on the audio thread, at
  the handoff point, where the outgoing instance is retired.
- The only things that cross render -> document are lock-free feedback
  (transport position and finished state) and messages on a lock-free queue
  (meters, events).
- Value commands update both the document (for undo) and the parameter
  buffer (for realtime). Parameter values reach the render side through the
  same lock-free command queue and are applied to stable slots.

Ownership summary:

| Object | Owned by | Lifetime |
| --- | --- | --- |
| `Project` | `Session` | project |
| `History` | `Session` | project |
| `RenderPlan` | `Engine` (current realtime state, built by control) | replaced on rebuild |
| `param_buffer` | `Engine` (render side), updated from control over a lock-free queue | application |
| `Transport` | `Engine` | application |
| `AssetStore` | `Session` | project |
| `AudioBackend`, `Window` | app | application |

---

## 7. Platform notes

- Platform-specific code is confined to `platform/` and the plugin-GUI
  adapter. Nothing else sees `cfg(target_os)`.
- On Linux, X11 and Wayland are a runtime choice handled by `winit`.
  XWayland is acceptable and requires no separate build.
- The only places that genuinely differ per platform are: audio backend,
  MIDI backend, plugin-GUI embedding, and file dialogs.
- `[OPEN]` Plugin GUI embedding as child windows is painful on Wayland
  (CLAP). Floating plugin windows first; embedding later.

---

## 8. Decisions and remaining questions

Resolved (items already stated in section 3 are not repeated):

1. `[CONFIRMED]` Container layout: `song.odaw/` containing
   `manifest.json`, `project.postcard`, `assets/<hash>.*`, and a
   disposable `cache/`. There is no stored `project.json` mirror.
   Inspection and diffing are done on demand:
   `odaw inspect song.odaw --json`. Saves are crash-safe: write to a
   sibling temp directory, fsync, then atomically replace (avoids the new
   file landing next to the old). See section 4.6.
2. `[CONFIRMED]` Control protocol: JSON-RPC for control, with a separate
   binary channel for bulk data.
3. `[CONFIRMED]` Operators stay, are in-project, and use `TargetRef`. See
   section 2.6.
4. `[CONFIRMED]` Parameter-buffer slots are a stable map keyed by
   `ParamAddress`. A slot is assigned when an address first appears and is
   never moved, invalidated, or reclaimed for the life of the project; a
   plan rebuild does not disturb it. See section 2.8.
5. `[CONFIRMED]` A `History` entry stores the operator invocation
   `(id, version, params)` when present, plus the resulting commands for the
   operation log. See sections 2.4 and 2.6.
6. `[CONFIRMED]` `ChangeSet` is a structured, composable, directionless delta:
   touched-id sets plus structural, layout/latency, meta/tempo, and arrangement
   flags. It records no before/after values. See section 2.7.
7. `[CONFIRMED]` Asset blobs live in `AssetStore` outside `History`, and
   memory has two budgets: structural (transaction count) and asset bytes.
   See sections 2.9 and 2.10.
8. `[CONFIRMED]` Determinism is tiered (core, plugins, realtime), and
   `Meta.sample_rate` is immutable. See section 3.
9. `[CONFIRMED]` `im` provides the persistent collections: `im::OrdMap` for
   id-keyed sets and ordered maps, `im::Vector` for ordered lists. The core
   aliases them as `PersistentMap` and `PersistentVector`, so replacing the
   library later stays localized.
10. `[CONFIRMED]` Selection is control-side state, not document state. It is
    not serialized in the project and is not an undo step. It holds nodes,
    clips, notes, an optional tick range, and a separate focused node; it is
    pruned after edits and undo/redo.
11. `[CONFIRMED]` The default `History` bound is 512 transactions. Asset bytes
    are bounded separately; see section 2.10.
12. `[CONFIRMED]` MIDI notes are inline in the clip and carry `NoteId`s.
    Project format v3 encodes them together with track name, color, mute, and
    solo metadata; v1 and v2 containers migrate with defaults. See section 4.3.
13. `[CONFIRMED]` Realtime engine handoff uses bounded SPSC ring buffers of
    owned render state. The audio thread swaps at block boundaries and returns
    old state on a retirement queue; it never allocates, locks, or frees.
    Parameter updates and transport commands use the same lock-free command
    queue.
14. `[CONFIRMED]` The native UI is `egui` on `eframe`/`winit` and holds no
    document state. Edits go through `Session` commands; selection stays in the
    control layer; structural edits recompile and republish a `RealtimeState`.
    The UI is replaceable by another client of the same command API.
15. `[CONFIRMED]` Track presentation and mix flags (`name`, `color`, `muted`,
    `soloed`) live on `Node` and are serialized in format v3. The compiler
    silences muted tracks and, when any track is soloed, non-soloed tracks.
16. `[CONFIRMED]` Every track, bus, folder, and the master ends in a built-in
    channel strip in the compiled plan. Its `gain_db` and `pan` values are
    ordinary value parameters on the owner node, so a fader preview writes the
    live `ParamBuffer` and release commits one `ParamCommand`; the graph is not
    rebuilt for a level change. The strip is plan-only and never a document
    device, which keeps saved projects free of implicit devices.
17. `[CONFIRMED]` The transport cursor is musical (`Tick`) UI state, not
    document state. Seeking issues `RealtimeHandle::seek` during a scrub and
    republishes a state at the cursor frame on release or play, so note
    scheduling and oscillator phase restart cleanly without recompiling the
    graph. The clip split/duplicate workflow is expressed as ordinary commands,
    so `Ctrl+E` and context-menu splits are undoable.
18. `[CONFIRMED]` Audio-to-UI metering uses a `MeterBank` of atomic peaks shared
    by the compiled channel strips and the realtime handle. Channel strips write
    post-fader L/R peaks per block; the UI reads them lock-free and applies its
    own release decay. The audio thread never allocates, locks, or waits for the
    UI, and the meter bank is replaced together with the engine state.
19. `[CONFIRMED]` The piano roll uses a pointer gesture model rather than a
    draw/select tool mode: a tap on empty grid adds a snap-length note, a drag
    on empty grid marquee-selects notes, and a drag on a selected note moves the
    whole selection. Group transpose, copy/paste, duplicate, and move are
    ordinary note commands, so they undo and replay deterministically.
20. `[CONFIRMED]` Audio clips compile into a per-track `AudioClipPlayer` source
    that is summed at the channel-strip input beside the rack output. Clips stay
    in the document as asset ids plus source `Frames`; decoded interleaved
    samples are runtime data supplied to the compiler, never document state.
    The player reads preloaded samples and linearly resamples to the project
    rate on the audio thread. Missing sources are silent, not a compile error,
    so a project remains loadable while assets are unavailable.
21. `[CONFIRMED]` Imported audio blobs are content-addressed and copied into
    `<container>/assets/<asset-id>.<ext>` after the project transaction is
    written. Reloading finds them by asset id, decodes samples and peaks, and
    feeds the same compiler path as a fresh import.
22. `[CONFIRMED]` The bottom panel is contextual: MIDI clips show the piano
    roll, audio clips show a waveform/detail view plus track options, and a
    track with no selected clip shows track options. The roll remains one
    toggle away for MIDI tracks and the panel stays user-resizable.
23. `[CONFIRMED]` The Time dialog edits the first (global) time signature at
    tick zero, the tempo, and the cursor bar/beat. Bar/beat conversion is
    `((bar - 1) * numerator + (beat - 1)) * PPQ`; the signature numerator also
    drives bar lines and labels in the timeline and piano roll. Per-section
    signature changes remain supported by the document but are not exposed in
    this dialog yet.

Still open:

1. `[OPEN]` How value smoothing is specified: per node, per node kind, or
   one global policy.
2. `[OPEN]` The maximum structural size of one transaction and the asset byte
   budget. See section 2.10.
3. `[OPEN]` The thinning tolerance used when recording automation into
   `Keyframe`s. See section 4.3.
4. `[OPEN]` Whether a VST3 (or AU) adapter is added later behind the same
   `Device` abstraction. CLAP-only means plugins without a CLAP build
   cannot be loaded.

---

Implementation sequencing lives in `LTPLAN.md`, next to this document.

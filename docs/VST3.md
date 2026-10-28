# VST3 backend

VST3 is a plugin backend behind the existing device boundary, not a second
plugin model. `core`, `compile`, `engine`, `control`, and `format` stay
ABI-agnostic; `odaw-vst3` depends on them and is reached through
`odaw_engine::DeviceFactory`.

```
Project -> Compiler (+ Vst3Factory) -> RenderPlan -> Engine
                              |
                              +-- odaw-vst3 scan cache / Plugin / PluginWindow
```

## Boundaries

- `DeviceFactory::build(Node, DeviceContext) -> Box<dyn DeviceProcessor>`
  runs on the control thread. `DeviceContext` carries the node id, sample
  rate, maximum block size, and channel count.
- `Vst3Processor` owns one live `Plugin` behind `Arc<Mutex<Plugin>>` and only
  calls `process_audio` from the audio thread.
- `vst3-host` 0.10 is MIT-licensed, ships pre-generated bindings, and does not
  link the GPLv3 VST3 SDK. Only discovery, plugin loading, parameters, state,
  and window APIs are used; its playback/timeline/cpal layers are disabled.

## Document mapping

| Document | VST3 |
| --- | --- |
| `NodeKind` | `vst3:<32-hex class id>` |
| `Node.params` | `param.<parameter id>` as normalized `Float(0.0..=1.0)` |
| `Node.state` | opaque versioned component + controller state from `Plugin::save_state` |
| rack order | serial processing order |
| track `event_target` | note events for instruments |
| `DeviceProcessor::latency_frames` | `IAudioProcessor::getLatencySamples` |

Plugin parameters are discovered at load time and reported through
`DeviceProcessor::parameter_defaults`, so the compiler creates a stable
`ParamBuffer` slot for every parameter. A slot version moves only when its
value changes; `Vst3Processor` forwards changed slots per block.

On save, the UI captures the live component/controller state and every live
normalized parameter value into a throwaway project snapshot. That snapshot is
what gets written, so edits made in the plugin's own editor survive a reload.
Structural edits use the same snapshot to re-instantiate the plugin with the
current patch. This is the MVP: voices/tails reset on structural edits.

## Scanning

`Catalog::scan` walks the standard Linux paths (`~/.vst3`, `/usr/lib/vst3`,
`/usr/local/lib/vst3`) plus `VST3_PATH`. Bundles with `moduleinfo.json` are
inspected without loading code; older bundles fall back to the host's
introspection. Results are cached in
`~/.cache/odaw/vst3-plugins.json` and invalidated by bundle mtime.

The UI reads the cache at startup and rescans on a worker thread. The CLI does
the same explicitly:

```sh
cargo run -p odaw-cli -- plugins --refresh
cargo run -p odaw-cli -- plugins --json
```

## Plugin chains

A track rack is a serial chain, top to bottom. Instruments (devices with no
audio input) stay first; effects follow in processing order. The UI can add an
effect to the selected track, move devices up/down, and remove them. Removing a
device also removes its node, so the document never leaves an orphaned device.

Every device in the chain is still a normal `Node` in `Rack` order; VST3 is not
special-cased in the compiler. Latency accumulates serially, and the existing
per-edge compensation aligns parallel tracks. Effects can be stacked freely as
long as each one has one main mono/stereo input and output.

Each VST3 device has a **Bypass/Enable** button in its device panel. Bypass is
a host-level parameter stored as `Node.params["bypass"]`: effects pass their
input through untouched, instruments go silent, and entering bypass releases
held notes. It is value-only, so toggling it does not rebuild the graph, and it
persists and undoes like any other parameter.

## Persistent plugin state

`Node.state` is the canonical patch. Three things write it:

- **Save** captures component/controller state and current normalized parameter
  values into the document before writing the container.
- **Close editor** and the **Save state** button do the same without saving the
  project, so in-memory edits and undo see the current patch.
- **Structural rebuilds** capture state into a throwaway snapshot, so
  re-instantiating a plugin for a rack/clip edit keeps the current patch.

The capture is one history entry labelled `Update plugin state`. Undo/redo
load the document snapshot and deliberately do *not* overwrite it with the live
plugin patch. Voices and tails still reset when a structural edit
re-instantiates the plugin; instance retention remains future work.

## Audio and events

- one main audio input bus (effects) and one main output bus; mono or stereo;
- multi-out, sidechains, and extra active buses are rejected with a clear error;
- note on/off events are sample-offset per block; pitch bend, CC, MPE, and note
  expression are later work;
- transport position is forwarded every block (`projectTimeSamples`);
- PDC inserts compensating delays on parallel branches. The master's own
  latency remains the overall output delay.

The current path is in-process and not allocation-free under MIDI load:
`vst3-host` queues events and updates metering with short locks. The engine
thread never loads, scans, or does file I/O, but process isolation and the
lock-free runner are follow-up hardening work.

## Editors

The UI opens a standalone `PluginWindow` (X11/XWayland on Linux), services it
once per frame, and does not embed plugin UIs into egui. Wayland-native
embedding is out of scope. The bundled spike is:

```sh
cargo run -p odaw-vst3 --example editor_spike
```

## Example project

`examples/ode-to-joy.odaw` is a six-track Dexed arrangement of Ode to Joy:

- Lead — PHAROH Dexed program, Hall reverb;
- Harmony — diatonic third below the lead;
- Bass — RUMBLE 1 playing a syncopated root/octave/fifth pattern;
- Bass Bell — offbeat bell figures using the C,D,Eb,F program;
- Pad — Slow3D Pad playing sustained triads, Plate reverb;
- Arp — ARP 2600 playing syncopated eighth-note chord figures with rests.

It is 16 bars at 120 BPM with a loop region and two markers. Regenerate it from
the installed catalogue with:

```sh
cargo run -p odaw-cli -- demo-dexed examples/ode-to-joy.odaw
cargo run -p odaw-cli -- render examples/ode-to-joy.odaw -o ode.wav
```

Program choices are written into each Dexed `Node.state`, so the patch survives
a reload even though the preset index is not a normal parameter.

## Testing

With Dexed installed in `~/.vst3`:

```sh
cargo test -p odaw-vst3
```

The tests skip when the plugin is absent. They cover catalogue metadata, note
processing, state capture/restore, and an offline project render through the
normal compiler.

With Dragonfly Reverb (or any effect) installed:

```sh
cargo test -p odaw-vst3 --test effects
```

It processes an impulse through a real effect, restores its state and
parameters, and renders a project with two stacked effects. Plugin tests take
a process-local mutex because vendor factories and DPF plugins are not safe
under concurrent instantiate/process/teardown. The app performs those
operations on one control thread and one audio thread.

Stacking is also covered without needing an effect plugin:

```sh
cargo test -p odaw-compile effects_stack_in_rack_order
cargo test -p odaw-ui rack_devices_can_be_reordered_and_removed
cargo test -p odaw-ui two_installed_effects_stack_reorder_and_remove_through_the_ui_path
cargo test -p odaw-ui live_plugin_state_is_persisted_and_undoable
```

`effects_stack_in_rack_order` runs two order-sensitive test effects and checks
that the rack is actually serial. For a manual VST3 effect test, install a free
VST3 effect from the distro (for example `dragonfly-reverb-vst3`,
`elephantdsp-roomreverb-vst3`, or `dpf-plugins-vst3`), then:

1. Click **Plugins** → **Rescan**.
2. Add two different effects to the same track.
3. Move the second effect up/down and confirm the sound changes accordingly.
4. Open each editor, change a parameter, click **Save state**.
5. Save, reopen, and confirm both patches and the chain order are restored.

The next step for CI parity is a tiny MIT-licensed test effect bundle built from
the `vst3-rs` bindings, so effect stacking and state round-trips can run without
a user-installed plugin.

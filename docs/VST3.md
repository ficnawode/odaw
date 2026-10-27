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

## Testing

With Dexed installed in `~/.vst3`:

```sh
cargo test -p odaw-vst3
```

The tests skip when the plugin is absent. They cover catalogue metadata, note
processing, state capture/restore, and an offline project render through the
normal compiler.

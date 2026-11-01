# PipeWire 1.6 notes

## Build baseline

- `pkg-config --modversion libpipewire-0.3` in the dev shell: **1.6.9**.
- The locked nixpkgs revision is `151fa4e8ddfdd8dd25d945ad94ed54a13de9f6e4`; `pkgs.pipewire` from that pin resolves to 1.6.9.
- `build.rs` now rejects libpipewire below **1.6.6**.
- No PipeWire source or POD builder is vendored.

## Thread loop and ring

`native/pipewire_sink.c` uses `pw_thread_loop_new` and the public
`pw_thread_loop_lock/unlock/start/stop` API directly. It does not layer
epoll/eventfd plumbing on top of PipeWire.

The PCM ring is a separate lock-free SPSC structure. Its atomics do not depend
on the thread-loop lock, and the real-time `process` callback does not take a
Rust or thread-loop mutex. PipeWire 1.6's priority-inversion-aware loop locking
is therefore used only by the control path (`pw_stream_set_active`).

## POD building

The format negotiation uses `SPA_POD_BUILDER_INIT` and
`spa_format_audio_raw_build` from the linked 1.6.9 headers. There is no
alternate or vendored builder.

The sink currently declares one format: S16LE, stereo, 48 kHz.

## Capability Params

`pipewire/capabilities.h` exposes capability names such as
`pipewire.device-id-negotiation` and `pipewire.device-ids`.
`SPA_PARAM_Capability` / `SPA_PARAM_PeerCapability` are transport/device
capability negotiation, not a replacement for `SPA_PARAM_EnumFormat` for a
fixed audio sink.

Musika has a single fixed output format and no alternate-format or device-id
fallback to simplify. Capability Params are therefore not adopted for format
negotiation in this pass. This can be revisited if Musika gains selectable
devices, channel layouts or sample formats.

## Resampling ownership

`native/pipewire_decode.c` resamples every decoded frame with
`swr_alloc_set_opts2` to S16LE, stereo, 48 kHz. The PipeWire stream is declared
with the same fixed rate. PipeWire's graph resampler is not in the normal
playback path, so no PipeWire resampler property is required.

If the sink later negotiates the source rate directly, this ownership changes
and the graph resampler becomes relevant.

## Future: multichannel

Supporting 5.1 or other layouts would require:

- a channel-layout setting carried from player/queue configuration;
- replacing the fixed `MUSIKA_CHANNELS`/stereo assumptions in the sink and
  decoder resampler;
- using `info.position`/`audio.layout` rather than only `info.channels`;
- updating row/queue metadata and the session settings surface.

This is a feature, not part of the 1.6 upgrade.

## Future: exclusive and reliable nodes

`PW_KEY_NODE_EXCLUSIVE` and `PW_KEY_NODE_RELIABLE` exist in PipeWire 1.6.
Neither is enabled:

- exclusive hardware access would conflict with other applications and must
  not be a default;
- reliable transport is aimed at lossy/remote links and is not needed for the
  current small in-memory playback sink.

Both would need a user-facing setting and a separate design/PR.

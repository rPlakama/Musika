//! Native playback transport.
//!
//! The sink, FFmpeg decoder, workers and clock are all Rust-owned; the only C
//! code left in this path is the external system `libpipewire`/`libav*` code
//! reached through the `pipewire` and `ffmpeg-next` bindings.

mod decode;
mod native;
mod ring;
mod sink;
mod status;
mod worker;

pub(crate) use native::Native;
pub(crate) use status::SinkState;

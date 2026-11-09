//! Rust PipeWire output sink.
//!
//! A dedicated thread owns every `!Send` PipeWire object and runs the main
//! loop. The decoder thread writes PCM into [`Ring`]; the RT process callback
//! drains it into the negotiated S16LE/stereo/48 kHz stream. Controls and
//! shutdown are delivered through a PipeWire channel processed on the loop
//! thread, so no PipeWire object crosses threads.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pipewire as pw;
use pw::spa::sys as spa_sys;
use pw::{properties::properties, spa};
use spa::pod::Pod;

use super::ring::{self, Ring};

enum SinkCommand {
    SetPaused(bool),
    Shutdown,
}

pub(crate) struct Sink {
    ring: Arc<Ring>,
    commands: pw::channel::Sender<SinkCommand>,
    thread: Option<JoinHandle<()>>,
}

impl Sink {
    pub(crate) fn new() -> Result<Self, String> {
        let ring = Arc::new(Ring::new()?);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let (command_tx, command_rx) = pw::channel::channel::<SinkCommand>();
        let thread_ring = Arc::clone(&ring);
        let thread = thread::Builder::new()
            .name("musika-pipewire".to_string())
            .spawn(move || sink_thread(thread_ring, command_rx, ready_tx))
            .map_err(|error| format!("cannot start PipeWire thread: {error}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                ring,
                commands: command_tx,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(error) => {
                let _ = thread.join();
                Err(format!("PipeWire sink did not start: {error}"))
            }
        }
    }

    pub(crate) fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.ring.epoch()
    }

    pub(crate) fn write(&self, pcm: &[i16], epoch: u64) {
        self.ring.write(pcm, epoch);
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.ring.set_paused(paused);
        let _ = self.commands.send(SinkCommand::SetPaused(paused));
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.ring.set_muted(muted);
    }

    pub(crate) fn set_volume(&self, volume: f64) {
        self.ring.set_volume(volume);
    }

    pub(crate) fn flush(&self) {
        self.ring.flush();
    }

    pub(crate) fn reset(&self) {
        self.ring.reset();
    }

    pub(crate) fn close_writes(&self) {
        self.ring.close_writes();
    }

    pub(crate) fn played_frames(&self) -> u64 {
        self.ring.played_frames()
    }

    pub(crate) fn buffered_frames(&self) -> u64 {
        self.ring.buffered_frames()
    }

    pub(crate) fn failed(&self) -> bool {
        self.ring.failed()
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.ring.close_writes();
        self.ring.wake();
        let _ = self.commands.send(SinkCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn sink_thread(
    ring: Arc<Ring>,
    commands: pw::channel::Receiver<SinkCommand>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let result = run_sink(ring, commands, ready.clone());
    if let Err(error) = result {
        let _ = ready.send(Err(error));
    }
}

fn run_sink(
    ring: Arc<Ring>,
    commands: pw::channel::Receiver<SinkCommand>,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|error| format!("cannot create PipeWire main loop: {error}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|error| format!("cannot create PipeWire context: {error}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|error| format!("cannot connect to PipeWire: {error}"))?;
    let props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "Musika",
    };
    let stream = pw::stream::StreamBox::new(&core, "Musika", props)
        .map_err(|error| format!("cannot create PipeWire stream: {error}"))?;

    let listener_ring = Arc::clone(&ring);
    let _listener = stream
        .add_local_listener_with_user_data(listener_ring)
        .state_changed(|_, ring, _old, new| {
            if matches!(new, pw::stream::StreamState::Error(_)) {
                ring.set_failed(true);
            }
        })
        .process(|stream, ring| process(stream, ring))
        .register()
        .map_err(|error| format!("cannot register PipeWire listener: {error}"))?;

    let stream_ptr = stream.as_raw_ptr() as usize;
    let command_loop = mainloop.clone();
    let command_stream = stream_ptr;
    let _command_listener = commands.attach(mainloop.loop_(), move |command| match command {
        SinkCommand::SetPaused(paused) => unsafe {
            pw::sys::pw_stream_set_active(command_stream as *mut pw::sys::pw_stream, !paused);
        },
        SinkCommand::Shutdown => {
            command_loop.quit();
        }
    });

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::S16LE);
    audio_info.set_rate(ring::RATE);
    audio_info.set_channels(ring::CHANNELS as u32);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa_sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa_sys::SPA_AUDIO_CHANNEL_FR;
    audio_info.set_position(position);
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa_sys::SPA_TYPE_OBJECT_Format,
            id: spa_sys::SPA_PARAM_EnumFormat,
            properties: audio_info.into(),
        }),
    )
    .map_err(|error| format!("cannot build PipeWire format: {error}"))?
    .0
    .into_inner();
    let pod = Pod::from_bytes(&values).ok_or_else(|| "invalid PipeWire format pod".to_string())?;
    let mut params = [pod];

    stream
        .connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|error| format!("cannot connect PipeWire stream: {error}"))?;

    // Signal readiness only after the stream is connected. The caller may flip
    // pause/volume immediately; commands are queued on the loop channel.
    let _ = ready.send(Ok(()));
    mainloop.run();
    drop(stream);
    Ok(())
}

fn process(stream: &pw::stream::Stream, ring: &Arc<Ring>) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let requested = buffer.requested() as usize;
    let datas = buffer.datas_mut();
    if datas.is_empty() {
        return;
    }
    let data = &mut datas[0];
    let Some(bytes) = data.data() else {
        return;
    };
    let max_frames = bytes.len() / (ring::CHANNELS * std::mem::size_of::<i16>());
    let frames = if requested > 0 {
        requested.min(max_frames)
    } else {
        max_frames
    };
    if frames == 0 {
        return;
    }
    let ptr = bytes.as_mut_ptr() as *mut i16;
    if (ptr as usize) % std::mem::align_of::<i16>() != 0 {
        return;
    }
    // SAFETY: the PipeWire buffer is at least `max_frames * 2` samples long and
    // is writable for the duration of the process callback.
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, frames * ring::CHANNELS) };
    let paused = ring.paused();
    // A deactivated PipeWire stream can still get a final/settling process
    // callback. Do not consume or advance the clock while paused; service a
    // pending reset/flush with a zero-frame read so the producer can wake.
    if paused {
        let _ = ring.read_into(out, 0);
        out.fill(0);
    } else {
        let written = ring.read_into(out, frames);
        out[written * ring::CHANNELS..].fill(0);
        ring::apply_gain(out, ring.muted(), ring.volume_milli(), frames);
    }
    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = (ring::CHANNELS * std::mem::size_of::<i16>()) as i32;
    *chunk.size_mut() = (frames * ring::CHANNELS * std::mem::size_of::<i16>()) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn sink_starts_and_consumes_pcm() {
        let sink = Sink::new().expect("PipeWire sink");
        sink.set_volume(0.25);
        let epoch = sink.epoch();
        let pcm = vec![0i16; 4_800 * ring::CHANNELS];
        sink.write(&pcm, epoch);
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(3) {
            if sink.played_frames() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!sink.failed(), "PipeWire sink reported a stream error");
        assert!(sink.played_frames() > 0, "sink consumed no frames");
    }
}

#[cfg(test)]
mod pause_tests {
    use super::*;

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn paused_sink_does_not_advance_played_frames() {
        let sink = Sink::new().expect("PipeWire sink");
        let epoch = sink.epoch();
        let pcm = vec![0i16; 48_000 * 2 * 2];
        sink.write(&pcm, epoch);
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(2) && sink.played_frames() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let before = sink.played_frames();
        assert!(before > 0, "sink did not start: {:?}", sink.ring().failed());
        sink.set_paused(true);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let after = sink.played_frames();
        eprintln!("paused sink played {before} -> {after}");
        assert!(
            after.saturating_sub(before) < 480,
            "sink advanced {} frames while paused",
            after.saturating_sub(before)
        );
    }
}

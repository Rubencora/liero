//! `liero-audio` — software mixer with a cpal backend.
//!
//! # Architecture
//! ```
//! [game thread]  AudioEngine::play(sound_idx)
//!      │  MPSC channel (PlayCmd)
//!      ▼
//! [cpal audio thread]  Mixer::fill_buffer()
//!      └─ up to MAX_CHANNELS active Channel voices mixed to output stream
//! ```
//!
//! All sounds are stored as mono f32 PCM at 22050 Hz in `liero-data::Tc::sounds`.
//! The cpal stream runs at the device's native sample rate; linear interpolation
//! resamples when the device rate differs from 22050 Hz.
//!
//! On WASM the engine is a no-op stub (Web Audio API integration is future work).

// ── Native (non-WASM) implementation ─────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::{Arc, Mutex};
    use std::sync::mpsc::{self, SyncSender};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use liero_data::SoundSamples;

    const SOURCE_RATE: f64  = 22050.0;
    const MAX_CHANNELS: usize = 16;     // max simultaneous voices
    const MAX_VOLUME: f32   = 0.5;      // master volume (avoids clipping on heavy mix)

    #[derive(Clone)]
    struct Channel {
        samples: Arc<Vec<f32>>,  // shared reference to TC sound data
        pos:     f64,            // read position (float for resampling)
        step:    f64,            // pos increment per output sample
    }

    struct PlayCmd {
        samples: Arc<Vec<f32>>,
        step:    f64,
    }

    struct Mixer {
        rx:       mpsc::Receiver<PlayCmd>,
        channels: Vec<Channel>,
    }

    impl Mixer {
        fn new(rx: mpsc::Receiver<PlayCmd>) -> Self {
            Self { rx, channels: Vec::with_capacity(MAX_CHANNELS) }
        }

        /// Fill one cpal output buffer.  Called from the audio thread.
        fn fill(&mut self, data: &mut [f32]) {
            // Accept all pending play commands.
            while let Ok(cmd) = self.rx.try_recv() {
                if self.channels.len() >= MAX_CHANNELS {
                    // Drop oldest channel to make room.
                    self.channels.remove(0);
                }
                self.channels.push(Channel {
                    samples: cmd.samples,
                    pos:     0.0,
                    step:    cmd.step,
                });
            }

            // Clear output buffer.
            for s in data.iter_mut() { *s = 0.0; }

            // Mix all active channels.
            let mut i = 0;
            while i < self.channels.len() {
                let ch = &mut self.channels[i];
                let len = ch.samples.len();
                let mut done = false;

                for out in data.iter_mut() {
                    if ch.pos >= len as f64 {
                        done = true;
                        break;
                    }
                    // Linear interpolation between two adjacent samples.
                    let idx0 = ch.pos as usize;
                    let idx1 = (idx0 + 1).min(len - 1);
                    let frac = ch.pos - idx0 as f64;
                    let s0 = ch.samples[idx0];
                    let s1 = ch.samples[idx1];
                    *out += (s0 + (s1 - s0) * frac as f32) * MAX_VOLUME;
                    ch.pos += ch.step;
                }

                if done {
                    self.channels.swap_remove(i);
                } else {
                    i += 1;
                }
            }
        }
    }

    /// Native audio engine — owns the cpal stream and send-half of the MPSC channel.
    pub struct AudioEngine {
        /// Shared decoded samples, indexed by sound index (same as `Tc::sounds`).
        samples: Vec<Arc<Vec<f32>>>,
        tx:      SyncSender<PlayCmd>,
        /// Hold the stream alive (dropped when AudioEngine is dropped → stream stops).
        _stream: cpal::Stream,
        /// Output sample rate (for resampling calculation).
        out_rate: f64,
    }

    impl AudioEngine {
        /// Initialise cpal and start the audio thread.
        ///
        /// `sounds` should be `tc.sounds.iter().map(|v| Arc::new(v.clone())).collect()`.
        pub fn new(sounds: Vec<SoundSamples>) -> anyhow::Result<Self> {
            let host   = cpal::default_host();
            let device = host.default_output_device()
                .ok_or_else(|| anyhow::anyhow!("no default audio output device"))?;
            let config = device.default_output_config()?;
            let out_rate = config.sample_rate().0 as f64;

            let (tx, rx) = mpsc::sync_channel::<PlayCmd>(256);

            let mixer = Arc::new(Mutex::new(Mixer::new(rx)));

            let ch_count = config.channels() as usize;

            let stream = {
                let mixer = Arc::clone(&mixer);
                device.build_output_stream(
                    &config.into(),
                    move |data: &mut [f32], _info| {
                        let mono_len = data.len() / ch_count;
                        let mut mono_buf = vec![0f32; mono_len];
                        mixer.lock().unwrap().fill(&mut mono_buf);
                        // Copy mono to all output channels (interleaved).
                        for (frame, &m) in data.chunks_mut(ch_count).zip(mono_buf.iter()) {
                            for s in frame.iter_mut() { *s = m; }
                        }
                    },
                    |err| eprintln!("[liero-audio] stream error: {err}"),
                    None,
                )?
            };

            stream.play()?;

            let arc_samples: Vec<Arc<Vec<f32>>> = sounds
                .into_iter()
                .map(Arc::new)
                .collect();

            Ok(Self {
                samples: arc_samples,
                tx,
                _stream: stream,
                out_rate,
            })
        }

        /// Enqueue sound `idx` for immediate playback.  Non-blocking; silently
        /// drops the command if the channel is full or the index is out of range.
        pub fn play(&self, idx: usize) {
            if let Some(samples) = self.samples.get(idx) {
                if samples.is_empty() { return; }
                let step = SOURCE_RATE / self.out_rate;
                let _ = self.tx.try_send(PlayCmd {
                    samples: Arc::clone(samples),
                    step,
                });
            }
        }
    }
}

// ── WASM stub ─────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod native {
    use liero_data::SoundSamples;

    /// No-op stub for WASM (Web Audio API integration is future work).
    pub struct AudioEngine;

    impl AudioEngine {
        pub fn new(_sounds: Vec<SoundSamples>) -> anyhow::Result<Self> {
            Ok(Self)
        }
        pub fn play(&self, _idx: usize) {}
    }
}

// ── Public re-export ──────────────────────────────────────────────────────────

pub use native::AudioEngine;

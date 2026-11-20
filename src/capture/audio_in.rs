//! Sound for recordings: the microphone and the computer's own sound (WASAPI loopback through cpal), mixed into one
//! 48 kHz stereo stream on the wall clock (loopback sends nothing during silence, so gaps are filled with silence).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::media::player::lock;

pub const RATE: u32 = 48_000;
/// Most sound kept waiting per source (seconds) before the oldest is dropped.
const MAX_BACKLOG_SECS: usize = 1;

struct SourceBuf {
    raw: Mutex<VecDeque<f32>>,
    rate: u32,
    channels: usize,
}

impl SourceBuf {
    fn push(&self, data: impl Iterator<Item = f32>) {
        let mut q = lock(&self.raw);
        q.extend(data);
        let max = self.rate as usize * self.channels * MAX_BACKLOG_SECS * 2;
        while q.len() > max {
            q.pop_front();
        }
    }
}

/// One source converted to 48 kHz stereo.
struct Source {
    buf: Arc<SourceBuf>,
    ready: VecDeque<(f32, f32)>,
    frac: f64,
    prev: (f32, f32),
}

impl Source {
    fn convert(&mut self) {
        let raw: Vec<f32> = lock(&self.buf.raw).drain(..).collect();
        let ch = self.buf.channels.max(1);
        let step = f64::from(self.buf.rate) / f64::from(RATE);
        for frame in raw.chunks_exact(ch) {
            let cur = if ch == 1 { (frame[0], frame[0]) } else { (frame[0], frame[1]) };
            // Linear resampling: emit output samples that fall between prev and cur.
            while self.frac <= 1.0 {
                let t = self.frac as f32;
                self.ready.push_back((self.prev.0 + (cur.0 - self.prev.0) * t, self.prev.1 + (cur.1 - self.prev.1) * t));
                self.frac += step;
            }
            self.frac -= 1.0;
            self.prev = cur;
        }
        let max = RATE as usize * MAX_BACKLOG_SECS / 2;
        while self.ready.len() > max {
            self.ready.pop_front();
        }
    }
}

pub struct Capture {
    _streams: Vec<cpal::Stream>,
    sources: Vec<Source>,
    pub level: Arc<AtomicU32>,
    pub notes: Vec<String>,
}

fn start_source(device: &cpal::Device, loopback: bool) -> Result<(cpal::Stream, Arc<SourceBuf>), String> {
    let config = if loopback { device.default_output_config() } else { device.default_input_config() }
        .map_err(|e| e.to_string())?;
    let buf = Arc::new(SourceBuf { raw: Mutex::new(VecDeque::new()), rate: config.sample_rate(), channels: usize::from(config.channels()) });
    let b = Arc::clone(&buf);
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(config.config(), move |d: &[f32], _| b.push(d.iter().copied()), |e| eprintln!("rimelight-media: sound capture: {e}"), None),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config.config(),
            move |d: &[i16], _| b.push(d.iter().map(|s| f32::from(*s) / 32768.0)),
            |e| eprintln!("rimelight-media: sound capture: {e}"),
            None,
        ),
        cpal::SampleFormat::I32 => device.build_input_stream(
            config.config(),
            move |d: &[i32], _| b.push(d.iter().map(|s| *s as f32 / 2_147_483_648.0)),
            |e| eprintln!("rimelight-media: sound capture: {e}"),
            None,
        ),
        other => return Err(format!("unsupported sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, buf))
}

impl Capture {
    /// Starts the chosen sources. A source that can't start is left out and noted.
    pub fn start(system: bool, mic: bool) -> Capture {
        let host = cpal::default_host();
        let mut streams = Vec::new();
        let mut sources = Vec::new();
        let mut notes = Vec::new();
        let mut add = |device: Option<cpal::Device>, loopback: bool, name: &str| match device {
            Some(d) => match start_source(&d, loopback) {
                Ok((s, buf)) => {
                    streams.push(s);
                    sources.push(Source { buf, ready: VecDeque::new(), frac: 0.0, prev: (0.0, 0.0) });
                }
                Err(e) => notes.push(format!("{name}: {e}")),
            },
            None => notes.push(format!("{name}: no device")),
        };
        if system {
            add(host.default_output_device(), true, "Computer sound");
        }
        if mic {
            add(host.default_input_device(), false, "Microphone");
        }
        Capture { _streams: streams, sources, level: Arc::new(AtomicU32::new(0)), notes }
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// The next `n` frames of the mix (left and right planes), silence where a source has nothing.
    pub fn pull(&mut self, n: usize) -> (Vec<f32>, Vec<f32>) {
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        for s in &mut self.sources {
            s.convert();
            for i in 0..n {
                let Some((a, b)) = s.ready.pop_front() else { break };
                l[i] += a;
                r[i] += b;
            }
        }
        let mut peak = 0.0f32;
        for v in l.iter_mut().chain(r.iter_mut()) {
            *v = v.clamp(-1.0, 1.0);
            peak = peak.max(v.abs());
        }
        let old = f32::from_bits(self.level.load(Relaxed));
        self.level.store(peak.max(old * 0.85).to_bits(), Relaxed);
        (l, r)
    }
}

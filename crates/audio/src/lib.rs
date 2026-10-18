mod backend;
mod buffer;
mod decode;
pub mod mixer;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

pub use buffer::AudioBuffer;
pub use decode::{decode, output_sample_rate, DecodeError};

#[derive(Debug, Clone)]
pub enum AudioEvent {
    Decoded {
        request: u32,
        buffer: u32,
        error: Option<String>,
    },
    VoiceEnded {
        voice: u32,
    },
}

pub const BLOCK_FRAMES: usize = 512;
const ANALYSER_FRAMES: usize = 32768;

struct AnalyserTap {
    samples: Vec<f32>,
    write: usize,
}

impl AnalyserTap {
    fn new() -> Self {
        Self {
            samples: vec![0.0; ANALYSER_FRAMES],
            write: 0,
        }
    }

    fn push(&mut self, out: &[f32]) {
        for frame in out.as_chunks::<2>().0 {
            self.samples[self.write] = (frame[0] + frame[1]) * 0.5;
            self.write = (self.write + 1) % self.samples.len();
        }
    }

    fn read(&self, out: &mut [f32]) -> usize {
        let count = out.len().min(self.samples.len());
        let start = (self.write + self.samples.len() - count) % self.samples.len();
        for (index, sample) in out[..count].iter_mut().enumerate() {
            *sample = self.samples[(start + index) % self.samples.len()];
        }
        count
    }
}

pub struct Engine {
    mixer: Mutex<mixer::Mixer>,
    analyser: Mutex<AnalyserTap>,
    events: Mutex<Vec<AudioEvent>>,
    clock: AtomicU64,
    sample_rate: u32,
    paused: AtomicBool,
    silent: AtomicBool,
    silent_start: OnceLock<Instant>,
}

impl Engine {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            mixer: Mutex::new(mixer::Mixer::new(sample_rate)),
            analyser: Mutex::new(AnalyserTap::new()),
            events: Mutex::new(Vec::new()),
            clock: AtomicU64::new(0),
            sample_rate: sample_rate.max(1),
            paused: AtomicBool::new(false),
            silent: AtomicBool::new(false),
            silent_start: OnceLock::new(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn set_silent(&self) {
        self.silent.store(true, Ordering::Relaxed);
        let _ = self.silent_start.set(Instant::now());
    }

    pub fn current_time(&self) -> f64 {
        self.advance_silent();
        self.clock.load(Ordering::Relaxed) as f64 / self.sample_rate as f64
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn push_event(&self, event: AudioEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    pub fn take_events(&self) -> Vec<AudioEvent> {
        self.advance_silent();
        self.events
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default()
    }

    fn advance_silent(&self) {
        if !self.silent.load(Ordering::Relaxed) {
            return;
        }
        let Some(start) = self.silent_start.get() else {
            return;
        };
        let target = (start.elapsed().as_secs_f64() * self.sample_rate as f64) as u64;
        let mut clock = self.clock.load(Ordering::Relaxed);
        if target <= clock {
            return;
        }
        const MAX_ADVANCE: u64 = (BLOCK_FRAMES as u64) * 200;
        if target.saturating_sub(clock) > MAX_ADVANCE {
            clock = target - MAX_ADVANCE;
            self.clock.store(clock, Ordering::Relaxed);
        }
        if self.paused.load(Ordering::Relaxed) {
            self.clock.store(target, Ordering::Relaxed);
            return;
        }
        let mut scratch = [0.0f32; BLOCK_FRAMES * 2];
        let mut remaining = (target - clock) as usize;
        while remaining > 0 {
            let frames = remaining.min(BLOCK_FRAMES);
            self.render(&mut scratch[..frames * 2]);
            remaining -= frames;
        }
    }

    pub fn render(&self, out: &mut [f32]) {
        if self.paused.load(Ordering::Relaxed) {
            out.fill(0.0);
            return;
        }
        let base = self.clock.load(Ordering::Relaxed);
        let finished = {
            let mut mixer = match self.mixer.lock() {
                Ok(mixer) => mixer,
                Err(_) => {
                    out.fill(0.0);
                    return;
                }
            };
            mixer.render(out, base);
            mixer.take_finished()
        };
        self.clock
            .fetch_add((out.len() / 2) as u64, Ordering::Relaxed);
        self.tap_analyser(out);
        if !finished.is_empty() {
            if let Ok(mut events) = self.events.lock() {
                events.extend(
                    finished
                        .into_iter()
                        .map(|voice| AudioEvent::VoiceEnded { voice }),
                );
            }
        }
    }

    fn with_mixer<T>(&self, f: impl FnOnce(&mut mixer::Mixer) -> T) -> Option<T> {
        self.mixer.lock().ok().map(|mut mixer| f(&mut mixer))
    }

    fn tap_analyser(&self, out: &[f32]) {
        if let Ok(mut analyser) = self.analyser.lock() {
            analyser.push(out);
        }
    }

    fn read_analyser(&self, out: &mut [f32]) -> usize {
        self.analyser
            .lock()
            .map_or(0, |analyser| analyser.read(out))
    }
}

struct DecodeJob {
    request: u32,
    bytes: Arc<[u8]>,
}

fn spawn_decode_workers(shared: &Arc<Shared>) -> Option<Sender<DecodeJob>> {
    let workers = std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().div_ceil(2))
        .unwrap_or(2)
        .clamp(1, 4);
    let (sender, receiver) = channel::<DecodeJob>();
    let queue = Arc::new(Mutex::new(receiver));
    let mut spawned = 0;
    for index in 0..workers {
        let queue = Arc::clone(&queue);
        let shared = Arc::clone(shared);
        match std::thread::Builder::new()
            .name(format!("kaplay-audio-decode-{index}"))
            .spawn(move || decode_worker(&shared, &queue))
        {
            Ok(_) => spawned += 1,
            Err(error) => log::error!("failed to start the audio decode worker: {error}"),
        }
    }
    (spawned > 0).then_some(sender)
}

fn decode_worker(shared: &Arc<Shared>, queue: &Mutex<Receiver<DecodeJob>>) {
    loop {
        let job = match queue.lock() {
            Ok(queue) => queue.recv(),
            Err(_) => return,
        };
        let Ok(job) = job else {
            return;
        };
        decode_job(shared, job);
    }
}

fn decode_job(shared: &Arc<Shared>, job: DecodeJob) {
    let (buffer, error) = match decode::decode(&job.bytes) {
        Ok(buffer) => {
            let id = shared.next_buffer.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut buffers) = shared.buffers.lock() {
                buffers.insert(id, Arc::new(buffer));
            }
            (id, None)
        }
        Err(error) => (0, Some(error.to_string())),
    };
    shared.engine.push_event(AudioEvent::Decoded {
        request: job.request,
        buffer,
        error,
    });
}

struct Shared {
    engine: Arc<Engine>,
    buffers: Mutex<HashMap<u32, Arc<AudioBuffer>>>,
    next_buffer: AtomicU32,
}

impl Shared {
    fn new(engine: Arc<Engine>) -> Self {
        Self {
            engine,
            buffers: Mutex::new(HashMap::new()),
            next_buffer: AtomicU32::new(1),
        }
    }
}

pub struct Audio {
    shared: Arc<Shared>,
    decode: Option<Sender<DecodeJob>>,
    _backend: backend::Handle,
    has_device: bool,
}

impl Audio {
    pub fn new() -> Self {
        let sample_rate = backend::preferred_sample_rate().unwrap_or_else(output_sample_rate);
        let engine = Arc::new(Engine::new(sample_rate));
        let (handle, has_device) = match backend::start(engine.clone()) {
            Some(handle) => (handle, true),
            None => {
                engine.set_silent();
                (backend::silent(), false)
            }
        };
        Self::with_engine(engine, handle, has_device)
    }

    pub fn silent() -> Self {
        let engine = Arc::new(Engine::new(output_sample_rate()));
        engine.set_silent();
        Self::with_engine(engine, backend::silent(), false)
    }

    fn with_engine(engine: Arc<Engine>, handle: backend::Handle, has_device: bool) -> Self {
        let shared = Arc::new(Shared::new(engine));
        let decode = spawn_decode_workers(&shared);
        Self {
            shared,
            decode,
            _backend: handle,
            has_device,
        }
    }

    pub fn has_device(&self) -> bool {
        self.has_device
    }

    pub fn sample_rate(&self) -> u32 {
        self.shared.engine.sample_rate()
    }

    pub fn current_time(&self) -> f64 {
        self.shared.engine.current_time()
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.engine.set_paused(paused);
    }

    pub fn take_events(&self) -> Vec<AudioEvent> {
        self.shared.engine.take_events()
    }

    pub fn read_analyser(&self, out: &mut [f32]) -> usize {
        self.shared.engine.read_analyser(out)
    }

    pub fn decode_async(&self, request: u32, bytes: Arc<[u8]>) {
        let job = DecodeJob { request, bytes };
        if let Some(sender) = &self.decode {
            if let Err(error) = sender.send(job) {
                let shared = self.shared.clone();
                std::thread::spawn(move || decode_job(&shared, error.0));
            }
            return;
        }
        let shared = self.shared.clone();
        std::thread::spawn(move || decode_job(&shared, job));
    }

    pub fn upload_buffer(&self, sample_rate: u32, channels: u16, samples: Vec<f32>) -> u32 {
        let buffer = Arc::new(AudioBuffer::new(sample_rate, channels, samples));
        let id = self.shared.next_buffer.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut buffers) = self.shared.buffers.lock() {
            buffers.insert(id, buffer);
        }
        id
    }

    pub fn buffer_info(&self, id: u32) -> Option<(u32, u16, usize)> {
        self.shared
            .buffers
            .lock()
            .ok()?
            .get(&id)
            .map(|buffer| (buffer.sample_rate, buffer.channels, buffer.frames))
    }

    pub fn read_channel(&self, id: u32, channel: usize, out: &mut [f32]) -> usize {
        let Some(buffer) = self
            .shared
            .buffers
            .lock()
            .ok()
            .and_then(|b| b.get(&id).cloned())
        else {
            return 0;
        };
        let Some(samples) = buffer.channel(channel) else {
            return 0;
        };
        let count = samples.len().min(out.len());
        out[..count].copy_from_slice(&samples[..count]);
        count
    }

    pub fn free_buffer(&self, id: u32) {
        if let Ok(mut buffers) = self.shared.buffers.lock() {
            buffers.remove(&id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn play_voice(
        &self,
        buffer: u32,
        gain: f32,
        pan: f32,
        rate: f64,
        looping: bool,
        offset_seconds: f64,
        start_at_frame: Option<u64>,
    ) -> Option<u32> {
        let buffer = self
            .shared
            .buffers
            .lock()
            .ok()
            .and_then(|buffers| buffers.get(&buffer).cloned())?;
        self.shared.engine.with_mixer(|mixer| {
            mixer.add_voice(
                buffer,
                gain,
                pan,
                rate,
                looping,
                offset_seconds,
                start_at_frame,
            )
        })
    }

    pub fn set_voice_gain(&self, voice: u32, gain: f32) {
        self.shared
            .engine
            .with_mixer(|mixer| mixer.set_gain(voice, gain));
    }

    pub fn set_voice_pan(&self, voice: u32, pan: f32) {
        self.shared
            .engine
            .with_mixer(|mixer| mixer.set_pan(voice, pan));
    }

    pub fn set_voice_rate(&self, voice: u32, rate: f64) {
        self.shared
            .engine
            .with_mixer(|mixer| mixer.set_rate(voice, rate));
    }

    pub fn set_voice_loop(&self, voice: u32, looping: bool) {
        self.shared
            .engine
            .with_mixer(|mixer| mixer.set_loop(voice, looping));
    }

    pub fn voice_position(&self, voice: u32) -> Option<f64> {
        self.shared
            .engine
            .with_mixer(|mixer| mixer.position(voice))?
    }

    pub fn voice_duration(&self, voice: u32) -> Option<f64> {
        self.shared
            .engine
            .with_mixer(|mixer| mixer.duration(voice))?
    }

    pub fn stop_voice(&self, voice: u32) {
        self.shared.engine.with_mixer(|mixer| mixer.stop(voice));
    }

    pub fn stop_voice_at(&self, voice: u32, frame: u64) {
        self.shared
            .engine
            .with_mixer(|mixer| mixer.stop_at(voice, frame));
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyser_tap_returns_the_latest_mono_frames() {
        let engine = Engine::new(48_000);
        engine.tap_analyser(&[0.1, 0.3, 0.5, 0.5, -0.2, -0.2]);
        let mut out = [0.0; 2];
        assert_eq!(engine.read_analyser(&mut out), 2);
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert!((out[1] + 0.2).abs() < 1e-6);
    }
}

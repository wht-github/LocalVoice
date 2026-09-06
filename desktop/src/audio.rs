use crate::{Event, config::Config};
use anyhow::{Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::VecDeque,
    io::Cursor,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
};

pub fn microphones() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

pub struct Capture {
    stop: Arc<AtomicBool>,
    flush: Arc<AtomicBool>,
}
impl Capture {
    pub fn start(config: Config, generation: u64, events: Sender<Event>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flush = Arc::new(AtomicBool::new(true));
        let (s, f) = (stop.clone(), flush.clone());
        thread::spawn(move || {
            if let Err(error) = capture(config, generation, events.clone(), s, f) {
                let _ = events.send(Event::CaptureError(generation, error.to_string()));
            }
        });
        Self { stop, flush }
    }
    pub fn stop(&self, flush: bool) {
        crate::diagnostics::record("capture_stop_requested", serde_json::json!({"flush":flush}));
        self.flush.store(flush, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn capture(
    config: Config,
    generation: u64,
    events: Sender<Event>,
    stop: Arc<AtomicBool>,
    flush: Arc<AtomicBool>,
) -> Result<()> {
    let host = cpal::default_host();
    let device = if config.microphone.is_empty() {
        host.default_input_device()
    } else {
        host.input_devices()?
            .find(|d| d.name().ok().as_deref() == Some(config.microphone.as_str()))
    }
    .ok_or_else(|| anyhow!("未找到麦克风，请检查设置和 Windows 麦克风权限"))?;
    let supported = device.default_input_config()?;
    let rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let segmenter = Arc::new(Mutex::new(Segmenter::new(
        rate,
        config.speech_threshold,
        config.continuous_dictation.then_some(config.silence_ms),
    )));
    let stream_config: cpal::StreamConfig = supported.clone().into();
    macro_rules! stream {
        ($sample:ty, $convert:expr) => {{
            let state = segmenter.clone();
            let out = events.clone();
            let failed = events.clone();
            let stopped = stop.clone();
            let should_flush = flush.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[$sample], _| {
                    let Ok(mut state) = state.lock() else { return };
                    if stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    for frame in data.chunks_exact(channels) {
                        let mono = frame.iter().map($convert).sum::<f32>() / channels as f32;
                        for item in state.push(mono) {
                            let event = match item {
                                Piece::Limit => {
                                    should_flush.store(false, Ordering::SeqCst);
                                    stopped.store(true, Ordering::SeqCst);
                                    Event::CaptureError(generation, "录音已达到内存容量上限，请缩短录音后重试".into())
                                }
                                Piece::Level(level) => Event::Level(generation, level),
                                Piece::Audio(samples) => Event::Audio(generation, samples, rate, Instant::now()),
                            };
                            let _ = out.send(event);
                        }
                        if stopped.load(Ordering::SeqCst) { break; }
                    }
                },
                move |error| {
                    let _ = failed.send(Event::CaptureError(generation, error.to_string()));
                },
                None,
            )?
        }};
    }
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => stream!(f32, |s: &f32| *s),
        cpal::SampleFormat::I16 => stream!(i16, |s: &i16| *s as f32 / 32768.),
        cpal::SampleFormat::U16 => stream!(u16, |s: &u16| (*s as f32 - 32768.) / 32768.),
        _ => bail!("暂不支持此麦克风的采样格式"),
    };
    stream.play()?;
    let _ = events.send(Event::CaptureStarted(generation));
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(20));
    }
    // Flush the final captured samples before joining the WASAPI driver thread.
    // Callbacks check stop under the same lock, so no samples arrive after flush.
    if flush.load(Ordering::SeqCst) {
        if let Some(samples) = segmenter.lock().unwrap().finish() {
            let _ = events.send(Event::Audio(generation, samples, rate, Instant::now()));
        }
    }
    let _ = events.send(Event::CaptureStopped(generation));
    crate::diagnostics::record(
        "capture_tail_dispatched",
        serde_json::json!({"generation":generation}),
    );
    let closing = Instant::now();
    drop(stream);
    crate::diagnostics::record(
        "capture_driver_closed",
        serde_json::json!({"generation":generation,"close_ms":closing.elapsed().as_millis()}),
    );
    Ok(())
}

enum Piece {
    Audio(Vec<f32>),
    Limit,
    Level(f32),
}
struct Segmenter {
    rate: u32,
    frame: Vec<f32>,
    preroll: VecDeque<f32>,
    samples: Vec<f32>,
    quiet: u32,
    voiced: u32,
    threshold: f32,
    ticks: u32,
    silence_ms: Option<u32>,
    continuation: bool,
}
impl Segmenter {
    fn new(rate: u32, threshold: f32, silence_ms: Option<u32>) -> Self {
        Self {
            rate,
            frame: Vec::new(),
            preroll: VecDeque::new(),
            samples: Vec::new(),
            quiet: 0,
            voiced: 0,
            threshold,
            ticks: 0,
            silence_ms,
            continuation: false,
        }
    }
    fn push(&mut self, sample: f32) -> Vec<Piece> {
        self.frame
            .push(if sample.is_finite() { sample } else { 0. });
        if self.frame.len() < self.rate as usize / 50 {
            return vec![];
        }
        let frame = std::mem::take(&mut self.frame);
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
        let speech = rms >= self.threshold;
        let mut out = Vec::new();
        self.ticks += 1;
        if self.ticks % 5 == 0 {
            out.push(Piece::Level((rms * 12.).min(1.)));
        }
        if self.samples.is_empty() && !speech {
            self.preroll.extend(frame);
            while self.preroll.len() > self.rate as usize / 5 {
                self.preroll.pop_front();
            }
            return out;
        }
        if self.samples.is_empty() {
            self.samples.extend(self.preroll.drain(..));
        }
        // Check before growth so Vec does not double capacity beyond this bound.
        if self.samples.len() + frame.len() > 64 * 1024 * 1024 {
            out.push(Piece::Limit);
            return out;
        }
        self.samples.extend(frame);
        if speech {
            self.quiet = 0;
            self.voiced += 20;
        } else {
            self.quiet += 20;
        }
        // Only continuous sessions submit on pauses. Manual sessions retain
        // every internal pause until the user's explicit stop.
        if let Some(silence_ms) = self.silence_ms {
            let limit = self.samples.len() >= self.rate as usize * 25;
            if self.quiet >= silence_ms || limit {
                let ongoing = limit && speech;
                if let Some(data) = self.finish() {
                    out.push(Piece::Audio(data));
                }
                self.continuation = ongoing;
            }
        }
        out
    }
    fn finish(&mut self) -> Option<Vec<f32>> {
        let mut data = std::mem::take(&mut self.samples);
        if !data.is_empty() {
            data.append(&mut self.frame);
        }
        let valid = self.voiced >= 200 || (self.continuation && self.voiced > 0);
        self.continuation = false;
        // Keep 150 ms trailing context; avoid a full silent tail in ASR input.
        if self.quiet > 150 {
            data.truncate(
                data.len()
                    .saturating_sub((self.quiet - 150) as usize * self.rate as usize / 1000),
            );
        }
        self.quiet = 0;
        self.voiced = 0;
        valid.then_some(data)
    }
}

pub fn encode_wav(samples: &[f32], rate: u32) -> Result<Vec<u8>> {
    // Preserve the device sample rate. The existing ASR service performs proper
    // anti-aliased resampling to 16 kHz, avoiding lossy client-side decimation.
    if rate == 0 {
        bail!("Invalid microphone sample rate");
    }
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels: 1,
                sample_rate: rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for value in samples {
            writer.write_sample((value.clamp(-1., 1.) * 32767.) as i16)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silence_is_not_transcribed_and_speech_tail_is_flushed() {
        let mut s = Segmenter::new(16000, 0.008, None);
        for _ in 0..32000 {
            assert!(!s.push(0.).iter().any(|p| matches!(p, Piece::Limit)));
        }
        for n in 0..8000 {
            s.push((n as f32 * 0.2).sin() * 0.15);
        }
        assert!(s.finish().unwrap().len() >= 8000);
        assert!(s.finish().is_none());
    }
    #[test]
    fn long_recording_and_pauses_are_kept_until_manual_finish() {
        let mut s = Segmenter::new(16000, 0.008, None);
        for n in 0..16000 * 52 {
            let sample = if n > 16000 * 20 && n < 16000 * 24 {
                0.
            } else {
                (n as f32 * 0.2).sin() * 0.15
            };
            assert!(s.push(sample).iter().all(|p| matches!(p, Piece::Level(_))));
        }
        assert_eq!(s.finish().unwrap().len(), 16000 * 52);
        assert!(s.finish().is_none());
    }
    #[test]
    fn continuous_pauses_emit_several_utterances_and_stop_flushes_tail() {
        let mut s = Segmenter::new(16000, 0.008, Some(700));
        let mut parts = Vec::new();
        for _ in 0..2 {
            for sample in std::iter::repeat_n(0.1, 16000).chain(std::iter::repeat_n(0., 16000)) {
                for piece in s.push(sample) {
                    if let Piece::Audio(audio) = piece {
                        parts.push(audio);
                    }
                }
            }
        }
        assert_eq!(parts.len(), 2);
        assert!(
            parts
                .iter()
                .all(|p| p.len() >= 16000 && p.len() < 16000 * 2)
        );
        assert!(s.finish().is_none());
        for _ in 0..8000 {
            s.push(0.1);
        }
        assert!(s.finish().unwrap().len() >= 8000);
        assert!(s.finish().is_none());
    }
    #[test]
    fn continuous_long_speech_is_bounded_and_keeps_short_final_tail() {
        let mut s = Segmenter::new(16000, 0.008, Some(700));
        let mut sizes = Vec::new();
        for _ in 0..16000 * 50 + 1600 {
            for piece in s.push(0.1) {
                if let Piece::Audio(audio) = piece {
                    sizes.push(audio.len());
                }
            }
        }
        sizes.push(s.finish().unwrap().len());
        assert_eq!(sizes, vec![16000 * 25, 16000 * 25, 1600]);
    }
    #[test]
    fn wav_keeps_device_sample_rate_and_duration() {
        let bytes = encode_wav(&vec![0.1; 48000], 48000).unwrap();
        let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.spec().sample_rate, 48000);
        assert_eq!(reader.duration(), 48000);
    }
}

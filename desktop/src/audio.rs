use crate::{
    Event,
    config::Config,
    vad::{self, Detector, FRAME_SIZE, Resampler, SAMPLE_RATE},
};
use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::VecDeque,
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Sender, TrySendError},
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
    let model = crate::paths::runtime()?.join("models/silero-vad/silero_vad.onnx");
    let mut processor = Processor::new(
        rate,
        config
            .recording_continuous()
            .then_some(config.effective_silence_ms()),
        &model,
    )?;
    let (sender, receiver) = mpsc::sync_channel(64);
    let callbacks = Arc::new(AtomicUsize::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let stream_config: cpal::StreamConfig = supported.clone().into();
    macro_rules! stream {
        ($sample:ty, $convert:expr) => {{
            let input = sender.clone();
            let failed = sender.clone();
            let stopped = stop.clone();
            let active = callbacks.clone();
            let full = overflow.clone();
            let failed_full = overflow.clone();
            let failed_stop = stop.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[$sample], _| {
                    if stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    active.fetch_add(1, Ordering::SeqCst);
                    // The second check pairs with the worker's stop barrier.
                    if !stopped.load(Ordering::SeqCst) {
                        let mono = data
                            .chunks_exact(channels)
                            .map(|frame| {
                                let value =
                                    frame.iter().map($convert).sum::<f32>() / channels as f32;
                                if value.is_finite() { value } else { 0. }
                            })
                            .collect();
                        if matches!(
                            input.try_send(Input::Audio(mono)),
                            Err(TrySendError::Full(_))
                        ) {
                            full.store(true, Ordering::SeqCst);
                            stopped.store(true, Ordering::SeqCst);
                        }
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                },
                move |error| {
                    if failed.try_send(Input::Error(error.to_string())).is_err() {
                        failed_full.store(true, Ordering::SeqCst);
                    }
                    failed_stop.store(true, Ordering::SeqCst);
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
    let mut dispatch = |piece| {
        let event = match piece {
            Piece::Level(level) => Event::Level(generation, level),
            Piece::Audio(samples) => Event::Audio(generation, samples, SAMPLE_RATE, Instant::now()),
        };
        let _ = events.send(event);
    };
    while !stop.load(Ordering::SeqCst) {
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(Input::Audio(samples)) => processor.push(&samples, false, &mut dispatch)?,
            Ok(Input::Error(error)) => bail!("麦克风录音失败：{error}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // Let an already-running callback enqueue its tail. New callbacks see stop
    // before admitting audio, so the queue is stable before it is drained.
    while callbacks.load(Ordering::SeqCst) != 0 {
        thread::yield_now();
    }
    if overflow.load(Ordering::SeqCst) {
        bail!("音频处理未能跟上麦克风，已取消本次录音以避免漏字；请关闭高负载任务后重试");
    }
    if flush.load(Ordering::SeqCst) {
        for input in receiver.try_iter() {
            match input {
                Input::Audio(samples) => processor.push(&samples, false, &mut dispatch)?,
                Input::Error(error) => bail!("麦克风录音失败：{error}"),
            }
        }
        processor.push(&[], true, &mut dispatch)?;
        if let Some(samples) = processor.segmenter.finish() {
            dispatch(Piece::Audio(samples));
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
    Level(f32),
}
enum Input {
    Audio(Vec<f32>),
    Error(String),
}

struct Processor {
    resampler: Resampler,
    detector: Detector,
    pending: Vec<f32>,
    segmenter: Segmenter,
    ticks: u32,
}
impl Processor {
    fn new(rate: u32, silence_ms: Option<u32>, model: &std::path::Path) -> Result<Self> {
        Ok(Self {
            resampler: Resampler::new(rate, SAMPLE_RATE)?,
            detector: Detector::new(model)?,
            pending: Vec::new(),
            segmenter: Segmenter::new(silence_ms),
            ticks: 0,
        })
    }

    fn push(&mut self, samples: &[f32], finish: bool, emit: &mut impl FnMut(Piece)) -> Result<()> {
        self.pending.extend(self.resampler.process(samples, finish));
        let pending = std::mem::take(&mut self.pending);
        for chunk in pending.chunks(FRAME_SIZE) {
            if chunk.len() < FRAME_SIZE && !finish {
                self.pending.extend_from_slice(chunk);
                break;
            }
            let mut frame = [0.; FRAME_SIZE];
            frame[..chunk.len()].copy_from_slice(chunk);
            let speech = self.detector.speech(&frame);
            self.ticks += 1;
            if self.ticks % 3 == 0 {
                emit(Piece::Level((vad::rms(chunk) * 12.).min(1.)));
            }
            if let Some(audio) = self.segmenter.push(chunk, speech)? {
                emit(Piece::Audio(audio));
            }
        }
        Ok(())
    }
}

/// Feed a fixture through the real capture pipeline without opening a microphone.
pub fn offline_capture(
    samples: &[f32],
    rate: u32,
    silence_ms: Option<u32>,
    model: &std::path::Path,
) -> Result<Vec<Vec<f32>>> {
    let mut processor = Processor::new(rate, silence_ms, model)?;
    let mut parts = Vec::new();
    let mut collect = |piece| {
        if let Piece::Audio(audio) = piece {
            parts.push(audio);
        }
    };
    for block in samples.chunks((rate as usize / 100).max(1)) {
        processor.push(block, false, &mut collect)?;
    }
    processor.push(&[], true, &mut collect)?;
    if let Some(audio) = processor.segmenter.finish() {
        parts.push(audio);
    }
    Ok(parts)
}

const PREROLL: usize = SAMPLE_RATE as usize * 400 / 1000;
const TAIL: usize = SAMPLE_RATE as usize * 250 / 1000;
const MAX_SAMPLES: usize = 64 * 1024 * 1024;
const CONTINUOUS_LIMIT: usize = SAMPLE_RATE as usize * 25;

struct Segmenter {
    preroll: VecDeque<f32>,
    samples: Vec<f32>,
    quiet: usize,
    silence_samples: Option<usize>,
    continuation: bool,
}
impl Segmenter {
    fn new(silence_ms: Option<u32>) -> Self {
        Self {
            preroll: VecDeque::new(),
            samples: Vec::new(),
            quiet: 0,
            silence_samples: silence_ms.map(|ms| {
                ms.saturating_sub(vad::HANGOVER_MS).max(32) as usize * SAMPLE_RATE as usize / 1000
            }),
            continuation: false,
        }
    }
    fn push(&mut self, frame: &[f32], speech: bool) -> Result<Option<Vec<f32>>> {
        if self.samples.is_empty() && !speech && !self.continuation {
            self.preroll.extend(frame);
            while self.preroll.len() > PREROLL {
                self.preroll.pop_front();
            }
            return Ok(None);
        }
        if self.samples.is_empty() {
            self.samples.extend(self.preroll.drain(..));
        }
        if self.samples.len() + frame.len() > MAX_SAMPLES {
            bail!("录音已达到内存容量上限，请缩短录音后重试");
        }
        if self.samples.capacity() < self.samples.len() + frame.len() {
            let capacity = (self.samples.capacity().max(SAMPLE_RATE as usize) * 2).min(MAX_SAMPLES);
            self.samples
                .try_reserve_exact(capacity - self.samples.len())
                .context("录音内存不足，请缩短录音后重试")?;
        }
        self.samples.extend_from_slice(frame);
        if speech {
            self.quiet = 0;
        } else {
            self.quiet += frame.len();
        }
        // Manual notes keep every internal pause and every quiet word after
        // onset. The model decides only the leading/trailing speech boundary.
        if let Some(silence_samples) = self.silence_samples {
            let limit = self.samples.len() >= CONTINUOUS_LIMIT;
            if self.quiet >= silence_samples || limit {
                let ongoing = limit && speech;
                let audio = self.finish();
                self.continuation = ongoing;
                return Ok(audio);
            }
        }
        Ok(None)
    }
    fn finish(&mut self) -> Option<Vec<f32>> {
        let mut data = std::mem::take(&mut self.samples);
        self.continuation = false;
        if self.quiet > TAIL {
            data.truncate(data.len().saturating_sub(self.quiet - TAIL));
        }
        self.quiet = 0;
        (!data.is_empty()).then_some(data)
    }
}

pub fn encode_wav(samples: &[f32], rate: u32) -> Result<Vec<u8>> {
    // Preserve the supplied rate; capture already uses band-limited resampling.
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

    fn feed(s: &mut Segmenter, samples: &[f32], speech: bool) -> Vec<Vec<f32>> {
        samples
            .chunks(FRAME_SIZE)
            .filter_map(|frame| s.push(frame, speech).unwrap())
            .collect()
    }

    #[test]
    fn model_classification_controls_admission_instead_of_volume() {
        let mut s = Segmenter::new(None);
        feed(&mut s, &vec![0.2; 32000], false); // loud noise rejected by VAD
        assert!(s.finish().is_none());
        feed(&mut s, &vec![0.001; 8000], true); // quiet speech admitted by VAD
        let audio = s.finish().unwrap();
        assert_eq!(&audio[audio.len() - 8000..], &vec![0.001; 8000]);
        assert!(s.finish().is_none());
    }

    #[test]
    fn long_recording_and_pauses_are_kept_until_manual_finish() {
        let mut s = Segmenter::new(None);
        let mut source = vec![0.1; 16000 * 20];
        source.extend(vec![0.001; 16000 * 4]);
        source.extend(vec![0.2; 16000 * 28]);
        assert!(feed(&mut s, &source[..16000 * 20], true).is_empty());
        assert!(feed(&mut s, &source[16000 * 20..16000 * 24], false).is_empty());
        assert!(feed(&mut s, &source[16000 * 24..], true).is_empty());
        assert_eq!(s.finish().unwrap(), source);
        assert!(s.finish().is_none());
    }

    #[test]
    fn continuous_pauses_emit_several_utterances_and_stop_flushes_tail() {
        let mut s = Segmenter::new(Some(700));
        let mut parts = Vec::new();
        for _ in 0..2 {
            parts.extend(feed(&mut s, &vec![0.1; 16000], true));
            parts.extend(feed(&mut s, &vec![0.; 16000], false));
        }
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.len() >= 16000 && p.len() < 32000));
        assert!(s.finish().is_none());
        feed(&mut s, &vec![0.1; 8000], true);
        assert!(s.finish().unwrap().len() >= 8000);
        assert!(s.finish().is_none());
    }

    #[test]
    fn daily_continuous_keeps_a_short_pause_and_following_correction_together() {
        let mut source = vec![0.1; 16000];
        source.extend(vec![0.; 14400]); // a pause before a spoken correction
        source.extend(vec![0.2; 8000]);
        for (mode, expected_parts) in [("raw", 2), ("clean", 1)] {
            let config = Config {
                output_mode: mode.into(),
                continuous_dictation: true,
                ..Config::default()
            };
            let mut s = Segmenter::new(
                config
                    .recording_continuous()
                    .then_some(config.effective_silence_ms()),
            );
            let mut parts = feed(&mut s, &source[..16000], true);
            parts.extend(feed(&mut s, &source[16000..30400], false));
            parts.extend(feed(&mut s, &source[30400..], true));
            parts.extend(s.finish());
            assert_eq!(parts.len(), expected_parts, "{mode}");
            if mode == "clean" {
                assert_eq!(parts[0], source);
            }
        }
    }

    #[test]
    fn continuous_long_speech_is_bounded_and_keeps_short_final_tail() {
        let mut s = Segmenter::new(Some(700));
        let source: Vec<_> = (0..16000 * 50 + 1000)
            .map(|n| (n as f32 * 0.2).sin())
            .collect();
        let mut parts = feed(&mut s, &source, true);
        parts.push(s.finish().unwrap());
        assert_eq!(parts.len(), 3);
        assert!(
            parts
                .iter()
                .all(|p| p.len() < CONTINUOUS_LIMIT + FRAME_SIZE)
        );
        assert_eq!(parts.concat(), source);
    }

    #[test]
    fn speech_has_preroll_and_a_short_tail_without_trimming_internal_audio() {
        let mut s = Segmenter::new(None);
        feed(&mut s, &vec![0.001; 16000], false);
        feed(&mut s, &vec![0.1; 8000], true);
        feed(&mut s, &vec![0.; 16000], false);
        let audio = s.finish().unwrap();
        assert_eq!(audio.len(), PREROLL + 8000 + TAIL);
        assert_eq!(&audio[..PREROLL], &vec![0.001; PREROLL]);
        assert_eq!(&audio[PREROLL..PREROLL + 8000], &vec![0.1; 8000]);
    }

    #[test]
    fn wav_keeps_device_sample_rate_and_duration() {
        let bytes = encode_wav(&vec![0.1; 48000], 48000).unwrap();
        let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.spec().sample_rate, 48000);
        assert_eq!(reader.duration(), 48000);
    }
}

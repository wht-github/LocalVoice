//! Lightweight, CPU-only voice detection. Audio stays in Rust throughout.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sherpa_onnx::{LinearResampler, SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::{path::Path, time::Instant};

pub const SAMPLE_RATE: u32 = 16_000;
pub const FRAME_SIZE: usize = 512;
pub const HANGOVER_MS: u32 = 96;

pub struct Detector {
    model: VoiceActivityDetector,
    buffered: usize,
}

impl Detector {
    pub fn new(path: &Path) -> Result<Self> {
        ensure!(
            path.is_file(),
            "缺少人声检测模型，请运行 install.ps1 安装 Silero VAD"
        );
        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(path.to_string_lossy().into_owned()),
                threshold: 0.5,
                min_silence_duration: HANGOVER_MS as f32 / 1000.,
                min_speech_duration: 0.128,
                window_size: FRAME_SIZE as i32,
                max_speech_duration: 30.,
            },
            sample_rate: SAMPLE_RATE as i32,
            num_threads: 1,
            provider: Some("cpu".into()),
            ..Default::default()
        };
        let model = VoiceActivityDetector::create(&config, 30.)
            .context("无法加载 Silero 人声检测模型；请检查原生运行库和模型文件")?;
        Ok(Self { model, buffered: 0 })
    }

    /// A complete 32 ms frame; classification includes the model's short hangover.
    pub fn speech(&mut self, frame: &[f32]) -> bool {
        debug_assert_eq!(frame.len(), FRAME_SIZE);
        self.model.accept_waveform(frame);
        let speech = self.model.detected();
        self.model.clear();
        self.buffered += frame.len();
        if self.buffered >= SAMPLE_RATE as usize * 5 {
            // sherpa buffers speech for its segment API. We keep the recording
            // separately, so release that buffer without resetting model context.
            self.model.flush();
            self.model.clear();
            self.buffered = 0;
        }
        speech
    }
}

/// Stateful band-limited resampling, including the final filter tail.
pub struct Resampler(Option<LinearResampler>);

impl Resampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Result<Self> {
        ensure!(source_rate > 0 && target_rate > 0, "音频采样率无效");
        let resampler = if source_rate == target_rate {
            None
        } else {
            Some(
                LinearResampler::create(source_rate.try_into()?, target_rate.try_into()?)
                    .context("无法创建音频重采样器")?,
            )
        };
        Ok(Self(resampler))
    }

    pub fn process(&self, samples: &[f32], finish: bool) -> Vec<f32> {
        match &self.0 {
            Some(resampler) => resampler.resample(samples, finish),
            None => samples.to_vec(),
        }
    }
}

pub fn resample_mono(samples: &[f32], source_rate: u32, target_rate: u32) -> Result<Vec<f32>> {
    Ok(Resampler::new(source_rate, target_rate)?.process(samples, true))
}

#[derive(Serialize)]
pub struct Evaluation {
    pub audio_seconds: f64,
    pub load_ms: f64,
    pub inference_ms: f64,
    pub realtime_factor: f64,
    pub detected_seconds: f64,
    pub first_detected_ms: Option<f64>,
    pub last_detected_ms: Option<f64>,
    pub legacy_energy_seconds: f64,
    pub model_bytes: u64,
}

/// Offline diagnostic only; the legacy energy gate is never used by capture.
pub fn offline_evaluate(path: &Path, samples: &[f32], source_rate: u32) -> Result<Evaluation> {
    let samples = resample_mono(samples, source_rate, SAMPLE_RATE)?;
    let started = Instant::now();
    let mut detector = Detector::new(path)?;
    let load_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    let mut detected = 0;
    let mut first = None;
    let mut last = None;
    for (index, chunk) in samples.chunks(FRAME_SIZE).enumerate() {
        let mut frame = [0.; FRAME_SIZE];
        frame[..chunk.len()].copy_from_slice(chunk);
        if detector.speech(&frame) {
            detected += chunk.len();
            first.get_or_insert(index * FRAME_SIZE);
            last = Some(index * FRAME_SIZE + chunk.len());
        }
    }
    let inference_ms = started.elapsed().as_secs_f64() * 1000.;
    let legacy_energy_samples: usize = samples
        .chunks(320)
        .filter(|frame| rms(frame) >= 0.008)
        .map(<[f32]>::len)
        .sum();
    let audio_seconds = samples.len() as f64 / SAMPLE_RATE as f64;
    Ok(Evaluation {
        audio_seconds,
        load_ms,
        inference_ms,
        realtime_factor: if audio_seconds > 0. {
            inference_ms / 1000. / audio_seconds
        } else {
            0.
        },
        detected_seconds: detected as f64 / SAMPLE_RATE as f64,
        first_detected_ms: first.map(|n| n as f64 / 16.),
        last_detected_ms: last.map(|n| n as f64 / 16.),
        legacy_energy_seconds: legacy_energy_samples as f64 / SAMPLE_RATE as f64,
        model_bytes: path.metadata()?.len(),
    })
}

pub fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_preserves_duration_and_suppresses_aliasing() {
        let tone = |hz: f32| {
            (0..48_000)
                .map(|n| (n as f32 * hz * std::f32::consts::TAU / 48_000.).sin())
                .collect::<Vec<_>>()
        };
        let low = resample_mono(&tone(1000.), 48_000, 16_000).unwrap();
        let high = resample_mono(&tone(12_000.), 48_000, 16_000).unwrap();
        assert_eq!(low.len(), 16_000);
        assert!(rms(&low[100..15_900]) > 0.65);
        assert!(rms(&high[100..15_900]) < 0.02);
    }

    #[test]
    fn chunked_resampling_matches_one_shot_including_tail() {
        let input: Vec<_> = (0..44_101).map(|n| (n as f32 * 0.17).sin()).collect();
        let expected = resample_mono(&input, 44_100, 16_000).unwrap();
        let resampler = Resampler::new(44_100, 16_000).unwrap();
        let mut actual = Vec::new();
        for chunk in input.chunks(441) {
            actual.extend(resampler.process(chunk, false));
        }
        actual.extend(resampler.process(&[], true));
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < 1e-5)
        );
    }
}

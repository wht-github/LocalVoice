//! Native recognition and editing; the complete original survives any editor failure.
use crate::config::Config;
use anyhow::{Context, Result, bail};

use std::{io::Cursor, sync::OnceLock, time::Instant};

static EDITOR: OnceLock<crate::text::TextProcessor> = OnceLock::new();
fn editor() -> Result<&'static crate::text::TextProcessor> {
    let root = crate::paths::root()?;
    Ok(EDITOR.get_or_init(|| crate::text::TextProcessor::new(root)))
}
pub(crate) fn stop_text() {
    if let Some(editor) = EDITOR.get() {
        editor.stop();
    }
}
pub(crate) fn shutdown_text() {
    if let Some(editor) = EDITOR.get() {
        editor.shutdown();
    }
}
pub fn ready(config: &Config) -> Result<()> {
    crate::runtime::ready(config)
}

#[derive(Clone)]
pub struct Transcript {
    pub text: String,
    pub request_seconds: f64,
    pub inference_seconds: Option<f64>,
}
#[derive(Clone)]
pub struct Dictation {
    pub original: String,
    pub text: String,
    pub mode: String,
    pub warning: Option<String>,
    pub request_seconds: f64,
    pub inference_seconds: Option<f64>,
    pub processing_seconds: f64,
    pub preview_only: bool,
}
pub fn process_transcript(
    config: &Config,
    transcript: Transcript,
    preview_only: bool,
    cancelled: impl Fn() -> bool,
) -> Result<Dictation> {
    // Reserve before process_with checks the caller's generation. Whichever side
    // of Stop this runs on, either that check or the model ticket rejects old work.
    let editor = editor()?;
    let ticket = editor.ticket();
    process_with(config, transcript, preview_only, cancelled, |text, mode| {
        editor.process_with_ticket(ticket, text, mode)
    })
}
fn process_with(
    config: &Config,
    transcript: Transcript,
    preview_only: bool,
    cancelled: impl Fn() -> bool,
    process: impl FnOnce(&str, &str) -> Result<String>,
) -> Result<Dictation> {
    if cancelled() {
        bail!("已取消处理");
    }
    let mut result = Dictation {
        original: transcript.text.clone(),
        text: transcript.text,
        mode: config.output_mode.clone(),
        warning: None,
        request_seconds: transcript.request_seconds,
        inference_seconds: transcript.inference_seconds,
        processing_seconds: 0.,
        preview_only,
    };
    if config.output_mode == "raw" || result.original.trim().is_empty() {
        return Ok(result);
    }
    let started = Instant::now();
    let processed = process(&result.original, &config.output_mode).and_then(|text| {
        if text.trim().is_empty() {
            bail!("整理结果为空");
        }
        Ok(text.trim().to_owned())
    });
    if cancelled() {
        bail!("已取消处理");
    }
    result.processing_seconds = started.elapsed().as_secs_f64();
    match processed {
        Ok(text) => result.text = text,
        Err(error) => {
            result.warning = Some(format!(
                "{}：{error}",
                if config.output_mode == "notes" {
                    "笔记整理未完成，本次使用原始转写"
                } else {
                    "整理未完成，本次使用原始转写"
                }
            ))
        }
    }
    crate::diagnostics::record(
        "text_processed",
        serde_json::json!({
            "mode":result.mode, "asr_mode":config.asr_mode,
            "seconds":result.processing_seconds, "fallback":result.warning.is_some(),
        }),
    );
    Ok(result)
}
pub fn prepare_text_task() -> Result<impl FnOnce() -> Result<String> + Send + 'static> {
    let editor = editor()?.clone();
    let ticket = editor.ticket();
    Ok(move || editor.prepare_with_ticket(ticket, true))
}

pub use crate::text::WarmSession as WarmTextSession;

/// Prepare while the user speaks, using installed files only. Stop invalidates
/// the ticket before any queued background preparation can load another model.
pub fn warm_text_session(config: &Config) -> Option<WarmTextSession> {
    if config.output_mode == "raw" {
        return None;
    }
    let editor = editor().ok()?.clone();
    let status = editor.health();
    if !status.installed {
        return None;
    }
    let ticket = editor.ticket();
    let session = editor.keep_warm();
    if !status.ready {
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = editor.prepare_with_ticket(ticket, false);
            crate::diagnostics::record(
                "text_prewarm",
                serde_json::json!({"seconds":started.elapsed().as_secs_f64(),
                    "ready":result.is_ok()}),
            );
        });
    }
    Some(session)
}
pub struct TextStatus {
    pub installed: bool,
    pub ready: bool,
    pub message: String,
}
pub fn text_status(_config: &Config) -> Result<TextStatus> {
    let status = editor()?.health();
    let message = match status.state {
        "downloading" => "正在下载整理模型，完成后可以离线使用",
        "loading" => "正在加载整理模型，完成后继续处理",
        "processing" => "正在整理完整正文",
        _ if status.ready => "Agentic 已就绪；空闲 60 秒释放内存，下次整理自动加载",
        _ if status.installed => "模型已安装，当前未占用模型内存；下次整理自动加载，也可提前预加载",
        _ => "尚未下载整理模型；准备一次后即可离线使用",
    };
    Ok(TextStatus {
        installed: status.installed,
        ready: status.ready,
        message: message.into(),
    })
}
pub fn transcribe_recording(
    config: &Config,
    samples: &[f32],
    rate: u32,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Transcript> {
    if rate == 0 {
        bail!("录音采样率无效");
    }
    let ranges = recognition_ranges(samples, rate);
    let mut result = Transcript {
        text: String::new(),
        request_seconds: 0.,
        inference_seconds: None,
    };
    for (index, range) in ranges.iter().enumerate() {
        if cancelled() {
            bail!("已取消识别");
        }
        progress(index + 1, ranges.len());
        let started = Instant::now();
        let part = crate::runtime::transcribe(config, &samples[range.clone()], rate)?;
        if cancelled() {
            bail!("已取消识别");
        }
        if result
            .text
            .ends_with(|c: char| c.is_ascii_alphanumeric() || c.is_ascii_punctuation())
            && part.starts_with(|c: char| c.is_ascii_alphanumeric())
        {
            result.text.push(' ');
        }
        result.text.push_str(&part);
        let seconds = started.elapsed().as_secs_f64();
        result.request_seconds += seconds;
        crate::diagnostics::record(
            "asr_response",
            serde_json::json!({
                "asr_mode":config.asr_mode, "term_count":config.terms.len(),
                "seconds":seconds, "index":index+1, "total":ranges.len(),
                "audio_seconds":range.len() as f64/rate as f64, "runtime":"native",
            }),
        );
    }
    Ok(result)
}
pub fn decode_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32)> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).context("无法读取 WAV 录音")?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        bail!("WAV 格式无效");
    }
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 2_f32.powi(spec.bits_per_sample as i32 - 1);
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| s as f32 / scale))
                .collect::<std::result::Result<Vec<_>, _>>()?
        }
    };
    let mono = samples
        .chunks_exact(spec.channels as usize)
        .map(|frame| frame.iter().sum::<f32>() / spec.channels as f32)
        .collect();
    Ok((mono, spec.sample_rate))
}
pub fn transcribe_timed(config: &Config, wav: Vec<u8>) -> Result<Transcript> {
    let (samples, rate) = decode_wav(&wav)?;
    transcribe_recording(config, &samples, rate, || false, |_, _| {})
}
fn recognition_ranges(samples: &[f32], rate: u32) -> Vec<std::ops::Range<usize>> {
    // Manual stop owns submission. Split only after it, with lossless boundaries.
    let rate = rate.max(1) as usize;
    let mut start = 0;
    let mut result = Vec::new();
    while start < samples.len() {
        let mut end = (start + rate * 25).min(samples.len());
        if end < samples.len() {
            let width = (rate / 10).max(1);
            if let Some((offset, _)) = samples[start + rate * 20..end]
                .chunks(width)
                .enumerate()
                .map(|(i, frame)| {
                    (
                        i,
                        frame.iter().map(|v| v * v).sum::<f32>() / frame.len() as f32,
                    )
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
            {
                end = start + rate * 20 + offset * width + width / 2;
            }
        }
        result.push(start..end);
        start = end;
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn transcript() -> Transcript {
        Transcript {
            text: "嗯，把 AGV 的 blueprint 导入 UE，不要修改 shader。".into(),
            request_seconds: 0.5,
            inference_seconds: Some(0.3),
        }
    }
    #[test]
    fn raw_bypasses_editor() {
        let r = process_with(
            &Config::default(),
            transcript(),
            false,
            || false,
            |_, _| panic!("raw must not load editor"),
        )
        .unwrap();
        assert_eq!(r.text, r.original);
        assert_eq!(r.request_seconds, 0.5);
        assert_eq!(r.processing_seconds, 0.);
    }
    #[test]
    fn only_explicit_retry_skips_delivery_and_original_is_retained() {
        for mode in ["clean", "notes"] {
            for retry in [false, true] {
                let config = Config {
                    output_mode: mode.into(),
                    ..Config::default()
                };
                let r = process_with(
                    &config,
                    transcript(),
                    retry,
                    || false,
                    |text, m| {
                        assert_eq!(text, transcript().text);
                        assert_eq!(m, mode);
                        Ok("整理正文".into())
                    },
                )
                .unwrap();
                assert_eq!(r.original, transcript().text);
                assert_eq!(r.text, "整理正文");
                assert_eq!(r.preview_only, retry);
            }
        }
    }
    #[test]
    fn failure_or_empty_output_keeps_complete_original() {
        let config = Config {
            output_mode: "notes".into(),
            ..Config::default()
        };
        for fail in [false, true] {
            let r = process_with(
                &config,
                transcript(),
                false,
                || false,
                |_, _| {
                    if fail {
                        bail!("model unavailable");
                    }
                    Ok(" ".into())
                },
            )
            .unwrap();
            assert_eq!(r.text, transcript().text);
            assert!(r.warning.is_some());
        }
    }
    #[test]
    fn cancellation_discards_both_output_and_fallback() {
        let config = Config {
            output_mode: "notes".into(),
            ..Config::default()
        };
        for fail in [false, true] {
            let cancelled = Cell::new(false);
            let r = process_with(
                &config,
                transcript(),
                false,
                || cancelled.get(),
                |_, _| {
                    cancelled.set(true);
                    if fail {
                        bail!("stopped");
                    }
                    Ok("正文".into())
                },
            );
            assert!(r.is_err());
        }
    }
    #[test]
    fn long_audio_ranges_are_bounded_lossless_and_prefer_pauses() {
        let mut samples = vec![0.2; 16000 * 63];
        samples[16000 * 23..16000 * 24].fill(0.);
        let r = recognition_ranges(&samples, 16000);
        assert!(r.len() >= 3);
        assert!((16000 * 23..16000 * 24).contains(&r[0].end));
        assert!(r.iter().all(|r| !r.is_empty() && r.len() <= 16000 * 25));
        assert_eq!(r[0].start, 0);
        assert_eq!(r.last().unwrap().end, samples.len());
        assert!(r.windows(2).all(|r| r[0].end == r[1].start));
    }
}

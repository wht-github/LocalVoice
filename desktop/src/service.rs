use crate::config::Config;
use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client, multipart};
use serde::Deserialize;
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(90))
        .build()?)
}
#[derive(Deserialize)]
pub struct Voice {
    pub id: String,
    pub language: String,
    #[serde(default)]
    pub name: String,
}
#[derive(Deserialize)]
struct VoiceList {
    voices: Vec<Voice>,
}
pub fn voices(config: &Config) -> Result<Vec<Voice>> {
    Ok(client()?
        .get(format!("{}/v1/audio/voices", config.tts_url))
        .send()?
        .error_for_status()?
        .json::<VoiceList>()?
        .voices)
}
pub fn ready(config: &Config) -> Result<()> {
    let client = client()?;
    let mut urls = vec![&config.asr_url];
    if config.tts_enabled {
        urls.push(&config.tts_url);
    }
    for url in urls {
        let value: serde_json::Value = client
            .get(format!("{url}/health"))
            .send()?
            .error_for_status()?
            .json()?;
        if value["ready"] != true {
            bail!("模型仍在加载");
        }
    }
    Ok(())
}
pub fn transcribe(client: &Client, config: &Config, wav: Vec<u8>) -> Result<String> {
    Ok(transcribe_timed(client, config, wav)?.text)
}
pub struct Transcript {
    pub text: String,
    pub request_seconds: f64,
    pub inference_seconds: Option<f64>,
}
pub fn transcribe_recording(
    client: &Client,
    config: &Config,
    samples: &[f32],
    rate: u32,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Transcript> {
    let ranges = recognition_ranges(samples, rate);
    let mut result = Transcript {
        text: String::new(),
        request_seconds: 0.,
        inference_seconds: Some(0.),
    };
    for (index, range) in ranges.iter().enumerate() {
        if cancelled() {
            bail!("已取消识别");
        }
        progress(index + 1, ranges.len());
        let encoding = Instant::now();
        let wav = crate::audio::encode_wav(&samples[range.clone()], rate)?;
        crate::diagnostics::record(
            "asr_chunk_encoded",
            serde_json::json!({"index":index+1,
            "total":ranges.len(),"audio_seconds":range.len() as f64 / rate as f64,
            "encode_ms":encoding.elapsed().as_millis()}),
        );
        let part = transcribe_timed(client, config, wav)?;
        if result
            .text
            .ends_with(|c: char| c.is_ascii_alphanumeric() || c.is_ascii_punctuation())
            && part.text.starts_with(|c: char| c.is_ascii_alphanumeric())
        {
            result.text.push(' ');
        }
        result.text.push_str(&part.text);
        result.request_seconds += part.request_seconds;
        result.inference_seconds = result
            .inference_seconds
            .zip(part.inference_seconds)
            .map(|(a, b)| a + b);
    }
    Ok(result)
}

fn recognition_ranges(samples: &[f32], rate: u32) -> Vec<std::ops::Range<usize>> {
    // Backend requests are bounded, but nothing is recognized or submitted before
    // manual stop. Prefer a quiet 100 ms boundary near each 25 second limit.
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
mod recording_tests {
    use super::*;
    #[test]
    fn long_audio_ranges_are_bounded_lossless_and_prefer_pauses() {
        let mut samples = vec![0.2; 16000 * 63];
        samples[16000 * 23..16000 * 24].fill(0.);
        let ranges = recognition_ranges(&samples, 16000);
        assert!(ranges.len() >= 3);
        assert!((16000 * 23..16000 * 24).contains(&ranges[0].end));
        assert!(
            ranges
                .iter()
                .all(|r| !r.is_empty() && r.len() <= 16000 * 25)
        );
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges.last().unwrap().end, samples.len());
        assert!(ranges.windows(2).all(|r| r[0].end == r[1].start));
    }
}
pub fn transcribe_timed(client: &Client, config: &Config, wav: Vec<u8>) -> Result<Transcript> {
    let started = Instant::now();
    let form = multipart::Form::new()
        .part(
            "file",
            multipart::Part::bytes(wav)
                .file_name("recording.wav")
                .mime_str("audio/wav")?,
        )
        .text("language", "auto");
    let response = client
        .post(format!("{}/v1/audio/transcriptions", config.asr_url))
        .multipart(form)
        .send()?;
    if !response.status().is_success() {
        crate::diagnostics::record(
            "asr_http_error",
            serde_json::json!({"status":response.status().as_u16(),"http_seconds":started.elapsed().as_secs_f64()}),
        );
        bail!("识别服务返回 {}", response.status());
    }
    let value: serde_json::Value = response.json()?;
    crate::diagnostics::record(
        "asr_response",
        serde_json::json!({"http_seconds":started.elapsed().as_secs_f64(),"server_seconds":value["inference_seconds"],"audio_seconds":value["duration"]}),
    );
    Ok(Transcript {
        text: value["text"].as_str().unwrap_or_default().to_owned(),
        request_seconds: started.elapsed().as_secs_f64(),
        inference_seconds: value["inference_seconds"].as_f64(),
    })
}
pub fn speak(
    config: Config,
    text: String,
    id: u64,
    generation: Arc<AtomicU64>,
    mut update: impl FnMut(&str),
) -> Result<()> {
    if text.chars().count() > 20000 {
        bail!("朗读文本超过 20000 字，请缩小选区；文本没有被截断朗读");
    }
    let parts: Vec<_> = crate::text::segments(&text)
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect();
    let client = client()?;
    let (_stream, handle) = rodio::OutputStream::try_default()?;
    let sink = rodio::Sink::try_new(&handle)?;
    queue_speech(&parts, &sink, id, generation, &mut update, |sentence| {
        for attempt in 0..4 {
            let response = client.post(format!("{}/v1/audio/speech", config.tts_url))
                .json(&serde_json::json!({"input":sentence,"voice":config.voice,"speed":config.speed}))
                .send().context("朗读合成失败")?;
            if response.status() == 429 && attempt < 3 {
                thread::sleep(Duration::from_millis(700));
                continue;
            }
            if !response.status().is_success() {
                bail!("朗读服务返回 {}", response.status());
            }
            return Ok(response.bytes()?.to_vec());
        }
        bail!("朗读服务忙碌")
    })
}

fn queue_speech(
    parts: &[String],
    sink: &rodio::Sink,
    id: u64,
    generation: Arc<AtomicU64>,
    mut update: impl FnMut(&str),
    mut synthesize: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<()> {
    use rodio::Source;
    update("正在准备朗读…");
    for part in parts {
        // Keep at most the current audio plus one successor. Synthesize the
        // successor while rodio plays the current source on its audio thread.
        while sink.len() >= 2 {
            if generation.load(Ordering::SeqCst) != id {
                sink.stop();
                return Ok(());
            }
            thread::sleep(Duration::from_millis(25));
        }
        if generation.load(Ordering::SeqCst) != id {
            sink.stop();
            return Ok(());
        }
        let audio = synthesize(part)?;
        if generation.load(Ordering::SeqCst) != id {
            sink.stop();
            return Ok(());
        }
        let cancellation = generation.clone();
        // Cancellation must also stop playback while this worker is blocked on
        // the next HTTP request. The audio thread checks it independently.
        let source = rodio::Decoder::new(Cursor::new(audio))?
            .stoppable()
            .periodic_access(Duration::from_millis(20), move |source| {
                if cancellation.load(Ordering::SeqCst) != id {
                    source.stop();
                }
            });
        sink.append(source);
        update("正在朗读 · 再按停止");
    }
    while !sink.empty() {
        if generation.load(Ordering::SeqCst) != id {
            sink.stop();
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

#[cfg(test)]
mod playback_tests {
    use super::*;
    #[test]
    fn cancellation_stops_audio_during_the_next_synthesis() {
        let (sink, mut output) = rodio::Sink::new_idle();
        let generation = Arc::new(AtomicU64::new(1));
        let cancel = generation.clone();
        let mut calls = 0;
        queue_speech(
            &["第一句".into(), "第二句".into()],
            &sink,
            1,
            generation,
            |_| {},
            |_| {
                calls += 1;
                if calls == 2 {
                    cancel.store(2, Ordering::SeqCst);
                    // Emulate the audio thread while synthesis has not returned.
                    for _ in 0..100000 {
                        output.next();
                        if sink.empty() {
                            break;
                        }
                    }
                    assert!(sink.empty(), "Cancellation waited for synthesis to finish");
                }
                crate::audio::encode_wav(&vec![0.1; 16000 * 10], 16000)
            },
        )
        .unwrap();
        assert_eq!(calls, 2);
    }
    #[test]
    fn prefetches_while_audio_is_queued_and_bounds_the_queue() {
        // Idle sink has no device: no sound is played. It stays full until cancelled.
        let (sink, _output) = rodio::Sink::new_idle();
        let generation = Arc::new(AtomicU64::new(1));
        let calls = std::cell::Cell::new(0);
        let parts = vec!["测试".to_owned(); 20];
        let cancel = generation.clone();
        let mut cancel_thread = None;
        queue_speech(
            &parts,
            &sink,
            1,
            generation,
            |_| {},
            |_| {
                let n = calls.get() + 1;
                calls.set(n);
                if n == 2 {
                    assert_eq!(
                        sink.len(),
                        1,
                        "Next synthesis must start before playback finishes"
                    );
                    let cancel = cancel.clone();
                    cancel_thread = Some(thread::spawn(move || {
                        thread::sleep(Duration::from_millis(80));
                        cancel.store(2, Ordering::SeqCst);
                    }));
                }
                assert!(n <= 2, "A full queue must not fetch a third chunk");
                crate::audio::encode_wav(&vec![0.1; 16000], 16000)
            },
        )
        .unwrap();
        cancel_thread.unwrap().join().unwrap();
        assert_eq!(calls.get(), 2);
    }
}

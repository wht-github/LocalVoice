//! Explicit native audio checks using the project's public fixture; no microphone.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::json;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use windows::Win32::{
    Foundation::FILETIME,
    System::{
        ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        },
        Threading::{GetCurrentProcess, GetProcessTimes},
    },
};

#[derive(Clone, Copy, Serialize)]
struct Resources {
    working_set_bytes: usize,
    private_bytes: usize,
    lifetime_peak_working_set_bytes: usize,
    cpu_ms: f64,
}

fn resources() -> Result<Resources> {
    unsafe {
        let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
        let size = std::mem::size_of_val(&memory) as u32;
        memory.cb = size;
        let process = GetCurrentProcess();
        GetProcessMemoryInfo(
            process,
            &mut memory as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
            size,
        )?;
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user)?;
        let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        Ok(Resources {
            working_set_bytes: memory.WorkingSetSize,
            private_bytes: memory.PrivateUsage,
            lifetime_peak_working_set_bytes: memory.PeakWorkingSetSize,
            cpu_ms: (ticks(kernel) + ticks(user)) as f64 / 10_000.,
        })
    }
}

fn evaluate(model: &Path, samples: &[f32], rate: u32) -> Result<serde_json::Value> {
    let before = resources()?;
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let monitor = thread::spawn(move || -> Result<Resources> {
        let mut peak = before;
        while !finished.load(Ordering::Relaxed) {
            let current = resources()?;
            peak.working_set_bytes = peak.working_set_bytes.max(current.working_set_bytes);
            peak.private_bytes = peak.private_bytes.max(current.private_bytes);
            thread::sleep(Duration::from_millis(2));
        }
        Ok(peak)
    });
    let result = crate::vad::offline_evaluate(model, samples, rate);
    done.store(true, Ordering::Relaxed);
    let peak = monitor
        .join()
        .map_err(|_| anyhow::anyhow!("资源监测线程异常"))??;
    let after = resources()?;
    let result = result?;
    Ok(json!({
        "vad": result,
        "resources": {
            "before": before,
            "after_model_drop": after,
            "sampled_peak_working_set_bytes": peak.working_set_bytes,
            "sampled_peak_private_bytes": peak.private_bytes,
            "working_set_increase_bytes": peak.working_set_bytes.saturating_sub(before.working_set_bytes),
            "private_increase_bytes": peak.private_bytes.saturating_sub(before.private_bytes),
            "process_cpu_ms": after.cpu_ms - before.cpu_ms,
            "measurement": "Same process, 2 ms memory sampling; CPU includes model load, inference and sampling thread. Allocation reuse can reduce later deltas."
        }
    }))
}

pub fn run_vad(output: &str) -> Result<()> {
    let root = crate::paths::root()?;
    let fixture = root.join("tests/fixtures/ascend-01219.wav");
    let model = root.join(".runtime/models/silero-vad/silero_vad.onnx");
    let (speech, rate) = crate::service::decode_wav(&fs::read(&fixture)?)?;
    let mut cases = Vec::new();
    let mut add = |name: &str, kind: &str, samples: Vec<f32>| -> Result<()> {
        cases.push(
            json!({ "name": name, "kind": kind, "result": evaluate(&model, &samples, rate)? }),
        );
        Ok(())
    };
    add("original", "real_public_speech", speech.clone())?;
    for db in [-24, -36] {
        let gain = 10_f32.powf(db as f32 / 20.);
        add(
            &format!("speech_{db}dB"),
            "synthetic_attenuation",
            speech.iter().map(|s| s * gain).collect(),
        )?;
    }
    let length = rate as usize * 5;
    add("silence", "synthetic_non_speech", vec![0.; length])?;
    let mut seed = 0x6d2b79f5u32;
    let white = (0..length)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f64 / u32::MAX as f64 * 2. - 1.) as f32 * 0.05
        })
        .collect();
    add("steady_white_noise", "synthetic_non_speech", white)?;
    let hum = (0..length)
        .map(|n| {
            let phase = n as f32 * std::f32::consts::TAU / rate as f32;
            (phase * 50.).sin() * 0.03 + (phase * 100.).sin() * 0.01
        })
        .collect();
    add("mains_hum_50_100Hz", "synthetic_non_speech", hum)?;

    // Five seconds of internal silence must remain inside one manual note.
    let mut note = speech.clone();
    note.extend(vec![0.; rate as usize * 5]);
    note.extend_from_slice(&speech);
    let manual = crate::audio::offline_capture(&note, rate, None, &model)?;
    let continuous = crate::audio::offline_capture(&note, rate, Some(700), &model)?;
    let manual_seconds = manual.iter().map(Vec::len).sum::<usize>() as f64 / 16_000.;
    let checks = json!({
        "manual_parts": manual.len(),
        "manual_output_seconds": manual_seconds,
        "continuous_parts": continuous.len(),
        "continuous_part_seconds": continuous.iter().map(|part| part.len() as f64 / 16_000.).collect::<Vec<_>>(),
        "manual_retains_internal_pause": manual.len() == 1 && manual_seconds > 7.,
        "continuous_submits_on_pause_and_flushes_tail": continuous.len() == 2,
    });
    let report = json!({
        "backend": "Silero VAD / sherpa-onnx 1.13.8, CPU, 1 inference thread",
        "fixture": "tests/fixtures/ascend-01219.wav; ASCEND test recording 01219, CC BY-SA 4.0; see tests/fixtures/README.md",
        "scope": "Small regression check, not an accuracy benchmark. Attenuated speech is not natural whispering. Noise is synthetic and does not cover keyboards or real rooms. No denoising is applied.",
        "ablation": "legacy_energy_seconds is the previous 20 ms RMS >= 0.008 gate; Silero detected_seconds includes its short hangover, so durations are not frame-accuracy scores.",
        "cases": cases,
        "capture_checks": checks,
    });
    let output = Path::new(output);
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, serde_json::to_vec_pretty(&report)?).context("无法写入 VAD 验证报告")?;
    ensure!(
        manual.len() == 1 && manual_seconds > 7.,
        "手动录音的内部停顿保留失败；查看报告"
    );
    ensure!(
        continuous.len() == 2,
        "连续录音的停顿提交或尾段保留失败；查看报告"
    );
    Ok(())
}

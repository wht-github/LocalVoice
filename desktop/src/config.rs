use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Order shared by settings display and persistence; keep the original backend
// available for existing settings and controlled PyTorch comparisons.
pub const ASR_MODES: &[(&str, &str)] = &[
    ("sensevoice-cpu", "SenseVoice · CPU"),
    ("qwen-llama-0.6b", "llama · Qwen ASR 0.6B Q8"),
    ("qwen-llama-1.7b", "llama · Qwen ASR 1.7B Q8"),
    ("qwen-native", "PyTorch · Qwen ASR 0.6B"),
];

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub asr_url: String,
    pub asr_mode: String,
    pub tts_url: String,
    pub record_key: String,
    pub read_key: String,
    pub microphone: String,
    pub continuous_dictation: bool,
    pub voice: String,
    pub speed: f32,
    pub silence_ms: u32,
    pub speech_threshold: f32,
    pub tts_enabled: bool,
    pub auto_start_services: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            asr_url: "http://127.0.0.1:8001".into(),
            asr_mode: "sensevoice-cpu".into(),
            tts_url: "http://127.0.0.1:8002".into(),
            record_key: "Ctrl+Alt+Space".into(),
            read_key: "Ctrl+Alt+R".into(),
            microphone: String::new(),
            continuous_dictation: false,
            voice: "default".into(),
            speed: 1.,
            silence_ms: 700,
            speech_threshold: 0.008,
            tts_enabled: true,
            auto_start_services: true,
        }
    }
}
impl Config {
    pub fn path() -> PathBuf {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()))
            .join("LocalVoice")
            .join("settings.json")
    }
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let config: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if !ASR_MODES.iter().any(|(mode, _)| *mode == self.asr_mode) {
            bail!("识别模式无效");
        }
        for address in [&self.asr_url, &self.tts_url] {
            let url = reqwest::Url::parse(address).context("服务地址无效")?;
            if url.scheme() != "http"
                || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                || !url.username().is_empty()
                || url.password().is_some()
            {
                bail!("初版仅连接本机 HTTP 服务");
            }
        }
        if !(0.5..=2.).contains(&self.speed) {
            bail!("语速应为 0.5–2.0");
        }
        if !(300..=2000).contains(&self.silence_ms)
            || !(0.001..=0.1).contains(&self.speech_threshold)
        {
            bail!("录音分段设置超出范围");
        }
        if self.record_key == self.read_key {
            bail!("两个快捷键不能相同");
        }
        Ok(())
    }
    pub fn save(&self) -> Result<()> {
        self.validate()?;
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_settings_keep_service_start_enabled() {
        let config: Config = serde_json::from_str(r#"{"voice":"zf_xiaobei"}"#).unwrap();
        assert!(config.tts_enabled && config.auto_start_services);
        assert_eq!(config.voice, "zf_xiaobei");
        assert!(!config.continuous_dictation);
    }
    #[test]
    fn continuous_mode_survives_settings_roundtrip() {
        let config = Config {
            continuous_dictation: true,
            ..Config::default()
        };
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(restored.continuous_dictation);
    }
    #[test]
    fn native_mode_validates_and_old_settings_default_to_cpu() {
        let old: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(old.asr_mode, "sensevoice-cpu");
        let config = Config::default();
        config.validate().unwrap();
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored.asr_mode, "sensevoice-cpu");
        let qwen = Config {
            asr_mode: "qwen-native".into(),
            ..Config::default()
        };
        qwen.validate().unwrap();
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&qwen).unwrap()).unwrap();
        assert_eq!(restored.asr_mode, "qwen-native");
        // Historical values (WSL qwen-vllm, removed sensevoice-gpu) no longer validate;
        // load() rejects them and the app falls back to defaults.
        for mode in ["qwen-vllm", "sensevoice-gpu", "other"] {
            assert!(
                Config {
                    asr_mode: mode.into(),
                    ..Config::default()
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn disabled_reading_survives_save_roundtrip() {
        let config = Config {
            tts_enabled: false,
            auto_start_services: false,
            ..Config::default()
        };
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(!restored.tts_enabled && !restored.auto_start_services);
    }

    #[test]
    fn all_selectable_models_roundtrip_without_changing_other_settings() {
        for (mode, _) in ASR_MODES {
            let config = Config {
                asr_mode: (*mode).into(),
                continuous_dictation: true,
                tts_enabled: false,
                ..Config::default()
            };
            let restored: Config =
                serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
            restored.validate().unwrap();
            assert_eq!(restored.asr_mode, *mode);
            assert!(restored.continuous_dictation && !restored.tts_enabled);
        }
    }
}

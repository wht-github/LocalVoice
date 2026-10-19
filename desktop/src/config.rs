use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

// Order shared by settings display and persistence.
pub const ASR_MODES: &[(&str, &str)] = &[
    ("sensevoice-cpu", "SenseVoice · CPU"),
    ("qwen-llama-0.6b", "llama · Qwen ASR 0.6B Q8"),
    ("qwen-llama-1.7b", "llama · Qwen ASR 1.7B Q8"),
];

pub const OUTPUT_MODES: &[(&str, &str)] = &[
    ("raw", "原样转写"),
    ("clean", "日常输入"),
    ("notes", "笔记"),
];

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Term {
    pub text: String,
    pub aliases: Vec<String>,
    pub description: String,
}

/// Human-editable rows: canonical spelling | comma-separated aliases | context.
pub fn parse_terms(source: &str) -> Result<Vec<Term>> {
    let mut terms = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split('|').map(str::trim).collect();
        if fields.len() > 3 {
            bail!(
                "术语第 {} 行请使用：规范词 | 别名, 别名 | 简短说明",
                index + 1
            );
        }
        terms.push(Term {
            text: fields[0].to_owned(),
            aliases: fields.get(1).map_or_else(Vec::new, |aliases| {
                aliases
                    .split([',', '，'])
                    .map(str::trim)
                    .filter(|alias| !alias.is_empty())
                    .map(str::to_owned)
                    .collect()
            }),
            description: fields.get(2).copied().unwrap_or_default().to_owned(),
        });
    }
    validate_terms(&terms)?;
    Ok(terms)
}

pub fn format_terms(terms: &[Term]) -> String {
    terms
        .iter()
        .map(|term| {
            format!(
                "{} | {} | {}",
                term.text,
                term.aliases.join(", "),
                term.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn validate_terms(terms: &[Term]) -> Result<()> {
    if terms.len() > 64 {
        bail!("个人术语最多 64 条，请保留常用词");
    }
    let mut spellings = HashSet::new();
    for term in terms {
        if term.aliases.len() > 8 {
            bail!("术语「{}」最多填写 8 个别名", term.text);
        }
        for spelling in std::iter::once(&term.text).chain(term.aliases.iter()) {
            if spelling.trim().is_empty()
                || spelling.trim() != spelling
                || spelling.chars().count() > 80
                || spelling.chars().any(|c| c.is_control() || c == '|')
            {
                bail!("术语和别名须为 1–80 字的单行文字，且不能包含 | 或首尾空格");
            }
            if !spellings.insert(spelling.to_lowercase()) {
                bail!("术语或别名「{spelling}」重复；每种写法只保留一次");
            }
        }
        if term.aliases.iter().any(|alias| alias.contains([',', '，'])) {
            bail!("别名不能包含逗号；请将不同别名分开填写");
        }
        if term.description.chars().count() > 160
            || term.description.trim() != term.description
            || term.description.chars().any(|c| c.is_control() || c == '|')
        {
            bail!("术语说明最多 160 字，须为单行文字且不能包含 | 或首尾空格");
        }
    }
    if serde_json::to_string(terms)?.chars().count() > 12000 {
        bail!("术语表内容过长，请精简说明或别名（含格式最多 12000 字）");
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub asr_mode: String,
    pub output_mode: String,
    pub terms: Vec<Term>,
    pub record_key: String,
    pub microphone: String,
    pub continuous_dictation: bool,
    pub show_note_preview: bool,
    pub silence_ms: u32,
    pub auto_start_services: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            asr_mode: "sensevoice-cpu".into(),
            output_mode: "raw".into(),
            terms: Vec::new(),
            record_key: "Ctrl+Alt+Space".into(),
            microphone: String::new(),
            continuous_dictation: false,
            show_note_preview: false,
            silence_ms: 700,
            auto_start_services: true,
        }
    }
}
impl Config {
    pub fn recording_continuous(&self) -> bool {
        self.continuous_dictation && matches!(self.output_mode.as_str(), "raw" | "clean")
    }

    pub fn effective_silence_ms(&self) -> u32 {
        if self.output_mode == "clean" {
            // Leave more room for a clause or spoken correction before refining it.
            self.silence_ms.max(1200)
        } else {
            self.silence_ms
        }
    }

    pub fn path() -> Result<PathBuf> {
        Ok(crate::paths::runtime()?.join("settings.json"))
    }
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
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
        if !OUTPUT_MODES
            .iter()
            .any(|(mode, _)| *mode == self.output_mode)
        {
            bail!("输出模式无效");
        }
        validate_terms(&self.terms)?;
        if !(300..=2000).contains(&self.silence_ms) {
            bail!("录音分段设置超出范围");
        }
        Ok(())
    }
    pub fn save(&self) -> Result<()> {
        self.validate()?;
        let path = Self::path()?;
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
        // Fields from removed features (TTS voice, read key, ports) are ignored.
        let config: Config = serde_json::from_str(
            r#"{"voice":"zf_xiaobei","read_key":"Ctrl+Alt+R","tts_url":"http://127.0.0.1:8002"}"#,
        )
        .unwrap();
        assert!(config.auto_start_services);
        assert!(!config.continuous_dictation);
        assert!(!config.show_note_preview);
        assert_eq!(config.output_mode, "raw");
        assert!(config.terms.is_empty());
    }

    #[test]
    fn terms_roundtrip_with_optional_columns_and_chinese_commas() {
        let terms = parse_terms("KV cache | key value cache，键值缓存 | attention 缓存\n\nFlashAttention\nBF16 | bfloat sixteen").unwrap();
        assert_eq!(terms.len(), 3);
        assert_eq!(terms[0].aliases, ["key value cache", "键值缓存"]);
        assert_eq!(parse_terms(&format_terms(&terms)).unwrap(), terms);
        let cfg = Config {
            output_mode: "clean".into(),
            terms,
            ..Config::default()
        };
        let restored: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.output_mode, "clean");
        assert_eq!(restored.terms, cfg.terms);
        assert!(parse_terms("\n  \n").unwrap().is_empty());
    }

    #[test]
    fn terms_reject_ambiguous_or_oversized_context() {
        for source in [
            "attention\nAttention",
            "KV cache | cache\ncache",
            "foo | shared\nbar | SHARED",
            " | alias",
            "foo | bar | note | extra",
        ] {
            assert!(parse_terms(source).is_err(), "{source}");
        }
        assert!(parse_terms(&"x".repeat(81)).is_err());
        assert!(parse_terms(&format!("foo | | {}", "说".repeat(161))).is_err());
        let many = (0..65)
            .map(|i| format!("term{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(parse_terms(&many).is_err());
        let aliases = (0..9)
            .map(|i| format!("alias{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_terms(&format!("foo | {aliases}")).is_err());
        let total = (0..64)
            .map(|i| format!("term{i} | | {}", "文".repeat(160)))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(parse_terms(&total).is_err());
        assert!(
            Config {
                output_mode: "unknown".into(),
                ..Config::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            validate_terms(&[Term {
                text: "foo".into(),
                aliases: vec!["a,b".into()],
                description: String::new()
            }])
            .is_err()
        );
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
    fn continuous_mode_respects_output_mode_without_changing_saved_preferences() {
        for (mode, enabled, silence_ms) in [
            ("raw", true, 700),
            ("clean", true, 1200),
            ("notes", false, 700),
        ] {
            let config = Config {
                output_mode: mode.into(),
                continuous_dictation: true,
                ..Config::default()
            };
            assert_eq!(config.recording_continuous(), enabled);
            assert_eq!(config.effective_silence_ms(), silence_ms);
            assert_eq!(config.silence_ms, 700);
            assert!(config.continuous_dictation);
        }
        let longer_pause = Config {
            output_mode: "clean".into(),
            silence_ms: 1800,
            ..Config::default()
        };
        assert_eq!(longer_pause.effective_silence_ms(), 1800);
        assert!(!longer_pause.recording_continuous());
    }
    #[test]
    fn note_preview_option_survives_settings_roundtrip() {
        assert!(!Config::default().show_note_preview);
        for enabled in [false, true] {
            let config = Config {
                show_note_preview: enabled,
                ..Config::default()
            };
            let restored: Config =
                serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
            assert_eq!(restored.show_note_preview, enabled);
        }
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
            asr_mode: "qwen-llama-0.6b".into(),
            ..Config::default()
        };
        qwen.validate().unwrap();
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&qwen).unwrap()).unwrap();
        assert_eq!(restored.asr_mode, "qwen-llama-0.6b");
        // Historical values (WSL qwen-vllm, removed sensevoice-gpu) no longer validate;
        // load() rejects them and the app falls back to defaults.
        for mode in ["qwen-native", "qwen-vllm", "sensevoice-gpu", "other"] {
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
    fn auto_start_disabled_survives_save_roundtrip() {
        let config = Config {
            auto_start_services: false,
            ..Config::default()
        };
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(!restored.auto_start_services);
    }

    #[test]
    fn all_selectable_models_roundtrip_without_changing_other_settings() {
        for (mode, _) in ASR_MODES {
            let config = Config {
                asr_mode: (*mode).into(),
                continuous_dictation: true,
                ..Config::default()
            };
            let restored: Config =
                serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
            restored.validate().unwrap();
            assert_eq!(restored.asr_mode, *mode);
            assert!(restored.continuous_dictation);
        }
    }
}

//! CPU SenseVoice inference through sherpa-onnx, without a Python service.
use anyhow::{Context, Result, bail};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};

pub struct Recognizer(OfflineRecognizer);

impl Recognizer {
    pub fn load() -> Result<Self> {
        let directory = crate::paths::runtime()?.join("models/sensevoice-int8");
        let model = directory.join("model.int8.onnx");
        let tokens = directory.join("tokens.txt");
        for path in [&model, &tokens] {
            if !path.is_file() {
                bail!(
                    "缺少 {}；{}",
                    path.display(),
                    crate::installation::setup_hint("请运行 scripts/setup-speech.ps1")
                );
            }
        }
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(model.to_string_lossy().into_owned()),
            language: Some("auto".into()),
            use_itn: true,
        };
        config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
        config.model_config.provider = Some("cpu".into());
        config.model_config.num_threads = 4;
        OfflineRecognizer::create(&config)
            .map(Self)
            .context("SenseVoice 模型加载失败，请检查原生模型文件是否完整")
    }

    pub fn transcribe(&self, samples: &[f32]) -> Result<String> {
        let stream = self.0.create_stream();
        stream.accept_waveform(16000, samples);
        self.0.decode(&stream);
        let result = stream.get_result().context("SenseVoice 未返回识别结果")?;
        Ok(result.text.trim().to_owned())
    }
}

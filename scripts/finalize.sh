#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env.sh"
UV="$VOICE_RUNTIME/bootstrap/bin/uv"
"$UV" pip check --python "$VOICE_RUNTIME/asr/bin/python"
"$UV" pip check --python "$VOICE_RUNTIME/tts/bin/python"
"$UV" pip freeze --python "$VOICE_RUNTIME/asr/bin/python" > "$PROJECT_DIR/.runtime/asr-lock.txt"
"$UV" pip freeze --python "$VOICE_RUNTIME/tts/bin/python" > "$PROJECT_DIR/.runtime/tts-lock.txt"
model_dir="$MODELSCOPE_CACHE/models/iic--SenseVoiceSmall/snapshots/master"
if [[ ! -f "$model_dir/model.pt" ]]; then
    model_dir=$("$VOICE_RUNTIME/asr/bin/python" -c 'from huggingface_hub import snapshot_download; print(snapshot_download("FunAudioLLM/SenseVoiceSmall", local_files_only=True))')
fi
test -f "$model_dir/model.pt"
tts_backend=kokoro
if [[ -f "$VOICE_RUNTIME/service.env" ]]; then
    tts_backend=$(sed -n 's/^TTS_BACKEND=//p' "$VOICE_RUNTIME/service.env")
    tts_backend=${tts_backend:-kokoro}
fi
cat > "$VOICE_RUNTIME/service.env" <<EOF
ASR_MODEL=$model_dir
HF_HUB_OFFLINE=1
TRANSFORMERS_OFFLINE=1
TTS_BACKEND=$tts_backend
EOF
echo 'Configured cached local models and saved dependency versions.'

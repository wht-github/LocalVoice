#!/usr/bin/env bash
set -euo pipefail
if [[ "${WSL_DISTRO_NAME:-}" != LocalVoice ]]; then
    echo 'Run setup only inside the dedicated LocalVoice instance.' >&2
    exit 1
fi
source "$(dirname "$0")/env.sh"
if [[ ! -x "$VOICE_RUNTIME/bootstrap/bin/uv" ]]; then
    python3 -m venv "$VOICE_RUNTIME/bootstrap"
    "$VOICE_RUNTIME/bootstrap/bin/python" -m pip install uv -i "$UV_DEFAULT_INDEX"
fi
UV="$VOICE_RUNTIME/bootstrap/bin/uv"
"$UV" venv --python /usr/bin/python3.12 --allow-existing "$VOICE_RUNTIME/asr"
"$UV" venv --python /usr/bin/python3.12 --allow-existing "$VOICE_RUNTIME/tts"
python3 "$PROJECT_DIR/scripts/download_torch.py"
TORCH_WHEEL="$VOICE_RUNTIME/downloads/torch-2.9.1+cpu-cp312-cp312-manylinux_2_28_x86_64.whl"
"$UV" pip install --python "$VOICE_RUNTIME/asr/bin/python" "$TORCH_WHEEL" 'https://download.pytorch.org/whl/cpu/torchaudio-2.9.1%2Bcpu-cp312-cp312-manylinux_2_28_x86_64.whl'
"$UV" pip install --python "$VOICE_RUNTIME/asr/bin/python" -r "$PROJECT_DIR/requirements-asr.txt"
"$UV" pip install --python "$VOICE_RUNTIME/tts/bin/python" "$TORCH_WHEEL"
"$UV" pip install --python "$VOICE_RUNTIME/tts/bin/python" -r "$PROJECT_DIR/requirements-tts.txt"
curl -fL --retry 3 --max-time 120 "$HF_ENDPOINT/spacy/en_core_web_sm/resolve/main/en_core_web_sm-any-py3-none-any.whl" \
    -o "$VOICE_RUNTIME/downloads/en_core_web_sm-3.7.1-py3-none-any.whl"
"$UV" pip install --python "$VOICE_RUNTIME/tts/bin/python" "$VOICE_RUNTIME/downloads/en_core_web_sm-3.7.1-py3-none-any.whl"
"$UV" pip check --python "$VOICE_RUNTIME/asr/bin/python"
"$UV" pip check --python "$VOICE_RUNTIME/tts/bin/python"
"$UV" pip freeze --python "$VOICE_RUNTIME/asr/bin/python" > "$PROJECT_DIR/.runtime/asr-lock.txt"
"$UV" pip freeze --python "$VOICE_RUNTIME/tts/bin/python" > "$PROJECT_DIR/.runtime/tts-lock.txt"

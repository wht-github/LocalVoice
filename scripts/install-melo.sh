#!/usr/bin/env bash
set -euo pipefail
[[ "${WSL_DISTRO_NAME:-}" == LocalVoice && "$USER" == voice ]]
source "$(dirname "$0")/env.sh"
UV="$VOICE_RUNTIME/bootstrap/bin/uv"
REV=209145371cff8fc3bd60d7be902ea69cbdb7965a
SOURCE="$VOICE_RUNTIME/melo-source-$REV"
export HF_HUB_OFFLINE=0 TRANSFORMERS_OFFLINE=0 CUDA_VISIBLE_DEVICES=""
export NLTK_DATA="$VOICE_RUNTIME/nltk_data"
mkdir -p "$SOURCE" "$NLTK_DATA"
if [[ ! -f "$SOURCE/melo/api.py" ]]; then
    curl -fL --retry 3 "https://codeload.github.com/myshell-ai/MeloTTS/tar.gz/$REV" -o "$VOICE_RUNTIME/downloads/melo-$REV.tar.gz"
    tar -xf "$VOICE_RUNTIME/downloads/melo-$REV.tar.gz" --strip-components=1 -C "$SOURCE"
fi
python3 "$PROJECT_DIR/scripts/prepare-melo.py" "$SOURCE"
[[ -x "$VOICE_RUNTIME/tts-melo/bin/python" ]] || "$UV" venv --python /usr/bin/python3 "$VOICE_RUNTIME/tts-melo"
PY="$VOICE_RUNTIME/tts-melo/bin/python"
# Reuse the already downloaded CPU torch wheel; ordinary Python packages use TUNA.
"$UV" pip install --python "$PY" "$VOICE_RUNTIME/downloads/torch-2.9.1+cpu-cp312-cp312-manylinux_2_28_x86_64.whl" -r "$PROJECT_DIR/requirements-melo.txt"
"$UV" pip install --python "$PY" 'https://download.pytorch.org/whl/cpu/torchaudio-2.9.1%2Bcpu-cp312-cp312-manylinux_2_28_x86_64.whl'
SITE=$("$PY" -c 'import site; print(site.getsitepackages()[0])')
printf '%s\n' "$SOURCE" > "$SITE/melo-source.pth"
"$PY" "$PROJECT_DIR/scripts/download-hf-ranges.py" myshell-ai/MeloTTS-Chinese checkpoint.pth
"$PY" "$PROJECT_DIR/scripts/download-hf-ranges.py" bert-base-multilingual-uncased model.safetensors
"$PY" "$PROJECT_DIR/scripts/download-melo.py"

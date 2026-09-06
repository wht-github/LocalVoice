#!/usr/bin/env bash
set -euo pipefail
[[ "${WSL_DISTRO_NAME:-}" == LocalVoice && "$EUID" != 0 ]]
source "$(dirname "$0")/env.sh"
UV="$VOICE_RUNTIME/bootstrap/bin/uv"
if [[ ! -x "$VOICE_RUNTIME/asr-gpu/bin/python" ]]; then
    "$UV" venv --python /usr/bin/python3.12 "$VOICE_RUNTIME/asr-gpu"
fi
"$UV" pip install --python "$VOICE_RUNTIME/asr-gpu/bin/python" -r "$PROJECT_DIR/requirements-asr-gpu.txt"
"$UV" pip check --python "$VOICE_RUNTIME/asr-gpu/bin/python"
"$UV" pip freeze --python "$VOICE_RUNTIME/asr-gpu/bin/python" > "$PROJECT_DIR/.runtime/asr-gpu-lock.txt"

#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env.sh"
cd "$PROJECT_DIR"
case "${1:-}" in
    asr)
        export ASR_BACKEND=sensevoice-cpu
        export CUDA_VISIBLE_DEVICES=""
        exec "$VOICE_RUNTIME/asr/bin/python" -u asr_server.py ;;
    tts)
        export CUDA_VISIBLE_DEVICES=""
        export NLTK_DATA="$VOICE_RUNTIME/nltk_data"
        if [[ "${TTS_BACKEND:-kokoro}" == melo ]]; then
            exec "$VOICE_RUNTIME/tts-melo/bin/python" -u tts_server.py
        fi
        exec "$VOICE_RUNTIME/tts/bin/python" -u tts_server.py ;;
    *) echo 'Usage: run-service.sh asr|tts' >&2; exit 2 ;;
esac

#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env.sh"
exec "$VOICE_RUNTIME/tts/bin/python" "$PROJECT_DIR/smoke_test.py"

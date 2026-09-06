#!/usr/bin/env bash
set -euo pipefail
[[ "${WSL_DISTRO_NAME:-}" == LocalVoice && "$EUID" == 0 ]]
RUNTIME=/home/voice/.local/share/local-voice-app
case "${1:-}" in
    melo) test -x "$RUNTIME/tts-melo/bin/python" ;;
    kokoro) test -x "$RUNTIME/tts/bin/python" ;;
    *) echo 'Usage: select-tts.sh melo|kokoro' >&2; exit 2 ;;
esac
if [[ ! -f "$RUNTIME/service.env.before-melo" ]]; then
    cp "$RUNTIME/service.env" "$RUNTIME/service.env.before-melo"
fi
sed '/^TTS_BACKEND=/d' "$RUNTIME/service.env" > "$RUNTIME/service.env.next"
printf 'TTS_BACKEND=%s\n' "$1" >> "$RUNTIME/service.env.next"
mv "$RUNTIME/service.env.next" "$RUNTIME/service.env"
systemctl restart local-voice-tts

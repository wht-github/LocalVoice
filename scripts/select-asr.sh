#!/usr/bin/env bash
set -euo pipefail
[[ "${WSL_DISTRO_NAME:-}" == LocalVoice && "$EUID" == 0 ]]
RUNTIME=/home/voice/.local/share/local-voice-app
mode=${1:-}
case "$mode" in
    sensevoice-cpu) test -x "$RUNTIME/asr/bin/python" ;;
    qwen-vllm) test -x "$RUNTIME/asr-gpu/bin/python"; test -s "$RUNTIME/qwen-model.path" ;;
    *) echo 'Usage: select-asr.sh sensevoice-cpu|qwen-vllm' >&2; exit 2 ;;
esac
exec 9>"$RUNTIME/asr-switch.lock"
flock -w 210 9
wait_ready() {
    /usr/bin/python3 - "$1" "$2" <<'PY'
import json, sys, time, urllib.request
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
deadline = time.monotonic() + int(sys.argv[2])
while time.monotonic() < deadline:
    try:
        with opener.open('http://127.0.0.1:8001/health', timeout=2) as response:
            health = json.load(response)
        if health.get('ready') and health.get('mode', 'sensevoice-cpu') == sys.argv[1]:
            sys.exit(0)
    except Exception:
        pass
    time.sleep(.5)
sys.exit(1)
PY
}
previous=$(sed -n 's/^ASR_BACKEND=//p' "$RUNTIME/service.env")
previous=${previous:-sensevoice-cpu}
if [[ "$previous" == "$mode" ]]; then
    systemctl start local-voice-asr
    wait_ready "$mode" 180
    exit
fi
backup=$(mktemp "$RUNTIME/service.env.switch.XXXXXX")
cp "$RUNTIME/service.env" "$backup"
trap 'rm -f "$backup"' EXIT
sed '/^ASR_BACKEND=/d' "$backup" > "$RUNTIME/service.env.next"
printf 'ASR_BACKEND=%s\n' "$mode" >> "$RUNTIME/service.env.next"
mv "$RUNTIME/service.env.next" "$RUNTIME/service.env"
systemctl restart local-voice-asr
if wait_ready "$mode" 180; then
    echo "STT ready: $mode"
else
    cp "$backup" "$RUNTIME/service.env"
    systemctl restart local-voice-asr
    wait_ready "$previous" 90 || true
    echo "STT switch failed; restored $previous. Inspect journalctl -u local-voice-asr." >&2
    exit 1
fi

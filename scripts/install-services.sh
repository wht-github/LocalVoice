#!/usr/bin/env bash
set -euo pipefail
if [[ "${WSL_DISTRO_NAME:-}" != LocalVoice || "$EUID" != 0 ]]; then
    echo 'Run as root inside LocalVoice only.' >&2
    exit 1
fi
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DEPLOY_DIR=/opt/local-voice-app
install -d -o voice -g voice "$DEPLOY_DIR" "$DEPLOY_DIR/scripts"
for file in asr_server.py tts_server.py tester.html smoke_test.py; do
    if [[ "$PROJECT_DIR" != "$DEPLOY_DIR" ]]; then
        install -m 644 "$PROJECT_DIR/$file" "$DEPLOY_DIR/$file"
    fi
done
for file in env.sh run-service.sh smoke-test.sh select-asr.sh install-asr-gpu.sh profile-asr.py download-qwen-asr.py; do
    if [[ "$PROJECT_DIR" != "$DEPLOY_DIR" ]]; then
        install -m 644 "$PROJECT_DIR/scripts/$file" "$DEPLOY_DIR/scripts/$file"
    fi
done
for service in asr tts; do
    cat > "/etc/systemd/system/local-voice-$service.service" <<EOF
[Unit]
Description=Local voice $service
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=voice
Environment=HOME=/home/voice
Environment=WSL_DISTRO_NAME=LocalVoice
EnvironmentFile=-/home/voice/.local/share/local-voice-app/service.env
WorkingDirectory=$DEPLOY_DIR
ExecStart=/bin/bash $DEPLOY_DIR/scripts/run-service.sh $service
KillMode=control-group
TimeoutStopSec=30
Restart=no

[Install]
WantedBy=multi-user.target
EOF
done
systemctl daemon-reload
# Services start on demand; do not enable at boot until requested.

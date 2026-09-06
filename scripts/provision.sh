#!/usr/bin/env bash
# Run as root only inside the dedicated LocalVoice Ubuntu 24.04 instance.
set -euo pipefail
if [[ "${WSL_DISTRO_NAME:-}" != LocalVoice ]]; then
    echo 'Refusing to provision anything except the LocalVoice WSL instance.' >&2
    exit 1
fi
export DEBIAN_FRONTEND=noninteractive
bash "$(dirname "$0")/configure-mirrors.sh"
apt-get update
apt-get install -y --no-install-recommends python3.12-venv python3.12-dev \
    ca-certificates curl build-essential libsndfile1 espeak-ng ffmpeg
getent group voice >/dev/null || groupadd voice
id voice >/dev/null 2>&1 || useradd --create-home --gid voice --shell /bin/bash voice
install -d -o voice -g voice /home/voice/.local/share/local-voice-app
if [[ ! -f /etc/wsl.conf ]]; then
cat > /etc/wsl.conf <<'EOF'
[boot]
systemd=true
[user]
default=voice
EOF
fi

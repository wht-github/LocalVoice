#!/usr/bin/env bash
# Scope: LocalVoice only. Never edit the Windows-wide .wslconfig here.
set -euo pipefail
if [[ "${WSL_DISTRO_NAME:-}" != LocalVoice || "$EUID" != 0 ]]; then
    echo 'Run as root inside LocalVoice only.' >&2
    exit 1
fi
python3 - <<'PY'
import configparser
import shutil
from pathlib import Path
path = Path('/etc/wsl.conf')
backup = Path('/etc/wsl.conf.before-local-voice-isolation')
if path.exists() and not backup.exists():
    shutil.copy2(path, backup)
cfg = configparser.ConfigParser()
cfg.read(path)
for section, values in {
    'boot': {'systemd': 'true'},
    'user': {'default': 'voice'},
    'automount': {'enabled': 'false', 'mountFsTab': 'false'},
    'interop': {'enabled': 'false', 'appendWindowsPath': 'false'},
}.items():
    if not cfg.has_section(section):
        cfg.add_section(section)
    for key, value in values.items():
        cfg.set(section, key, value)
with path.open('w') as f:
    cfg.write(f)
PY

cat > /etc/systemd/system/localvoice.slice <<'EOF'
[Unit]
Description=Dedicated local voice services resource budget

[Slice]
CPUAccounting=yes
MemoryAccounting=yes
CPUQuota=600%
MemoryHigh=6G
MemoryMax=8G
EOF

install -d -o voice -g voice /opt/local-voice-app/.runtime /opt/local-voice-app/outputs
for service in asr tts; do
    dir="/etc/systemd/system/local-voice-$service.service.d"
    install -d "$dir"
    cat > "$dir/isolation.conf" <<'EOF'
[Service]
Slice=localvoice.slice
NoNewPrivileges=yes
CapabilityBoundingSet=
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=read-only
ProtectKernelTunables=yes
ProtectControlGroups=yes
RestrictSUIDSGID=yes
InaccessiblePaths=-/mnt -/media -/run/desktop -/run/WSL
ReadWritePaths=/home/voice/.local/share/local-voice-app /opt/local-voice-app/.runtime /opt/local-voice-app/outputs
EOF
done
systemctl daemon-reload
echo 'LocalVoice isolation installed; terminate and restart only LocalVoice to apply WSL settings.'

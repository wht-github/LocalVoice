#!/usr/bin/env bash
# Persist mirror defaults only in the dedicated voice appliance.
set -euo pipefail
if [[ "${WSL_DISTRO_NAME:-}" != LocalVoice || "$EUID" != 0 ]]; then
    echo 'Run as root inside LocalVoice only.' >&2
    exit 1
fi
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
python3 - <<'PY'
from pathlib import Path
import configparser
import shutil

source = Path('/etc/apt/sources.list.d/ubuntu.sources')
backup = source.with_name('ubuntu.sources.before-local-voice-mirrors')
if not backup.exists():
    shutil.copy2(source, backup)
text = source.read_text()
for origin in ('http://archive.ubuntu.com/ubuntu/', 'http://security.ubuntu.com/ubuntu/',
               'https://archive.ubuntu.com/ubuntu/', 'https://security.ubuntu.com/ubuntu/'):
    text = text.replace(origin, 'https://mirrors.tuna.tsinghua.edu.cn/ubuntu/')
source.write_text(text)

# /etc/environment covers new sessions; profile.d also covers login shells.
environment = Path('/etc/environment')
backup = environment.with_name('environment.before-local-voice-mirrors')
if not backup.exists():
    shutil.copy2(environment, backup)
values = {
    'UV_DEFAULT_INDEX': 'https://pypi.tuna.tsinghua.edu.cn/simple',
    'PIP_INDEX_URL': 'https://pypi.tuna.tsinghua.edu.cn/simple',
    'HF_ENDPOINT': 'https://hf-mirror.com',
    'HF_HUB_DISABLE_XET': '1',
}
lines = [line for line in environment.read_text().splitlines()
         if line.split('=', 1)[0].strip() not in values]
lines += [f'{key}="{value}"' for key, value in values.items()]
environment.write_text('\n'.join(lines) + '\n')
Path('/etc/profile.d/local-voice-mirrors.sh').write_text(
    ''.join(f'export {key}="{value}"\n' for key, value in values.items()))

pip_path = Path('/etc/pip.conf')
pip_backup = pip_path.with_name('pip.conf.before-local-voice-mirrors')
if pip_path.exists() and not pip_backup.exists():
    shutil.copy2(pip_path, pip_backup)
pip_config = configparser.ConfigParser()
pip_config.read(pip_path)
if not pip_config.has_section('global'):
    pip_config.add_section('global')
pip_config.set('global', 'index-url', values['PIP_INDEX_URL'])
with pip_path.open('w') as stream:
    pip_config.write(stream)
PY
install -d /etc/uv
if [[ -f /etc/uv/uv.toml && ! -f /etc/uv/uv.toml.before-local-voice-mirrors ]]; then
    cp -p /etc/uv/uv.toml /etc/uv/uv.toml.before-local-voice-mirrors
fi
install -m 644 "$PROJECT_DIR/uv.toml" /etc/uv/uv.toml
echo 'Configured TUNA APT/uv/pip defaults and HF model mirror in LocalVoice.'

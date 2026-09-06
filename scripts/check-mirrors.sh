#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env.sh"
# Confirm the system-wide uv configuration works without shell/project overrides.
cd /tmp
env -u UV_DEFAULT_INDEX -u UV_INDEX_URL -u UV_INDEX -u UV_EXTRA_INDEX_URL \
    "$VOICE_RUNTIME/bootstrap/bin/uv" pip install --dry-run --no-cache -v \
    --python "$VOICE_RUNTIME/bootstrap/bin/python" packaging==26.3
"$VOICE_RUNTIME/asr/bin/python" - <<'PY'
import os
import tempfile
from huggingface_hub import HfApi, hf_hub_download

endpoint = HfApi().endpoint
assert endpoint == 'https://hf-mirror.com', endpoint
for repo, name in [('FunAudioLLM/SenseVoiceSmall', 'config.yaml'),
                   ('hexgrad/Kokoro-82M', 'config.json')]:
    with tempfile.TemporaryDirectory(prefix='voice-mirror-check-') as cache:
        path = hf_hub_download(repo, name, local_dir=cache, endpoint=endpoint)
        assert os.path.getsize(path) > 0
        print(f'Model mirror OK: {endpoint}/{repo}/{name}', flush=True)
PY

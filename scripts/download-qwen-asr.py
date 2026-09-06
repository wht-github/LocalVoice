"""Download model weights via the configured HF mirror, preserving verified cache."""
import os
from pathlib import Path
import subprocess
import sys
from huggingface_hub import snapshot_download

root = Path(__file__).resolve().parents[1]
runtime = Path(os.environ['VOICE_RUNTIME'])
repo = 'Qwen/Qwen3-ASR-0.6B'
subprocess.run([sys.executable, str(root / 'scripts/download-hf-ranges.py'), repo, 'model.safetensors'], check=True)
snapshot = snapshot_download(repo, allow_patterns=['*.json', '*.txt'], max_workers=3)
(runtime / 'qwen-model.path').write_text(snapshot + '\n')
print('Model snapshot:', snapshot, flush=True)

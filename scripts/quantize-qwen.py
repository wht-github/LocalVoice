"""Produce three local quantizations directly from the pinned BF16 source."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
MODELS = ROOT / '.runtime/models/Qwen3-ASR-1.7B-GGUF'


def digest(path):
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()


def main():
    source = MODELS / 'Qwen3-ASR-1.7B-bf16.gguf'
    if digest(source) != '1af18763dfafde2bbf071ef8a0952f7bee66f140c3565e5ccc5afb07dc1f9227':
        raise RuntimeError('BF16 source checksum mismatch')
    env = os.environ.copy()
    env['PATH'] = str(ROOT / '.venv-llama-build/Lib/site-packages/nvidia/cu13/bin/x86_64') + os.pathsep + env['PATH']
    records = []
    for quant in ('Q8_0', 'Q6_K', 'Q4_K_M'):
        output = MODELS / f'Qwen3-ASR-1.7B-local-{quant}.gguf'
        command = [str(ROOT / '.runtime/llama-build/bin/llama-quantize.exe'), str(source), str(output), quant, '4']
        start = time.perf_counter()
        with (MODELS / f'{quant}-quantize.log').open('w', encoding='utf-8') as log:
            subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                           creationflags=subprocess.CREATE_NO_WINDOW, check=True)
        records.append({'type': quant, 'command': command, 'seconds': time.perf_counter() - start,
                        'bytes': output.stat().st_size, 'sha256': digest(output)})
        print(f'{quant}: {output.stat().st_size / 1e9:.3f} GB; {records[-1]["seconds"]:.1f}s', flush=True)
    path = ROOT / 'experiments/qwen_asr/quantization.json'
    path.write_text(json.dumps({'source_sha256': digest(source),
        'source_commit': subprocess.check_output(['git', '-C', str(ROOT / 'external/llama.cpp'), 'rev-parse', 'HEAD'], text=True).strip(),
        'models': records}, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()

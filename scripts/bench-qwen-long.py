"""Lean qwen-vllm benchmark on the pre-assembled long mixed recording.

Reuses outputs/long-mixed/ascend-mixed-long.wav + reference.txt; replicates the
pause-splitting and MER scoring of benchmark_long_mixed.py without pyarrow.
"""
import io
import json
import re
import subprocess
import time
import unicodedata
from pathlib import Path

import httpx
import numpy as np
import soundfile as sf

OUT = Path('/opt/local-voice-app/outputs/long-mixed')
RATE = 16000
combined, rate = sf.read(OUT / 'ascend-mixed-long.wav', dtype='float32')
assert rate == RATE and combined.ndim == 1
reference = (OUT / 'reference.txt').read_text(encoding='utf-8')


def tokens(text):
    return re.findall(r"[\u3400-\u9fff]|[a-z]+(?:'[a-z]+)?|[0-9]+|[^\W\d_]+", unicodedata.normalize('NFKC', text).lower())


def distance(a, b):
    prev = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        row = [i]
        for j, y in enumerate(b, 1):
            row.append(min(row[-1] + 1, prev[j] + 1, prev[j - 1] + (x != y)))
        prev = row
    return prev[-1]


frame = 320
padded = np.pad(combined, (0, (-len(combined)) % frame))
rms = np.sqrt(np.mean(padded.reshape(-1, frame) ** 2, axis=1))
quiet = rms < .003
pauses = []
start = None
for i, flag in enumerate(np.append(quiet, False)):
    if flag and start is None:
        start = i
    elif not flag and start is not None:
        if (i - start) * .02 >= .45:
            pauses.append(int((start + i) / 2) * frame)
        start = None
boundaries = [0]
while len(combined) - boundaries[-1] > 25 * RATE:
    begin = boundaries[-1]
    candidates = [p for p in pauses if begin + 8 * RATE <= p <= begin + 25 * RATE]
    boundaries.append(candidates[0] if candidates else begin + 25 * RATE)
boundaries.append(len(combined))

pid = int(subprocess.check_output(['systemctl', 'show', 'local-voice-asr', '-p', 'MainPID', '--value'], text=True))


def mem():
    lines = Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines()
    return {line.split(':')[0]: int(line.split()[1]) / 1024 for line in lines if line.startswith(('Pss:', 'Rss:'))}


def vram_mib():
    try:
        out = subprocess.check_output(['nvidia-smi', '--query-gpu=memory.used', '--format=csv,noheader,nounits'], text=True)
        return int(out.strip().splitlines()[0])
    except Exception:
        return None


def wav(samples):
    buffer = io.BytesIO()
    sf.write(buffer, samples, RATE, format='WAV', subtype='PCM_16')
    return buffer.getvalue()


result = {'backend': 'qwen-vllm', 'model': 'Qwen/Qwen3-ASR-0.6B', 'device': 'cuda:0',
          'audio_seconds': len(combined) / RATE, 'segments': len(boundaries) - 1,
          'scoring': 'same as benchmark_long_mixed.py', 'runs': {}}

with httpx.Client(timeout=300, trust_env=False) as client:
    assert client.get('http://127.0.0.1:8001/health').json()['ready']
    for name, lang in [('long_pause_auto', 'auto'), ('long_pause_zh', 'zh')]:
        measurements = []
        started = time.perf_counter()
        for index, (a, b) in enumerate(zip(boundaries, boundaries[1:])):
            audio = combined[a:b]
            t = time.perf_counter()
            response = client.post('http://127.0.0.1:8001/v1/audio/transcriptions',
                                   files={'file': ('segment.wav', wav(audio), 'audio/wav')}, data={'language': lang})
            response.raise_for_status()
            answer = response.json()
            measurements.append({'id': f'{a/RATE:.2f}-{b/RATE:.2f}', 'duration': len(audio) / RATE,
                                 'wall_seconds': time.perf_counter() - t, 'inference_seconds': answer['inference_seconds'],
                                 'text': answer['text'], 'detected_language': answer['language'],
                                 'pss_mib': mem()['Pss'], 'vram_mib': vram_mib()})
            if (index + 1) % 10 == 0:
                print(f'{name}: {index + 1}/{len(boundaries) - 1}', flush=True)
        wall = time.perf_counter() - started
        text = ' '.join(m['text'] for m in measurements)
        errors = distance(tokens(reference), tokens(text))
        length = len(tokens(reference))
        summary = {'segments': len(measurements), 'wall_seconds': wall,
                   'rtf': wall / sum(m['duration'] for m in measurements),
                   'p95_request_seconds': float(np.percentile([m['wall_seconds'] for m in measurements], 95)),
                   'max_request_seconds': max(m['wall_seconds'] for m in measurements),
                   'max_segment_rtf': max(m['wall_seconds'] / m['duration'] for m in measurements),
                   'errors': errors, 'reference_tokens': length, 'mer': errors / length,
                   'empty_results': sum(not m['text'].strip() for m in measurements),
                   'pss_peak_mib': max(m['pss_mib'] for m in measurements),
                   'vram_peak_mib': max((m['vram_mib'] or 0) for m in measurements),
                   'measurements': measurements}
        result['runs'][name] = summary
        (OUT / f'{name}-qwen.txt').write_text(text, encoding='utf-8')
        (OUT / 'results-qwen-vllm.json').write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
        print(name, json.dumps({k: v for k, v in summary.items() if k != 'measurements'}, ensure_ascii=False), flush=True)
print('Finished:', OUT / 'results-qwen-vllm.json', flush=True)

"""Measure one selected STT backend with known prerecorded audio, no microphone."""
import concurrent.futures
import io
import json
from pathlib import Path
import subprocess
import sys
import time

import httpx
import numpy as np
import soundfile as sf

ROOT = Path(__file__).resolve().parents[1]
mode = sys.argv[1]
client = httpx.Client(timeout=90, trust_env=False)
health = client.get('http://127.0.0.1:8001/health').json()
assert health.get('mode', 'sensevoice-cpu') == mode, health
cg = Path('/sys/fs/cgroup') / subprocess.check_output([
    'systemctl', 'show', 'local-voice-asr', '-p', 'ControlGroup', '--value'], text=True).strip().lstrip('/')

def sample():
    totals = {'Pss': 0, 'Rss': 0, 'Pss_Anon': 0, 'Pss_File': 0}
    for pid in (cg / 'cgroup.procs').read_text().split():
        try:
            for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
                key, _, value = line.partition(':')
                if key in totals:
                    totals[key] += int(value.split()[0]) / 1024
        except FileNotFoundError:
            pass
    raw = subprocess.check_output(['/usr/lib/wsl/lib/nvidia-smi', '--query-gpu=memory.used', '--format=csv,noheader,nounits'], text=True)
    cpu = dict(line.split() for line in (cg / 'cpu.stat').read_text().splitlines())
    return {'seconds': time.monotonic(), **{k + '_MiB': round(v, 1) for k, v in totals.items()},
            'gpu_total_used_MiB': int(raw.strip()), 'cpu_usec': int(cpu['usage_usec']),
            'cgroup_MiB': round(int((cg / 'memory.current').read_text()) / 2**20, 1)}

audio, rate = sf.read(ROOT / 'outputs/melo-mixed.wav', dtype='float32')
audio = np.tile(audio, int(np.ceil(60 * rate / len(audio))))[:60 * rate]

def request(samples):
    buf = io.BytesIO()
    sf.write(buf, samples, rate, format='WAV')
    started = time.monotonic()
    response = client.post('http://127.0.0.1:8001/v1/audio/transcriptions',
                           files={'file': ('known.wav', buf.getvalue(), 'audio/wav')}, data={'language': 'auto'})
    response.raise_for_status()
    return {'http_seconds': round(time.monotonic() - started, 3), **response.json()}

request(audio[:rate * 5])
baseline = sample()
samples = [baseline]
results = []
with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
    for start, end in [(0, 25), (25, 50), (50, 60)]:
        job = pool.submit(request, audio[start * rate:end * rate])
        while not job.done():
            samples.append(sample())
            time.sleep(.3)
        results.append(job.result())
samples.append(sample())
report = {'mode': mode, 'health': health, 'baseline': baseline, 'after': samples[-1],
          'peak_pss_MiB': max(s['Pss_MiB'] for s in samples),
          'peak_gpu_total_used_MiB': max(s['gpu_total_used_MiB'] for s in samples),
          'average_cpu_cores': round((samples[-1]['cpu_usec'] - baseline['cpu_usec']) / 1e6 /
                                     (samples[-1]['seconds'] - baseline['seconds']), 2),
          'results': results, 'total_audio_seconds': 60,
          'total_http_seconds': round(sum(r['http_seconds'] for r in results), 3),
          'note': 'Known mixed-language TTS audio repeated to 60 seconds; not an accuracy benchmark. GPU total includes Windows apps. PSS sums all service processes.'}
(ROOT / f'outputs/asr-{mode}-profile.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
print(json.dumps(report, ensure_ascii=False, indent=2))

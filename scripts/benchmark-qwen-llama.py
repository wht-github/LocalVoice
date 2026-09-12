"""Measure native llama.cpp 1.7B Q8 ASR and the existing PyTorch 0.6B reference."""
import argparse
import base64
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import socket
import statistics
import subprocess
import time
import urllib.error
from qwen_eval_common import MemorySampler, request_json

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault('HF_HOME', str(ROOT / '.runtime/qwen-hf'))
os.environ.setdefault('HF_HUB_OFFLINE', '1')
os.environ.setdefault('NUMBA_CACHE_DIR', str(ROOT / '.runtime/numba-cache'))



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--backend', choices=['llama', 'pytorch'], default='llama')
    parser.add_argument('--repeat', type=int, default=3)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error('--repeat must be positive')
    import librosa
    import numpy as np
    import soundfile as sf
    output = args.output or ROOT / 'outputs/qwen-llama' / datetime.now().strftime('%Y%m%d-%H%M%S')
    output.mkdir(parents=True, exist_ok=True)
    zh, _ = librosa.load(ROOT / 'outputs/melo-mandarin.wav', sr=16000, mono=True)
    mixed, _ = librosa.load(ROOT / 'outputs/melo-mixed.wav', sr=16000, mono=True)
    cases = [('short', zh[:48000]), ('mandarin', zh), ('mixed', mixed),
             ('long', np.concatenate([zh, np.zeros(8000, np.float32), mixed]))]
    audio_data = {}
    for name, audio in cases:
        path = output / f'{name}.wav'
        sf.write(path, audio, 16000, subtype='FLOAT')
        audio_data[name] = base64.b64encode(path.read_bytes()).decode('ascii')
    sampler = MemorySampler()
    report = {
        'backend': args.backend, 'runs': [], 'warmups': [],
        'device_memory_before_load_mib': sampler.read(),
        'audio': {name: {'seconds': len(audio) / 16000,
                         'float32_sha256': hashlib.sha256(audio.tobytes()).hexdigest()} for name, audio in cases},
        'limitations': ['Synthetic speech only; no human-labelled accuracy test.',
                       '1.7B Q8 llama.cpp versus 0.6B FP16 PyTorch is a deployment comparison, not a backend-only comparison.',
                       'VRAM is sampled device-wide, including desktop and other applications.',
                       'llama latency includes local HTTP, WAV decoding, audio processing and generation; excludes base64 encoding.',
                       'PyTorch latency includes audio processing, GPU transfer, generation and text decoding; excludes file I/O.',
                       'Warm timings exclude model loading and per-case warmup. GPU clocks/background load are not controlled.'],
    }
    server = None
    log = None
    sampler.start()

    def save():
        (output / f'{args.backend}.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')

    try:
        if args.backend == 'llama':
            model_dir = ROOT / '.runtime/models/Qwen3-ASR-1.7B-GGUF'
            report['model'] = json.loads((model_dir / 'manifest.json').read_text())
            report['source_commit'] = subprocess.check_output(['git', '-C', str(ROOT / 'external/llama.cpp'), 'rev-parse', 'HEAD'], text=True).strip()
            cuda = ROOT / '.venv-llama-build/Lib/site-packages/nvidia/cu13'
            env = os.environ.copy()
            env['PATH'] = str(cuda / 'bin/x86_64') + os.pathsep + env['PATH']
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0))
                port = sock.getsockname()[1]
            endpoint = f'http://127.0.0.1:{port}'
            command = [str(ROOT / '.runtime/llama-build/bin/llama-server.exe'),
                       '-m', str(model_dir / 'Qwen3-ASR-1.7B-Q8_0.gguf'),
                       '--mmproj', str(model_dir / 'mmproj-Qwen3-ASR-1.7B-bf16.gguf'),
                       '--host', '127.0.0.1', '--port', str(port), '-ngl', '99',
                       '-c', '2048', '-np', '1', '-t', '4', '-tb', '4',
                       '--cache-ram', '0', '--no-context-shift', '--temp', '0', '-n', '1024']
            report['command'] = command
            log = (output / 'server.log').open('w', encoding='utf-8')
            start = time.perf_counter()
            server = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                      creationflags=subprocess.CREATE_NO_WINDOW)
            while True:
                if server.poll() is not None:
                    raise RuntimeError(f'llama-server exited: {server.returncode}; see {output / "server.log"}')
                try:
                    if request_json(endpoint + '/health').get('status') == 'ok':
                        break
                except (OSError, urllib.error.URLError):
                    pass
                if time.perf_counter() - start > 180:
                    raise TimeoutError('Server readiness timed out')
                time.sleep(0.5)
            report['load_and_server_ready_seconds'] = time.perf_counter() - start

            def infer(name, audio):
                result = request_json(endpoint + '/v1/chat/completions', {
                    'messages': [{'role': 'user', 'content': [
                        {'type': 'input_audio', 'input_audio': {'data': audio_data[name], 'format': 'wav'}}]}],
                    'temperature': 0, 'max_tokens': 1024, 'cache_prompt': False,
                    'stream': False, 'seed': 42,
                })
                choice = result['choices'][0]
                if choice['finish_reason'] != 'stop':
                    raise RuntimeError(f'Incomplete transcription: {choice}')
                return {'text': choice['message']['content'], 'response': result}
        else:
            import torch
            import transformers
            from transformers import AutoModelForMultimodalLM, AutoProcessor
            torch.set_num_threads(4)
            model_id = 'Qwen/Qwen3-ASR-0.6B-hf'
            start = time.perf_counter()
            processor = AutoProcessor.from_pretrained(model_id, local_files_only=True)
            model = AutoModelForMultimodalLM.from_pretrained(model_id, dtype=torch.float16,
                        attn_implementation='sdpa', local_files_only=True).to('cuda').eval()
            torch.cuda.synchronize()
            report.update(model=model_id, dtype='float16', torch=torch.__version__,
                          transformers=transformers.__version__, load_seconds=time.perf_counter() - start)

            @torch.inference_mode()
            def infer(name, audio):
                inputs = processor.apply_transcription_request(audio=audio, language=None).to(model.device, model.dtype)
                result = model.generate(**inputs, max_new_tokens=1024, do_sample=False)
                tokens = result[:, inputs['input_ids'].shape[1]:]
                if tokens.shape[1] >= 1024:
                    raise RuntimeError('Output limit reached')
                parsed = processor.decode(tokens, return_format='parsed')[0]
                torch.cuda.synchronize()
                return {'text': parsed, 'tokens': tokens[0].tolist(),
                        'torch_peak_allocated_mib': torch.cuda.max_memory_allocated() / 2**20}

        report['device_memory_loaded_mib'] = sampler.read()
        for name, audio in cases:
            for iteration in range(args.repeat + 1):
                sample_start = len(sampler.samples)
                start = time.perf_counter()
                row = infer(name, audio)
                row.update(case=name, iteration=iteration, seconds=time.perf_counter() - start,
                           device_peak_mib=max([sampler.read()] + [v for _, v in sampler.samples[sample_start:]]))
                report['warmups' if iteration == 0 else 'runs'].append(row)
                save()
                print(f'{args.backend} {name} {iteration}: {row["seconds"]:.3f}s {row["text"]}', flush=True)
        report['summary'] = {name: {
            'median_seconds': statistics.median(r['seconds'] for r in report['runs'] if r['case'] == name),
            'min_seconds': min(r['seconds'] for r in report['runs'] if r['case'] == name),
            'max_seconds': max(r['seconds'] for r in report['runs'] if r['case'] == name),
            'max_device_mib': max(r['device_peak_mib'] for r in report['runs'] if r['case'] == name),
        } for name, _ in cases}
        save()
    except Exception as error:
        report['error'] = repr(error)
        save()
        raise
    finally:
        if server is not None and server.poll() is None:
            server.terminate()
            try:
                server.wait(timeout=15)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
        if log:
            log.close()
        sampler.close()
    print(f'Report: {output}', flush=True)


if __name__ == '__main__':
    main()

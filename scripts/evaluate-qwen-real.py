"""Run one explicit Qwen 1.7B configuration on the frozen ASCEND manifest."""
import argparse
import base64
from contextlib import nullcontext
import hashlib
import json
import os
from pathlib import Path
import socket
import statistics
import subprocess
import time
import urllib.error

from qwen_asr_metrics import aggregate, score
from qwen_eval_common import MemorySampler, request_json

ROOT = Path(__file__).resolve().parents[1]
MODELS = ROOT / '.runtime/models/Qwen3-ASR-1.7B-GGUF'


def sha256(path):
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model', required=True, type=Path)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--split', choices=['pilot', 'holdout'], default='pilot')
    parser.add_argument('--flash', choices=['on', 'off'], default='on')
    parser.add_argument('--no-cuda-graphs', action='store_true')
    parser.add_argument('--gpu-layers', type=int, default=99)
    parser.add_argument('--context', type=int, default=1024)
    parser.add_argument('--repetitions', type=int, default=3)
    parser.add_argument('--nvtx', action='store_true', help='Mark HTTP request ranges for an external Nsight capture.')
    parser.add_argument('--results-dir', type=Path, default=ROOT / 'experiments/qwen_asr/results')
    parser.add_argument('--nsys-session', help='Stop this named Nsight session before terminating the CUDA server.')
    args = parser.parse_args()
    if args.context < 1024 or args.repetitions < 1:
        parser.error('Use context >= 1024 and at least one repetition')
    if args.nvtx:
        import nvtx
    manifest_path = ROOT / 'experiments/qwen_asr/ascend-manifest.json'
    manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
    cases = [r for r in manifest['samples'] if r['split'] == args.split]
    if len(cases) != 30:
        raise RuntimeError('Expected the frozen 30-sample split')
    encoded = {}
    for row in cases:
        path = ROOT / row['audio']
        if sha256(path) != row['audio_sha256']:
            raise RuntimeError(f'Audio checksum mismatch: {row["id"]}')
        encoded[row['id']] = base64.b64encode(path.read_bytes()).decode('ascii')
    mmproj = MODELS / 'mmproj-Qwen3-ASR-1.7B-bf16.gguf'
    key = f'{args.tag}-{args.split}'
    results = args.results_dir
    results.mkdir(parents=True, exist_ok=True)
    logs = ROOT / 'outputs/qwen-real'
    logs.mkdir(parents=True, exist_ok=True)
    destination = results / f'{key}.json'
    if destination.exists():
        raise FileExistsError(f'Use a new --tag to preserve the existing result: {destination}')
    env = os.environ.copy()
    env['PATH'] = str(ROOT / '.venv-llama-build/Lib/site-packages/nvidia/cu13/bin/x86_64') + os.pathsep + env['PATH']
    for name in ('GGML_CUDA_DISABLE_GRAPHS', 'GGML_CUDA_GRAPH_OPT', 'GGML_CUDA_PDL'):
        env.pop(name, None)
    if args.no_cuda_graphs:
        env['GGML_CUDA_DISABLE_GRAPHS'] = '1'
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    endpoint = f'http://127.0.0.1:{port}'
    command = [str(ROOT / '.runtime/llama-build/bin/llama-server.exe'),
               '-m', str(args.model.resolve()), '--mmproj', str(mmproj),
               '--host', '127.0.0.1', '--port', str(port), '-ngl', str(args.gpu_layers),
               '--device', 'CUDA0', '-c', str(args.context), '-b', '256', '-ub', '256', '-np', '1', '-t', '4', '-tb', '4',
               '-fa', args.flash, '-ctk', 'f16', '-ctv', 'f16', '--fit', 'off',
               '--cache-ram', '0', '--no-cache-idle-slots', '--no-context-shift', '-lv', '4']
    report = {'tag': args.tag, 'split': args.split, 'command': command,
              'context': args.context, 'repetitions': args.repetitions, 'nvtx_markers': args.nvtx,
              'nsys_session': args.nsys_session,
              'cuda_graphs_disabled': args.no_cuda_graphs, 'model_sha256': sha256(args.model),
              'model_bytes': args.model.stat().st_size, 'mmproj_sha256': sha256(mmproj),
              'manifest_sha256': sha256(manifest_path),
              'source_commit': subprocess.check_output(['git', '-C', str(ROOT / 'external/llama.cpp'), 'rev-parse', 'HEAD'], text=True).strip(),
              'quality': [], 'performance': [], 'warmups': []}

    def save():
        destination.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')

    sampler = MemorySampler()
    report['device_before_load_mib'] = sampler.read()
    sampler.start()
    server = None
    log = (logs / f'{key}.log').open('w', encoding='utf-8')
    try:
        started = time.perf_counter()
        server = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                  creationflags=subprocess.CREATE_NO_WINDOW)
        while True:
            if server.poll() is not None:
                raise RuntimeError(f'Server exited {server.returncode}; see {log.name}')
            try:
                if request_json(endpoint + '/health').get('status') == 'ok':
                    break
            except (OSError, urllib.error.URLError):
                pass
            if time.perf_counter() - started > 180:
                raise TimeoutError('Server readiness timed out')
            time.sleep(0.5)
        report['load_seconds'] = time.perf_counter() - started
        report['device_loaded_mib'] = sampler.read()
        report['gpu_snapshot'] = subprocess.check_output([
            'nvidia-smi', '--query-gpu=name,driver_version,memory.used,clocks.sm,temperature.gpu',
            '--format=csv'], text=True, creationflags=subprocess.CREATE_NO_WINDOW).strip()

        def infer(case, phase='quality', iteration=0):
            payload = {'messages': [{'role': 'user', 'content': [
                {'type': 'input_audio', 'input_audio': {'data': encoded[case['id']], 'format': 'wav'}}]}],
                'temperature': 0, 'max_tokens': 512, 'cache_prompt': False, 'stream': False, 'seed': 42}
            sample_start = len(sampler.samples)
            begin = time.perf_counter()
            label = f'{phase}/{case["id"]}/{iteration}'
            with nvtx.annotate(label, domain='qwen-asr') if args.nvtx else nullcontext():
                response = request_json(endpoint + '/v1/chat/completions', payload)
            elapsed = time.perf_counter() - begin
            choice = response['choices'][0]
            raw = choice['message']['content'] or ''
            text = raw.split('<asr_text>', 1)[-1].strip()
            if response['timings']['cache_n'] != 0:
                raise RuntimeError('Unexpected prompt-cache reuse')
            return {'id': case['id'], 'language': case['language'], 'reference': case['reference'],
                    'text': text, 'raw': raw, 'seconds': elapsed, 'audio_seconds': case['seconds'],
                    'finish_reason': choice['finish_reason'], 'timings': response['timings'],
                    'completion_tokens': response['usage']['completion_tokens'],
                    'peak_device_mib': max([sampler.read()] + [v for _, v in sampler.samples[sample_start:]]),
                    'mer': score(case['reference'], text)}

        for index, case in enumerate(cases, 1):
            row = infer(case)
            report['quality'].append(row)
            save()
            print(f'{key}: quality {index}/30, {row["seconds"]:.3f}s, {row["mer"]}', flush=True)
        performance_cases = []
        for language in ('zh', 'en', 'mixed'):
            subset = sorted((r for r in cases if r['language'] == language), key=lambda r: (r['seconds'], r['id']))
            performance_cases.extend([subset[0], subset[-1]])
        report['performance_ids'] = [r['id'] for r in performance_cases]
        for case in performance_cases:
            report['warmups'].append(infer(case, 'warmup'))
            for iteration in range(args.repetitions):
                row = infer(case, 'performance', iteration + 1)
                row['iteration'] = iteration + 1
                report['performance'].append(row)
                save()
        report['summary'] = {
            'overall_mer': aggregate(report['quality'], 'mer'),
            'by_language': {language: aggregate([r for r in report['quality'] if r['language'] == language], metric)
                            for language, metric in [('zh', 'cer'), ('en', 'wer'), ('mixed', 'mer')]},
            'nonstop_outputs': sum(r['finish_reason'] != 'stop' for r in report['quality']),
            'performance': {case['id']: {
                'median_seconds': statistics.median(r['seconds'] for r in report['performance'] if r['id'] == case['id']),
                'min_seconds': min(r['seconds'] for r in report['performance'] if r['id'] == case['id']),
                'max_seconds': max(r['seconds'] for r in report['performance'] if r['id'] == case['id']),
                'decode_ms_per_token': statistics.median(r['timings']['predicted_per_token_ms'] for r in report['performance'] if r['id'] == case['id']),
                'stable_text': len({r['text'] for r in report['performance'] if r['id'] == case['id']}) == 1,
            } for case in performance_cases},
            'peak_device_mib': max(v for _, v in sampler.samples),
        }
        save()
        print(f'{key}: DONE MER={report["summary"]["overall_mer"]["rate"]:.3%}', flush=True)
        if args.nsys_session:
            # TerminateProcess skips CUPTI shutdown on Windows. Stop collection
            # while the CUDA process is alive so the profiler can drain buffers.
            nsys = ROOT / '.runtime/nsight-systems/target-windows-x64/nsys.exe'
            stopped = subprocess.run([str(nsys), 'stop', '--session', args.nsys_session],
                                     capture_output=True, text=True, timeout=120,
                                     creationflags=subprocess.CREATE_NO_WINDOW)
            (logs / f'{key}-nsys-stop.log').write_text(stopped.stdout + stopped.stderr, encoding='utf-8')
            stopped.check_returncode()
            report['nsys_stopped_before_model_exit'] = True
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
        log.close()
        sampler.close()


if __name__ == '__main__':
    main()

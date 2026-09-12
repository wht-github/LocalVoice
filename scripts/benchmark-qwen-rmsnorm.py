"""Compare original Qwen ASR vs ONLY the student's 1024-wide RMSNorm."""
import argparse
from contextlib import contextmanager
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import statistics
import sys
import time
import types

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault('HF_HOME', str(ROOT/'.runtime/qwen-hf'))
os.environ.setdefault('HF_HUB_OFFLINE', '1')
os.environ.setdefault('NUMBA_CACHE_DIR', str(ROOT/'.runtime/numba-cache'))
os.environ.setdefault('TILELANG_CACHE_DIR', str(ROOT/'.runtime/tilelang-cache'))
os.environ.setdefault('TVM_FFI_CACHE_DIR', str(ROOT/'.runtime/tvm-ffi-cache'))
os.environ.setdefault('TVM_FFI_DISABLE_TORCH_C_DLPACK', '1')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repeat', type=int, default=3)
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error('--repeat must be positive')
    import librosa
    import numpy as np
    import torch
    import transformers
    from transformers import AutoModelForMultimodalLM, AutoProcessor
    from transformers.models.qwen3.modeling_qwen3 import Qwen3RMSNorm
    # Import Transformers first so the older exercise kernels.py cannot shadow
    # the optional Hugging Face "kernels" package during dependency discovery.
    sys.path.insert(0, str(ROOT/'experiments/tilelang'))
    from rmsnorm_exercise import rmsnorm_candidate
    from rmsnorm_run import check
    torch.set_num_threads(4)
    with torch.inference_mode():
        check(rmsnorm_candidate)
    output = ROOT/'outputs/qwen-rmsnorm'/datetime.now().strftime('%Y%m%d-%H%M%S')
    output.mkdir(parents=True, exist_ok=True)
    processor = AutoProcessor.from_pretrained('Qwen/Qwen3-ASR-0.6B-hf', local_files_only=True)
    model = AutoModelForMultimodalLM.from_pretrained('Qwen/Qwen3-ASR-0.6B-hf',
        dtype=torch.float16, attn_implementation='sdpa', local_files_only=True).to('cuda').eval()
    selected = [(name, module, module.forward) for name, module in model.named_modules()
                if isinstance(module, Qwen3RMSNorm) and module.weight.shape == (1024,)]
    assert len(selected) == 57, f'Unexpected model architecture: {len(selected)} RMSNorms'
    assert all(m.variance_epsilon == 1e-6 for _,m,_ in selected)

    def forward(module, hidden_states):
        return rmsnorm_candidate(hidden_states, module.weight)

    @contextmanager
    def implementation(backend):
        try:
            if backend == 'tilelang':
                for _, module, _ in selected:
                    module.forward = types.MethodType(forward, module)
            yield
        finally:
            for _, module, original in selected:
                module.forward = original

    zh, _ = librosa.load(str(ROOT/'outputs/melo-mandarin.wav'), sr=16000, mono=True)
    mixed, _ = librosa.load(str(ROOT/'outputs/melo-mixed.wav'), sr=16000, mono=True)
    cases = [('short', zh[:48000]), ('mandarin', zh), ('mixed', mixed),
             ('long', np.concatenate([zh, np.zeros(8000, np.float32), mixed]))]
    report = {'torch': torch.__version__, 'transformers': transformers.__version__,
              'gpu': torch.cuda.get_device_name(), 'changed_modules': [n for n,_,_ in selected],
              'exercise_sha256': hashlib.sha256((ROOT/'experiments/tilelang/rmsnorm_exercise.py').read_bytes()).hexdigest(),
              'runs': [], 'warmups': [],
              'scope': 'Only 57 width-1024 RMSNorm forward methods replaced. Same model weights, FP16, SDPA, HF greedy generate, automatic language, max_new_tokens=1024. No CUDA Graph. Includes preprocessing, transfer, generation, text decode; excludes file load, model load, warmup/JIT, microphone, HTTP, desktop insertion.'}

    @torch.inference_mode()
    def run(name, audio, backend):
        with implementation(backend):
            torch.cuda.synchronize()
            start = time.perf_counter()
            inputs = processor.apply_transcription_request(audio=audio, language=None).to(model.device, model.dtype)
            result = model.generate(**inputs, max_new_tokens=1024, do_sample=False)
            tokens = result[:, inputs['input_ids'].shape[1]:]
            parsed = processor.decode(tokens, return_format='parsed')[0]
            torch.cuda.synchronize()
            elapsed = time.perf_counter()-start
            ids = tokens[0].tolist()
            if len(ids) >= 1024:
                raise RuntimeError('Output limit reached')
            return {'case': name, 'backend': backend, 'audio_seconds': len(audio)/16000,
                    'seconds': elapsed, 'tokens': ids, 'result': parsed}

    def save():
        (output/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')

    for name, audio in cases:
        ref = run(name, audio, 'original')
        candidate = run(name, audio, 'tilelang')
        report['warmups'] += [ref, candidate]
        for repeat in range(args.repeat):
            for backend in (('original', 'tilelang') if repeat % 2 == 0 else ('tilelang', 'original')):
                row = run(name, audio, backend)
                row['repeat'] = repeat+1
                row['exact_tokens_match'] = row['tokens'] == ref['tokens']
                row['parsed_result_match'] = row['result'] == ref['result']
                report['runs'].append(row)
                save()
                print(f'{name} {backend} {row["seconds"]:.3f}s tokens={len(row["tokens"])} exact={row["exact_tokens_match"]}', flush=True)
    lines = ['# Qwen ASR: original vs RMSNorm-only replacement', '',
             '| Audio | Original median (s) [range] | TileLang median (s) [range] | Speed ratio | Exact tokens in all measured runs |',
             '|---|---:|---:|---:|---|']
    for name,_ in cases:
        vals = {b: [r['seconds'] for r in report['runs'] if r['case']==name and r['backend']==b] for b in ('original','tilelang')}
        cells = [f'{statistics.median(v):.3f} [{min(v):.3f}-{max(v):.3f}]' for v in vals.values()]
        ratio = statistics.median(vals['original'])/statistics.median(vals['tilelang'])
        exact = all(r['exact_tokens_match'] for r in report['runs'] if r['case']==name)
        lines.append(f'| {name} | {cells[0]} | {cells[1]} | {ratio:.2f}x | {exact} |')
    lines += ['', report['scope'], '', 'Synthetic audio fixtures. Exact output agreement on these inputs does not establish general ASR accuracy. GPU clocks/background load were not controlled. No desktop service modifications.']
    (output/'summary.md').write_text('\n'.join(lines)+'\n', encoding='utf-8')
    print('DONE:', output, flush=True)


if __name__ == '__main__':
    main()

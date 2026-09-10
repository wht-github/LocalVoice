"""Compare real audio transcription: HF generate vs captured greedy decode."""
import argparse
from datetime import datetime
import json
import os
from pathlib import Path
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault('HF_HOME', str(ROOT/'.runtime/qwen-hf'))
os.environ.setdefault('HF_HUB_OFFLINE', '1')
os.environ.setdefault('NUMBA_CACHE_DIR', str(ROOT/'.runtime/numba-cache'))
sys.path.insert(0, str(ROOT/'experiments/qwen_decode'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--short-only', action='store_true')
    parser.add_argument('--repeat', type=int, default=3)
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error('--repeat must be positive')
    import librosa
    import numpy as np
    import torch
    import transformers
    from transformers import AutoModelForMultimodalLM, AutoProcessor
    from cuda_graph import GreedyDecodeGraph
    torch.set_num_threads(4)
    output = ROOT/'outputs/qwen-decode'/datetime.now().strftime('%Y%m%d-%H%M%S')
    output.mkdir(parents=True, exist_ok=True)
    processor = AutoProcessor.from_pretrained('Qwen/Qwen3-ASR-0.6B-hf', local_files_only=True)
    model = AutoModelForMultimodalLM.from_pretrained('Qwen/Qwen3-ASR-0.6B-hf',
        dtype=torch.float16, attn_implementation='sdpa', local_files_only=True).to('cuda').eval()
    torch.cuda.synchronize()
    start = time.perf_counter()
    graph = GreedyDecodeGraph(model)
    setup = time.perf_counter()-start
    print(f'Graph captured in {setup:.3f} seconds', flush=True)
    zh, _ = librosa.load(str(ROOT/'outputs/melo-mandarin.wav'), sr=16000, mono=True)
    mixed, _ = librosa.load(str(ROOT/'outputs/melo-mixed.wav'), sr=16000, mono=True)
    cases = [('short', zh[:48000])]
    if not args.short_only:
        cases += [('mandarin', zh), ('mixed', mixed), ('long', np.concatenate([zh, np.zeros(8000, np.float32), mixed]))]
    report = {'torch': torch.__version__, 'transformers': transformers.__version__,
              'gpu': torch.cuda.get_device_name(), 'graph_setup_seconds': setup,
              'capacity': graph.capacity, 'max_new_tokens': 1024, 'runs': [],
              'scope': 'Preprocessing, transfer, prefill, decode and text parsing; excludes model load, graph setup, file read, microphone/HTTP/desktop. FP16 SDPA greedy, auto language. Exact token comparison.'}

    @torch.inference_mode()
    def run(name, audio, backend):
        torch.cuda.synchronize()
        start = time.perf_counter()
        inputs = processor.apply_transcription_request(audio=audio, language=None).to(model.device, model.dtype)
        if backend == 'hf':
            result = model.generate(**inputs, max_new_tokens=1024, do_sample=False)
            tokens = result[:, inputs['input_ids'].shape[1]:]
        else:
            tokens = graph.generate(inputs, max_new_tokens=1024)
        parsed = processor.decode(tokens, return_format='parsed')[0]
        torch.cuda.synchronize()
        elapsed = time.perf_counter()-start
        token_list = tokens[0].tolist()
        if len(token_list) >= 1024:
            raise RuntimeError('Output limit reached')
        return {'case': name, 'backend': backend, 'audio_seconds': len(audio)/16000,
                'seconds': elapsed, 'tokens': token_list, 'result': parsed,
                'allocated_mib': torch.cuda.memory_allocated()/2**20,
                'reserved_mib': torch.cuda.memory_reserved()/2**20}

    for name, audio in cases:
        ref = run(name, audio, 'hf')
        candidate = run(name, audio, 'graph')
        if candidate['tokens'] != ref['tokens']:
            report['mismatch'] = {'reference': ref, 'candidate': candidate}
            (output/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
            raise RuntimeError(f'Token mismatch for {name}; report saved in {output}')
        for repeat in range(args.repeat):
            for backend in (('hf', 'graph') if repeat % 2 == 0 else ('graph', 'hf')):
                row = run(name, audio, backend)
                row['repeat'] = repeat+1
                row['exact_tokens_match'] = row['tokens'] == ref['tokens']
                report['runs'].append(row)
                (output/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
                if not row['exact_tokens_match']:
                    raise RuntimeError(f'Token mismatch for {name} {backend}; report saved')
                print(f'{name} {backend} {row["seconds"]:.3f}s tokens={len(row["tokens"])} EXACT', flush=True)
    print('DONE:', output, flush=True)


if __name__ == '__main__':
    main()

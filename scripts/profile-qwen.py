"""Measure native Qwen ASR latency and profile real model operators offline."""
import argparse
from collections import Counter
from datetime import datetime
import json
import os
from pathlib import Path
import time

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault('HF_HOME', str(ROOT / '.runtime/qwen-hf'))
os.environ.setdefault('HF_HUB_OFFLINE', '1')
os.environ.setdefault('NUMBA_CACHE_DIR', str(ROOT / '.runtime/numba-cache'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    out = args.output or ROOT / 'outputs/qwen-profile' / datetime.now().strftime('%Y%m%d-%H%M%S')
    out.mkdir(parents=True, exist_ok=True)
    import numpy as np
    import librosa
    import torch
    import transformers
    from transformers import AutoModelForMultimodalLM, AutoProcessor
    torch.set_num_threads(4)
    start = time.perf_counter()
    processor = AutoProcessor.from_pretrained('Qwen/Qwen3-ASR-0.6B-hf', local_files_only=True)
    model = AutoModelForMultimodalLM.from_pretrained('Qwen/Qwen3-ASR-0.6B-hf',
        dtype=torch.float16, attn_implementation='sdpa', local_files_only=True).to('cuda').eval()
    torch.cuda.synchronize()
    report = {'model_load_seconds': time.perf_counter()-start, 'torch': torch.__version__,
              'transformers': transformers.__version__, 'gpu': torch.cuda.get_device_name(),
              'dtype': str(model.dtype), 'max_new_tokens': 1024, 'language': 'auto',
              'scope': 'In-process ASR; file loading, HTTP, microphone and desktop insertion excluded. Stage timing synchronizes GPU. Synthetic speech fixtures, not a recognition accuracy benchmark.', 'samples': []}
    start = time.perf_counter()
    zh, _ = librosa.load(str(ROOT/'outputs/melo-mandarin.wav'), sr=16000, mono=True)
    mixed, _ = librosa.load(str(ROOT/'outputs/melo-mixed.wav'), sr=16000, mono=True)
    report['fixture_loading_seconds'] = time.perf_counter()-start
    samples = [('short_crop', zh[:48000]), ('mandarin', zh), ('mixed', mixed),
               ('long_concat', np.concatenate([zh, np.zeros(8000, dtype=np.float32), mixed]))]

    def transcribe(audio):
        torch.cuda.synchronize()
        start = time.perf_counter()
        inputs = processor.apply_transcription_request(audio=audio, language=None)
        processed = time.perf_counter()
        inputs = inputs.to(model.device, model.dtype)
        torch.cuda.synchronize()
        transferred = time.perf_counter()
        with torch.inference_mode():
            generated = model.generate(**inputs, max_new_tokens=1024, do_sample=False)
        torch.cuda.synchronize()
        inferred = time.perf_counter()
        tokens = generated[:, inputs['input_ids'].shape[1]:]
        parsed = processor.decode(tokens, return_format='parsed')[0]
        ended = time.perf_counter()
        if tokens.shape[1] >= 1024:
            raise RuntimeError('Token limit reached; timing is not a completed transcription')
        return {'preprocess_ms': (processed-start)*1000, 'transfer_ms': (transferred-processed)*1000,
                'generate_ms': (inferred-transferred)*1000, 'text_decode_ms': (ended-inferred)*1000,
                'total_ms': (ended-start)*1000, 'tokens': tokens.shape[1], 'result': parsed}

    report['cold_request'] = transcribe(zh)
    print('Cold request:', report['cold_request']['total_ms'], flush=True)
    for name, audio in samples:
        transcribe(audio)
        runs = [transcribe(audio) for _ in range(3)]
        report['samples'].append({'name': name, 'audio_seconds': len(audio)/16000, 'runs': runs})
        (out/'baseline.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
        print(name, [round(r['total_ms'], 1) for r in runs], 'tokens', [r['tokens'] for r in runs], flush=True)

    # Instrument only a separate pass; ordinary baseline above has no hooks/profiler.
    shapes = Counter()
    handles = []
    def instrument(module, label, classify=False):
        stack = []
        def before(mod, positional, kwargs):
            tensors = [x for x in positional if isinstance(x, torch.Tensor)]
            tensors += [x for x in kwargs.values() if isinstance(x, torch.Tensor)]
            x = tensors[0] if tensors else None
            tag = label
            if classify:
                x = kwargs.get('inputs_embeds', kwargs.get('input_ids', x))
                tag = 'text_decode_step' if x is not None and x.shape[1] == 1 else 'text_prefill'
            if label.startswith('rmsnorm') and x is not None:
                shapes[(label, str(tuple(x.shape)), str(x.dtype), str(getattr(mod, 'variance_epsilon', None)))] += 1
            ctx = torch.profiler.record_function(tag)
            ctx.__enter__()
            stack.append(ctx)
        def after(mod, positional, kwargs, result):
            stack.pop().__exit__(None, None, None)
        handles.append(module.register_forward_pre_hook(before, with_kwargs=True))
        handles.append(module.register_forward_hook(after, with_kwargs=True))
    instrument(model.model.audio_tower, 'audio_encoder')
    instrument(model.model.language_model, 'text_model', classify=True)
    for name, module in model.named_modules():
        if module.__class__.__name__ == 'Qwen3RMSNorm':
            instrument(module, 'rmsnorm.' + name.split('.')[-1])
        elif module.__class__.__name__ == 'Qwen3MLP':
            instrument(module, 'text_mlp')
    print('Profiling full Mandarin transcription...', flush=True)
    try:
        with torch.profiler.profile(activities=[torch.profiler.ProfilerActivity.CPU, torch.profiler.ProfilerActivity.CUDA],
                                    record_shapes=True) as prof:
            profiled = transcribe(zh)
    finally:
        for handle in handles:
            handle.remove()
    (out/'profile.json').write_text(json.dumps({'transcription': profiled,
        'rmsnorm_shapes': [{'label': k[0], 'shape': k[1], 'dtype': k[2], 'epsilon': k[3], 'count': v} for k,v in shapes.items()]}, ensure_ascii=False, indent=2), encoding='utf-8')
    print('Exporting trace...', flush=True)
    prof.export_chrome_trace(str(out/'mandarin.trace.json'))
    # Avoid prof.events()/key_averages(): expanding this trace's Python event tree
    # used >11 GB on Windows. Analyze the exported JSON in a separate process.
    print('DONE:', out, flush=True)
    print('Analyze with scripts/analyze-qwen-trace.py <output>/mandarin.trace.json', flush=True)


if __name__ == '__main__':
    main()

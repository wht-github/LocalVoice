"""Transcribe a local audio file on Windows/NVIDIA and record repeat timings."""
import argparse
import json
import os
from pathlib import Path
import time

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault('HF_HOME', str(ROOT / '.runtime' / 'qwen-hf'))
os.environ.setdefault('HF_ENDPOINT', 'https://hf-mirror.com')
os.environ.setdefault('HF_HUB_DISABLE_TELEMETRY', '1')
os.environ.setdefault('HF_HUB_DISABLE_XET', '1')
os.environ.setdefault('NUMBA_CACHE_DIR', str(ROOT / '.runtime' / 'numba-cache'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('audio', type=Path)
    parser.add_argument('--model', default='Qwen/Qwen3-ASR-0.6B-hf')
    parser.add_argument('--language', default=None)
    parser.add_argument('--prompt', default=None, help='Vocabulary/context hint')
    parser.add_argument('--repeat', type=int, default=2)
    parser.add_argument('--max-new-tokens', type=int, default=512)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if not args.audio.is_file() or args.repeat < 1:
        parser.error('Provide an existing audio file and --repeat >= 1')

    import librosa
    import torch
    import transformers
    from transformers import AutoModelForMultimodalLM, AutoProcessor

    if not torch.cuda.is_available():
        raise RuntimeError('CUDA is unavailable in this Python environment')
    torch.set_num_threads(4)
    started = time.perf_counter()
    processor = AutoProcessor.from_pretrained(args.model)
    model = AutoModelForMultimodalLM.from_pretrained(
        args.model, dtype=torch.float16, attn_implementation='sdpa',
    ).to('cuda').eval()
    torch.cuda.synchronize()
    load_seconds = time.perf_counter() - started
    samples, _ = librosa.load(str(args.audio), sr=16000, mono=True)
    duration = len(samples) / 16000
    results = []
    for i in range(args.repeat):
        torch.cuda.reset_peak_memory_stats()
        torch.cuda.synchronize()
        started = time.perf_counter()
        inputs = processor.apply_transcription_request(
            audio=samples, language=args.language, prompt=args.prompt,
        ).to(model.device, model.dtype)
        with torch.inference_mode():
            generated = model.generate(**inputs, max_new_tokens=args.max_new_tokens, do_sample=False)
        token_ids = generated[:, inputs['input_ids'].shape[1]:]
        parsed = processor.decode(token_ids, return_format='parsed')[0]
        torch.cuda.synchronize()
        elapsed = time.perf_counter() - started
        row = {'run': i + 1, 'first_request': i == 0, 'seconds': round(elapsed, 3),
               'rtf': round(elapsed / duration, 4) if duration else None,
               'peak_allocated_mib': round(torch.cuda.max_memory_allocated() / 2**20),
               'peak_reserved_mib': round(torch.cuda.max_memory_reserved() / 2**20),
               'generated_tokens': token_ids.shape[1],
               'token_limit_reached': token_ids.shape[1] >= args.max_new_tokens,
               'result': parsed}
        results.append(row)
        print(json.dumps(row, ensure_ascii=False), flush=True)
    report = {'model': args.model, 'torch': torch.__version__, 'transformers': transformers.__version__,
              'gpu': torch.cuda.get_device_name(0), 'dtype': str(model.dtype),
              'audio': str(args.audio.resolve()), 'audio_seconds': duration,
              'load_seconds': round(load_seconds, 3),
              'timing_scope': 'preprocessing + GPU transfer + generation + text decoding; audio file loading excluded',
              'runs': results}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()

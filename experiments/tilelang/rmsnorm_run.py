"""Original Qwen RMSNorm vs ONLY the student's RMSNorm, on real model shapes."""
import bootstrap
import argparse
from datetime import datetime
import json
from pathlib import Path
import statistics
import time

import torch
from rmsnorm_reference import rmsnorm_reference


def fixtures():
    generator = torch.Generator(device='cuda').manual_seed(42)
    for rows in (1, 151):
        shape = (1, rows, 1024)
        w = torch.linspace(-0.75, 1.25, 1024, device='cuda', dtype=torch.float16)
        random = torch.randn(shape, device='cuda', generator=generator)
        for name, values in [('zero', torch.zeros_like(random)),
                             ('constant', torch.full_like(random, 2)),
                             ('random', random), ('tiny', random * 1e-5),
                             ('large', random * 1000),
                             ('row_scales', random * torch.linspace(0.01, 10, rows, device='cuda').view(1, rows, 1))]:
            yield f'{name}_{rows}', values.half(), w
        yield f'zero_weight_{rows}', random.half(), torch.zeros_like(w)


def check(candidate=None):
    rows = []
    for name, x, w in fixtures():
        expected = rmsnorm_reference(x, w)
        # Independent high precision check of the baseline's formula/rounding order.
        xd = x.double()
        oracle = (xd / torch.sqrt((xd * xd).mean(-1, keepdim=True) + 1e-6)).half() * w
        torch.testing.assert_close(expected, oracle, rtol=2e-3, atol=2e-3)
        if candidate:
            actual = candidate(x, w)
            if actual.shape != x.shape or actual.dtype != x.dtype or actual.device != x.device:
                raise AssertionError(f'{name}: output shape, dtype or device is wrong')
            if not torch.isfinite(actual).all():
                raise AssertionError(f'{name}: non-finite output')
            try:
                torch.testing.assert_close(actual, expected, rtol=2e-3, atol=2e-3)
            except AssertionError as exc:
                raise AssertionError(f'{name}: RMSNorm exercise not correct yet. Complete the TODOs in rmsnorm_exercise.py.\n{exc}') from exc
            error = (actual.float()-expected.float()).abs()
            rows.append({'case': name, 'max_abs_error': error.max().item(), 'mean_abs_error': error.mean().item()})
        else:
            rows.append({'case': name, 'reference_checked': True})
    return rows


def measure(fn):
    for _ in range(20):
        fn()
    samples = []
    for _ in range(7):
        torch.cuda.synchronize()
        start = time.perf_counter()
        for _ in range(100):
            fn()
        torch.cuda.synchronize()
        samples.append((time.perf_counter()-start)*1e6/100)
    return {'median_us': statistics.median(samples), 'min_us': min(samples),
            'max_us': max(samples), 'rounds_us': samples}


@torch.inference_mode()
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', action='store_true', help='Run original Qwen RMSNorm only; no student code required')
    parser.add_argument('--check-only', action='store_true')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if not torch.cuda.is_available():
        raise SystemExit('CUDA is required; run rmsnorm-lab.ps1')
    candidate = None
    if not args.baseline:
        from rmsnorm_exercise import rmsnorm_candidate
        candidate = rmsnorm_candidate
    try:
        checks = check(candidate)
    except AssertionError as exc:
        raise SystemExit(str(exc)) from exc
    print(f'PASS: {len(checks)} cases; ' + ('original reference' if args.baseline else 'student vs original'), flush=True)
    report = {'torch': torch.__version__, 'gpu': torch.cuda.get_device_name(), 'eps': 1e-6,
              'dtype': 'float16', 'checks': checks, 'benchmarks': [],
              'scope': 'Synchronized wall time including output allocation, Python dispatch and GPU work. 20 warmups, 7 x 100 calls. Compilation excluded. Original eager Qwen RMSNorm vs only RMSNorm replacement; no graphs or model decode optimizations.'}
    if not args.check_only:
        torch.manual_seed(123)
        for n in (1, 151):
            x = torch.randn((1, n, 1024), device='cuda', dtype=torch.float16)
            w = torch.linspace(-0.75, 1.25, 1024, device='cuda', dtype=torch.float16)
            entry = {'shape': list(x.shape), 'pytorch': measure(lambda: rmsnorm_reference(x, w))}
            if candidate:
                entry['tilelang'] = measure(lambda: candidate(x, w))
                entry['speed_ratio'] = entry['pytorch']['median_us']/entry['tilelang']['median_us']
            report['benchmarks'].append(entry)
            print(json.dumps(entry), flush=True)
    output = args.output or bootstrap.ROOT/'outputs/rmsnorm'/(
        datetime.now().strftime('%Y%m%d-%H%M%S') + ('-baseline.json' if args.baseline else '-comparison.json'))
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(f'Report: {output.resolve()}')


if __name__ == '__main__':
    main()

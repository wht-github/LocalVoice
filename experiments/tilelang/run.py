"""Check results first, then benchmark eager PyTorch and a TileLang kernel."""
import bootstrap
import argparse
import json
import statistics
import time
from pathlib import Path

import torch
import tilelang
from kernels import vector_add, add_relu_exercise


def measure(fn, repeats=100, rounds=7):
    for _ in range(20):
        fn()
    torch.cuda.synchronize()
    # Synchronized wall timing includes Python dispatch and GPU work. Do not use
    # graph capture here: this Windows NVRTC path can produce an empty graph.
    samples = []
    for _ in range(rounds):
        torch.cuda.synchronize()
        begin = time.perf_counter()
        for _ in range(repeats):
            fn()
        torch.cuda.synchronize()
        samples.append((time.perf_counter() - begin) * 1e6 / repeats)
    return {
        "wall_median_us": statistics.median(samples),
        "wall_min_us": min(samples),
        "wall_max_us": max(samples),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exercise", action="store_true", help="Check your add+ReLU implementation")
    parser.add_argument("--check-only", action="store_true")
    parser.add_argument("--block", type=int, choices=(128, 256, 512, 1024), default=256)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if not torch.cuda.is_available():
        raise SystemExit("CUDA unavailable; use .venv-tilelang and an NVIDIA GPU.")
    torch.manual_seed(42)
    kernel = add_relu_exercise if args.exercise else vector_add
    sizes = [1, 31, 127, 128, 129, 255, 256, 257, 511, 512, 513, 1025, 65536, 1048576]
    for n in sizes:
        a = torch.randn(n, device="cuda", dtype=torch.float32)
        b = torch.randn_like(a)
        # Deterministic negative, zero and positive sums, in addition to random data.
        a[:min(n, 3)] = torch.tensor([-2., 0., 2.], device="cuda")[:min(n, 3)]
        b[:min(n, 3)] = 0
        out = torch.full_like(a, float("nan"))
        kernel(a, b, out, block=args.block)
        expected = torch.relu(a + b) if args.exercise else a + b
        try:
            torch.testing.assert_close(out, expected, rtol=1e-6, atol=1e-6)
        except AssertionError as exc:
            if args.exercise:
                raise SystemExit("Exercise not correct yet: edit the TODO in kernels.py.\n" + str(exc)) from exc
            raise
    print(f"PASS: {len(sizes)} sizes, float32, block={args.block}; {'add_relu' if args.exercise else 'add'}")
    if args.check_only:
        return
    report = {
        "gpu": torch.cuda.get_device_name(), "torch": torch.__version__,
        "tilelang": tilelang.__version__, "torch_cuda": torch.version.cuda,
        "operation": "add_relu" if args.exercise else "add", "block": args.block,
        "correctness_sizes": sizes, "benchmarks": [],
        "notes": "Preallocated float32 CUDA tensors. Compilation excluded. Synchronized wall time includes Python dispatch and GPU work; not pure kernel duration. Not an ASR speedup measurement.",
    }
    for n in (1024, 65536, 1048576):
        a = torch.randn(n, device="cuda")
        b = torch.randn_like(a)
        out = torch.empty_like(a)

        def reference():
            torch.add(a, b, out=out)
            if args.exercise:
                torch.relu_(out)

        def candidate():
            kernel(a, b, out, block=args.block)

        baseline = measure(reference)
        optimized = measure(candidate)
        ratio = baseline["wall_median_us"] / optimized["wall_median_us"]
        row = {"elements": n, "pytorch": baseline, "tilelang": optimized, "wall_speed_ratio": ratio}
        report["benchmarks"].append(row)
        print(f"N={n:>8}: wall PyTorch={baseline['wall_median_us']:.3f} us, TileLang={optimized['wall_median_us']:.3f} us, ratio={ratio:.2f}x")
    output = args.output or bootstrap.ROOT / "outputs/tilelang" / ("exercise.json" if args.exercise else "baseline.json")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"Report: {output.resolve()}")


if __name__ == "__main__":
    main()

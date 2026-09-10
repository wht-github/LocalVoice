"""Profile eager add+ReLU and the student TileLang kernel separately."""
import bootstrap
import argparse
import json
import statistics
from datetime import datetime
from pathlib import Path

import torch
from kernels import add_relu_exercise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    output = args.output or bootstrap.ROOT / "outputs/tilelang" / datetime.now().strftime("profile-%Y%m%d-%H%M%S")
    output.mkdir(parents=True, exist_ok=True)
    torch.manual_seed(42)
    a = torch.randn(1048576, device="cuda", dtype=torch.float32)
    b = torch.randn_like(a)
    out = torch.empty_like(a)

    def reference():
        torch.add(a, b, out=out)
        torch.relu_(out)

    def candidate():
        add_relu_exercise(a, b, out, block=256)

    expected = torch.relu(a + b)
    for fn in (reference, candidate):
        for _ in range(20):
            fn()
        torch.cuda.synchronize()
        torch.testing.assert_close(out, expected)
    results = {"gpu": torch.cuda.get_device_name(), "torch": torch.__version__,
               "elements": a.numel(), "block": 256, "calls_per_capture": 5,
               "notes": "Profiler instrumentation affects timing. CPU ranges measure submission, not GPU completion. Missing kernel events are unavailable data, never zero GPU time.",
               "captures": []}
    # Reverse the order on the second pass to expose first-capture effects.
    for repeat, names in enumerate((('pytorch', 'tilelang'), ('tilelang', 'pytorch')), 1):
        for name in names:
            fn = reference if name == "pytorch" else candidate
            torch.cuda.synchronize()
            with torch.profiler.profile(activities=[torch.profiler.ProfilerActivity.CPU,
                                                   torch.profiler.ProfilerActivity.CUDA]) as prof:
                for _ in range(5):
                    with torch.profiler.record_function(name + "_add_relu"):
                        fn()
                torch.cuda.synchronize()
            stem = f"{name}-{repeat}"
            trace_path = output / (stem + ".trace.json")
            prof.export_chrome_trace(str(trace_path))
            (output / (stem + ".table.txt")).write_text(
                prof.key_averages().table(sort_by="self_cpu_time_total", row_limit=30), encoding="utf-8")
            trace = json.loads(trace_path.read_text(encoding="utf-8"))
            events = trace.get("traceEvents", [])
            kernels = [e for e in events if e.get("cat") == "kernel" and e.get("ph") == "X"]
            # GPU annotations can share the same label: never count them as CPU time.
            ranges = [e for e in events if e.get("cat") == "user_annotation"
                      and e.get("name") == name + "_add_relu" and e.get("ph") == "X"]
            kernels.sort(key=lambda e: e["ts"])
            ranges.sort(key=lambda e: e["ts"])
            per_call = 2 if name == "pytorch" else 1
            groups = [kernels[i:i + per_call] for i in range(0, len(kernels), per_call)] if len(kernels) == 5 * per_call else []
            gpu_times = [sum(e["dur"] for e in group) for group in groups]
            gaps = [groups[i + 1][0]["ts"] - (group[-1]["ts"] + group[-1]["dur"])
                    for i, group in enumerate(groups[:-1])]
            record = {"backend": name, "repeat": repeat, "trace": str(trace_path.resolve()),
                      "kernel_count": len(kernels),
                      "kernel_total_us": sum(e["dur"] for e in kernels) if kernels else None,
                      "cpu_submission_us": [e["dur"] for e in ranges],
                      "gpu_kernel_sum_per_call_us": gpu_times,
                      "gpu_intercall_gaps_us": gaps,
                      "kernels": [{k: e.get(k) for k in ("name", "ts", "dur", "args")} for e in kernels]}
            results["captures"].append(record)
            print(f"{stem}: kernels={len(kernels)}, GPU total={record['kernel_total_us']} us, CPU ranges={record['cpu_submission_us']}", flush=True)
    (output / "summary.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
    lines = ["# Add + ReLU CPU / GPU profile", "",
             "RTX 4050; 1,048,576 float32 elements; block=256; 128 threads. Correctness verified before capture. Each capture contains 5 calls, after 20 warmup calls per backend. Two passes reverse backend order.", "",
             "Times below are microseconds. CPU ranges are submission scope durations under profiler instrumentation, not synchronized latency. GPU sums exclude gaps. The first call of every capture is retained in JSON but excluded from the medians below because capture startup heavily distorts it.", "",
             "| Capture | GPU kernels / 5 calls | GPU kernel sum per call (median, calls 2-5) | CPU submission scope (median, calls 2-5) | Between-call GPU gap (median) |",
             "|---|---:|---:|---:|---:|"]
    for item in results["captures"]:
        def med(values):
            return f"{statistics.median(values):.2f}" if values else "unavailable"
        lines.append(f"| {item['backend']}-{item['repeat']} | {item['kernel_count']} | {med(item['gpu_kernel_sum_per_call_us'][1:])} | {med(item['cpu_submission_us'][1:])} | {med(item['gpu_intercall_gaps_us'])} |")
    lines += ["", "Interpretation: kernel counts verify whether fusion occurred. Kernel times characterize device work in these captures; CPU ranges and gaps are affected by profiling overhead. Gaps can include submission, scheduling or other delays and cannot be attributed solely to Python. Use the unprofiled benchmark for application-facing latency. Missing CUDA events mean unavailable data, not zero time.", "",
              "The .trace.json files use Chrome Trace format. Inspect CPU user_annotation and GPU kernel tracks; gpu_user_annotation is a GPU annotation, not a second CPU call. The .table.txt files contain operator aggregates; direct TileLang driver launches may not appear as ordinary aten operators."]
    (output / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Report directory: {output.resolve()}")


if __name__ == "__main__":
    main()

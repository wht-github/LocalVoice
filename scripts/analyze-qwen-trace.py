"""Bounded-memory Chrome Trace analysis; no PyTorch event-tree expansion."""
import argparse
from bisect import bisect_right
from collections import defaultdict
import json
from pathlib import Path


def trace_events(path):
    decoder = json.JSONDecoder()
    with path.open(encoding='utf-8') as source:
        buffer = ''
        while '"traceEvents"' not in buffer:
            chunk = source.read(65536)
            if not chunk:
                raise ValueError('Missing traceEvents')
            buffer += chunk
        buffer = buffer.split('"traceEvents"', 1)[1]
        buffer = buffer[buffer.index('[')+1:]
        while True:
            buffer = buffer.lstrip(' \r\n\t,')
            if buffer.startswith(']'):
                return
            try:
                event, end = decoder.raw_decode(buffer)
            except json.JSONDecodeError:
                chunk = source.read(65536)
                if not chunk:
                    raise ValueError('Incomplete trace')
                buffer += chunk
                continue
            yield event
            buffer = buffer[end:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trace', type=Path)
    args = parser.parse_args()
    ops, kernels, scopes = {}, [], defaultdict(list)
    scope_cpu = defaultdict(lambda: {'calls': 0, 'cpu_scope_us': 0})
    for e in trace_events(args.trace):
        if e.get('ph') != 'X':
            continue
        cat = e.get('cat')
        a = e.get('args', {})
        if cat == 'cpu_op' and 'External id' in a:
            ops[a['External id']] = (e['ts'], e['name'], a.get('Input Dims', []), a.get('Input type', []))
        elif cat == 'kernel':
            kernels.append((e['name'], e['dur'], a.get('External id'), a.get('grid'), a.get('block')))
        elif cat == 'user_annotation':
            name = e['name']
            scopes[name].append((e['ts'], e['ts']+e['dur']))
            scope_cpu[name]['calls'] += 1
            scope_cpu[name]['cpu_scope_us'] += e['dur']
    indexed = {}
    for name, intervals in scopes.items():
        intervals.sort()
        indexed[name] = ([x[0] for x in intervals], intervals)
    by_kernel = defaultdict(lambda: {'calls': 0, 'gpu_us': 0})
    by_op = defaultdict(lambda: {'kernel_calls': 0, 'gpu_us': 0})
    by_scope = defaultdict(float)
    scope_kernels = defaultdict(int)
    attributed = 0
    for name, dur, ext, grid, block in kernels:
        by_kernel[name]['calls'] += 1
        by_kernel[name]['gpu_us'] += dur
        op = ops.get(ext)
        if op:
            attributed += 1
            ts, op_name, shape, dtype = op
            key = json.dumps([op_name, shape, dtype])
            by_op[key]['kernel_calls'] += 1
            by_op[key]['gpu_us'] += dur
            for label, (starts, intervals) in indexed.items():
                i = bisect_right(starts, ts)-1
                if i >= 0 and ts <= intervals[i][1]:
                    by_scope[label] += dur
                    scope_kernels[label] += 1
    report = {'kernel_count': len(kernels), 'attributed_kernel_count': attributed,
              'gpu_kernel_total_us': sum(k[1] for k in kernels),
              'scopes': {k: dict(v, gpu_kernel_us=by_scope[k], kernel_calls=scope_kernels[k]) for k,v in scope_cpu.items()},
              'operators': [dict(name=json.loads(k)[0], shapes=json.loads(k)[1], dtypes=json.loads(k)[2], **v)
                            for k,v in sorted(by_op.items(), key=lambda x:-x[1]['gpu_us'])],
              'kernels': [dict(name=k, **v) for k,v in sorted(by_kernel.items(), key=lambda x:-x[1]['gpu_us'])],
              'notes': 'GPU durations attributed via External id to CPU operator timestamp within module annotation. Module scopes are inclusive and overlap; never add parent and child totals. GPU summed durations are not end-to-end latency.'}
    path = args.trace.parent/'trace-analysis.json'
    path.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps({k:v for k,v in report.items() if k not in ('operators','kernels')}, indent=2))
    print(json.dumps(report['operators'][:18], indent=2))
    print('Saved:', path)


if __name__ == '__main__':
    main()

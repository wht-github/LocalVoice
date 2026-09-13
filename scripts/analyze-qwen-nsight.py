"""Summarize warm ASR request ranges from a Nsight Systems SQLite export."""
import argparse
from collections import defaultdict
import csv
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import statistics

ROOT = Path(__file__).resolve().parents[1]


def encoder_times(run):
    """Wall time around mtmd_batch_encode, including its transfers/batch handling.

    This pinned server logs the start of encoding and the following embedding
    decode. They are not separate GPU-only timers.
    """
    path = ROOT / 'outputs/qwen-real' / f'{run["tag"]}-{run["split"]}.log'
    values = []
    encoding_start = None
    for line in path.read_text(encoding='utf-8').splitlines():
        match = re.match(r'(\d+)\.(\d+)\.(\d+)\.(\d+)', line)
        if not match:
            continue
        minutes, seconds, millis, micros = map(int, match.groups())
        timestamp = minutes*60000 + seconds*1000 + millis + micros/1000
        if 'new prompt, n_ctx_slot' in line:
            values.append(0.0)
        elif 'encoding mtmd batch from idx' in line:
            encoding_start = timestamp
        elif 'decoding audio batch' in line and encoding_start is not None:
            values[-1] += timestamp - encoding_start
            encoding_start = None
    assert len(values) == len(run['quality']) + len(run['warmups']) + len(run['performance'])
    assert encoding_start is None and all(value > 0 for value in values)
    performance = {}
    offset = len(run['quality'])
    for case_id in run['performance_ids']:
        performance[case_id] = values[offset+1:offset+1+run['repetitions']]
        offset += 1 + run['repetitions']
    return performance, digest(path)


def digest(path):
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()


def union_ns(intervals):
    total = 0
    end = 0
    for start, stop in sorted(intervals):
        total += max(0, stop - max(start, end))
        end = max(end, stop)
    return total


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--trace', type=Path, required=True)
    parser.add_argument('--profile', type=Path, required=True)
    parser.add_argument('--baseline', type=Path, nargs='+', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profile.read_text(encoding='utf-8'))
    baselines = [json.loads(p.read_text(encoding='utf-8')) for p in args.baseline]
    def fixed_command(run):
        command = run['command'].copy()
        command[command.index('--port') + 1] = '<ephemeral-port>'
        return command
    if not profile.get('nsys_stopped_before_model_exit'):
        raise RuntimeError('Use a capture stopped before the CUDA server is terminated')
    for run in [profile] + baselines:
        assert 'error' not in run
        assert len(run['quality']) == 30
        assert len(run['performance']) == 6 * run['repetitions']
        assert run['model_sha256'] == profile['model_sha256']
        assert run['mmproj_sha256'] == profile['mmproj_sha256']
        assert run['context'] == profile['context'] == 2048
        assert fixed_command(run) == fixed_command(profile)
        assert run['manifest_sha256'] == profile['manifest_sha256']
        for row in run['quality'] + run['performance'] + run['warmups']:
            assert row['finish_reason'] == 'stop' and row['timings']['cache_n'] == 0
    connection = sqlite3.connect(f'{args.trace.resolve().as_uri()}?mode=ro', uri=True)
    strings = dict(connection.execute('SELECT id,value FROM StringIds'))
    ranges = []
    marker_count = 0
    for start, end, text, text_id in connection.execute('SELECT start,end,text,textId FROM NVTX_EVENTS WHERE end IS NOT NULL'):
        label = text or strings.get(text_id, '')
        if label.startswith(('quality/', 'warmup/', 'performance/')):
            marker_count += 1
        if label.startswith('performance/'):
            ranges.append((start, end, label))
    expected = {f'performance/{r["id"]}/{r["iteration"]}' for r in profile['performance']}
    assert marker_count == len(profile['quality']) + len(profile['warmups']) + len(profile['performance'])
    assert len(ranges) == len(expected) and {r[2] for r in ranges} == expected
    kernel_totals = defaultdict(lambda: {'ns': 0, 'count': 0})
    shape_totals = defaultdict(lambda: {'ns': 0, 'count': 0})
    api_totals = defaultdict(lambda: {'ns': 0, 'count': 0})
    requests = []
    for start, end, label in sorted(ranges):
        kernels = connection.execute('''SELECT start,end,demangledName,gridX,gridY,gridZ,blockX,blockY,blockZ
            FROM CUPTI_ACTIVITY_KIND_KERNEL WHERE start>=? AND end<=?''', (start, end)).fetchall()
        assert kernels, f'No GPU events in {label}'
        for begin, stop, name_id, *shape in kernels:
            name = strings[name_id]
            family = name.split('<')[0].split('(')[0].removeprefix('void ')
            for totals, key in [(kernel_totals, family), (shape_totals, (name, *shape))]:
                totals[key]['ns'] += stop - begin
                totals[key]['count'] += 1
        copies = connection.execute('SELECT start,end,bytes FROM CUPTI_ACTIVITY_KIND_MEMCPY WHERE start>=? AND end<=?', (start, end)).fetchall()
        sets = connection.execute('SELECT start,end FROM CUPTI_ACTIVITY_KIND_MEMSET WHERE start>=? AND end<=?', (start, end)).fetchall()
        api = connection.execute('SELECT start,end,nameId FROM CUPTI_ACTIVITY_KIND_RUNTIME WHERE start>=? AND end<=?', (start, end)).fetchall()
        for begin, stop, name_id in api:
            api_totals[strings[name_id]]['ns'] += stop - begin
            api_totals[strings[name_id]]['count'] += 1
        allocations = connection.execute('SELECT count(*) FROM CUDA_GPU_MEMORY_USAGE_EVENTS WHERE start>=? AND start<=? AND memKind=2 AND memoryOperationType=0', (start, end)).fetchone()[0]
        requests.append({'label': label, 'wall_ms': (end-start)/1e6,
                         'kernel_count': len(kernels), 'kernel_sum_ms': sum(k[1]-k[0] for k in kernels)/1e6,
                         'gpu_active_union_ms': union_ns([(r[0], r[1]) for r in kernels+copies+sets])/1e6,
                         'gpu_tail_to_response_ms': (end-max(k[1] for k in kernels))/1e6,
                         'memcpy_ms': sum(r[1]-r[0] for r in copies)/1e6,
                         'memcpy_bytes': sum(r[2] for r in copies), 'device_allocations': allocations,
                         'graph_launches': sum('cudaGraphLaunch' in strings[r[2]] for r in api),
                         'graph_instantiations': sum('cudaGraphInstantiate' in strings[r[2]] for r in api)})
    total_kernel_ns = sum(r['ns'] for r in kernel_totals.values())
    families = [{'name': name, 'milliseconds': row['ns']/1e6, 'count': row['count'],
                 'percent_kernel_time': 100*row['ns']/total_kernel_ns}
                for name, row in sorted(kernel_totals.items(), key=lambda pair: -pair[1]['ns'])]
    baseline_rows = []
    log_hashes = {}
    for run in baselines:
        encoder, log_hash = encoder_times(run)
        log_hashes[f'{run["tag"]}-{run["split"]}.log'] = log_hash
        for case_id in run['performance_ids']:
            rows = [r for r in run['performance'] if r['id'] == case_id]
            row = rows[0]
            baseline_rows.append({'split': run['split'], 'id': case_id, 'language': row['language'],
                'audio_seconds': row['audio_seconds'], 'tokens': row['timings']['predicted_n'],
                'request_ms': statistics.median(r['seconds']*1000 for r in rows),
                'prompt_ms': statistics.median(r['timings']['prompt_ms'] for r in rows),
                'encoder_wall_ms': statistics.median(encoder[case_id]),
                'prompt_other_ms': statistics.median(r['timings']['prompt_ms']-enc for r,enc in zip(rows,encoder[case_id])),
                'decode_ms': statistics.median(r['timings']['predicted_ms'] for r in rows),
                'other_ms': statistics.median(r['seconds']*1000-r['timings']['prompt_ms']-r['timings']['predicted_ms'] for r in rows),
                'decode_ms_per_token': statistics.median(r['timings']['predicted_per_token_ms'] for r in rows),
                'stable_text': len({r['text'] for r in rows}) == 1})
    allocations = [{'time_ms': r[0]/1e6, 'bytes': r[1], 'operation': r[2]}
                   for r in connection.execute('SELECT start,bytes,memoryOperationType FROM CUDA_GPU_MEMORY_USAGE_EVENTS WHERE memKind=2 ORDER BY start')]
    same_split = next(r for r in baselines if r['split'] == profile['split'])
    quality_differences = [r['id'] for r, p in zip(same_split['quality'], profile['quality']) if r['text'] != p['text']]
    assert [r['id'] for r in same_split['quality']] == [r['id'] for r in profile['quality']]
    report = {'trace_sha256': digest(args.trace), 'profile_sha256': digest(args.profile),
              'server_log_sha256': log_hashes,
              'baseline_sha256': {p.name: digest(p) for p in args.baseline},
              'request_ranges': requests, 'kernel_families': families,
              'baseline_cases': baseline_rows, 'profile_quality_differences': quality_differences,
              'capture_checks': {'request_markers': marker_count,
                  'performance_markers': len(ranges),
                  'repeat_kernel_counts': {case_id: [r['kernel_count'] for r in requests if r['label'].split('/')[1] == case_id]
                                           for case_id in profile['performance_ids']}},
              'device_allocation_events': allocations,
              'diagnostics': [dict(zip(['severity','text','global_pid'], r)) for r in connection.execute('SELECT severity,text,globalPid FROM DIAGNOSTIC_EVENT WHERE severity>=2')],
              'cuda_api': [{'name': k, 'total_ms': v['ns']/1e6, 'count': v['count']} for k,v in sorted(api_totals.items(),key=lambda pair:-pair[1]['ns'])]}
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / 'nsight-summary.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    for name, rows in [('baseline-cases',baseline_rows),('profile-requests',requests),('kernel-families',families)]:
        with (args.output / f'{name}.csv').open('w', encoding='utf-8', newline='') as handle:
            writer = csv.DictWriter(handle, fieldnames=rows[0].keys())
            writer.writeheader()
            writer.writerows(rows)
    with (args.output / 'kernel-shapes.csv').open('w', encoding='utf-8', newline='') as handle:
        writer = csv.writer(handle)
        writer.writerow(['kernel','grid_x','grid_y','grid_z','block_x','block_y','block_z','count','total_ms'])
        for key, row in sorted(shape_totals.items(), key=lambda pair:-pair[1]['ns']):
            writer.writerow([*key,row['count'],row['ns']/1e6])
    print(json.dumps({'warm_ranges':len(requests), 'top_kernels':families[:5],
                      'warm_device_allocations':sum(r['device_allocations'] for r in requests),
                      'quality_differences':quality_differences}, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()

"""Validate the finished ablations and produce compact, auditable tables."""
import csv
import hashlib
import json
from pathlib import Path
import statistics

from qwen_asr_metrics import aggregate, units

ROOT = Path(__file__).resolve().parents[1]
RESULTS = ROOT / 'experiments/qwen_asr/results'


def normalized_command(report):
    command = report['command'].copy()
    for option in ('-m', '--port'):
        command[command.index(option) + 1] = option
    if report['tag'] == 'q8-no-fa':
        command[command.index('-fa') + 1] = 'on'
    return command


def main():
    reports = {}
    rows = []
    evidence = {}
    quantization = json.loads((RESULTS.parent / 'quantization.json').read_text(encoding='utf-8'))
    model_hashes = {'bf16': quantization['source_sha256']}
    for tag, quant in [('q8', 'Q8_0'), ('q6', 'Q6_K'), ('q4', 'Q4_K_M')]:
        model_hashes[tag] = next(r['sha256'] for r in quantization['models'] if r['type'] == quant)
    for split, tags in [('pilot', ['bf16', 'q8', 'q6', 'q4', 'q8-no-graphs', 'q8-no-fa']),
                        ('holdout', ['bf16', 'q8', 'q4'])]:
        for tag in tags:
            key = f'{tag}-{split}'
            report = json.loads((RESULTS / f'{key}.json').read_text(encoding='utf-8'))
            assert 'error' not in report, key
            assert report['model_sha256'] == model_hashes[tag.split('-')[0]], key
            assert len(report['quality']) == 30 and len(report['performance']) == 18 and len(report['warmups']) == 6, key
            assert report['summary']['overall_mer'] == aggregate(report['quality'], 'mer'), key
            for row in report['quality'] + report['performance'] + report['warmups']:
                assert row['finish_reason'] == 'stop' and row['timings']['cache_n'] == 0, (key, row['id'])
            assert all(p['stable_text'] for p in report['summary']['performance'].values()), key
            reports[key] = report
            reference = reports.get(f'q8-{split}', report)
            if tag.startswith('q8-'):
                assert report['model_sha256'] == reference['model_sha256']
            log_path = ROOT / f'outputs/qwen-real/{key}.log'
            log = log_path.read_text(encoding='utf-8')
            assert 'offloaded 29/29 layers to GPU' in log and 'CLIP using CUDA0 backend' in log, key
            evidence[key] = {
                'log_sha256': hashlib.sha256(log_path.read_bytes()).hexdigest(),
                'lines': [line for line in log.splitlines() if any(text in line for text in (
                    'offloaded 29/29', 'model buffer size', 'KV buffer size',
                    'CLIP using CUDA0', 'flash_attn ', 'warmup: flash attention'))],
            }
            summary = report['summary']
            perf = list(summary['performance'].values())
            rows.append({'split': split, 'tag': tag, 'main_model_gb': report['model_bytes'] / 1e9,
                         'overall_mer': summary['overall_mer']['rate'],
                         'zh_cer': summary['by_language']['zh']['rate'],
                         'en_wer': summary['by_language']['en']['rate'],
                         'mixed_mer': summary['by_language']['mixed']['rate'],
                         'mean_case_median_seconds': statistics.mean(v['median_seconds'] for v in perf),
                         'median_case_decode_ms_per_token': statistics.median(v['decode_ms_per_token'] for v in perf),
                         'peak_device_mib': summary['peak_device_mib']})
    first = reports['q8-pilot']
    assert first['manifest_sha256'] == hashlib.sha256((RESULTS.parent / 'ascend-manifest.json').read_bytes()).hexdigest()
    for key, report in reports.items():
        assert normalized_command(report) == normalized_command(first), key
        assert report['manifest_sha256'] == first['manifest_sha256'] and report['mmproj_sha256'] == first['mmproj_sha256'], key
        assert report['source_commit'] == first['source_commit'], key
        assert report['cuda_graphs_disabled'] == (report['tag'] == 'q8-no-graphs'), key
        reference = reports[f'q8-{report["split"]}']
        assert [r['id'] for r in report['quality']] == [r['id'] for r in reference['quality']], key
        assert report['performance_ids'] == reference['performance_ids'], key
    assert not ({r['id'] for r in reports['q8-pilot']['quality']} & {r['id'] for r in reports['q8-holdout']['quality']})
    with (RESULTS / 'summary.csv').open('w', newline='', encoding='utf-8') as handle:
        writer = csv.DictWriter(handle, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    comparisons = []
    for split in ('pilot', 'holdout'):
        tables = {tag: {r['id']: r for r in reports[f'{tag}-{split}']['quality']} for tag in ('bf16', 'q8', 'q4')}
        for identity, ref in tables['q8'].items():
            candidate = tables['q4'][identity]
            comparisons.append({'split': split, 'id': identity, 'language': ref['language'],
                                'reference': ref['reference'], 'bf16': tables['bf16'][identity]['text'],
                                'q8': ref['text'], 'q4': candidate['text'],
                                'q8_errors': ref['mer']['errors'], 'q4_errors': candidate['mer']['errors'],
                                'q4_minus_q8_errors': candidate['mer']['errors'] - ref['mer']['errors'],
                                'normalized_q8_q4_equal': units(ref['text']) == units(candidate['text'])})
    with (RESULTS / 'transcriptions.csv').open('w', newline='', encoding='utf-8') as handle:
        writer = csv.DictWriter(handle, fieldnames=list(comparisons[0]))
        writer.writeheader()
        writer.writerows(comparisons)
    (RESULTS / 'backend-evidence.json').write_text(json.dumps(evidence, indent=2) + '\n', encoding='utf-8')
    lines = ['# Generated evaluation summary', '',
             'Mean latency = mean of six per-recording medians (three warm measurements each).',
             'VRAM = device-wide sampled peak, including other applications. Different splits use different recordings.', '',
             '| Split | Configuration | Overall MER | Latency (s) | Decode (ms/token) | Device peak (MiB) |',
             '| --- | --- | ---: | ---: | ---: | ---: |']
    for row in rows:
        lines.append(f'| {row["split"]} | {row["tag"]} | {row["overall_mer"]:.3%} | {row["mean_case_median_seconds"]:.3f} | {row["median_case_decode_ms_per_token"]:.2f} | {row["peak_device_mib"]:.0f} |')
    (RESULTS / 'summary.md').write_text('\n'.join(lines) + '\n', encoding='utf-8')
    print('Validated 9 configurations and 486 requests: fixed controls, distinct splits, no truncation/cache reuse, stable warm outputs, GPU offload confirmed.')


if __name__ == '__main__':
    main()

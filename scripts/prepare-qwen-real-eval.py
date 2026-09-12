"""Freeze a small, stratified ASCEND human-speech pilot and holdout set."""
import hashlib
import io
import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
REVISION = '737e9800ae31be9932ba8464c80366559bd28424'
SHA256 = 'a4c81d2b5ed6124f052089a695972808c16e0ce0c365ec9773c5d1a8fcf043a7'
URL = f'https://huggingface.co/datasets/CAiRE/ASCEND/resolve/{REVISION}/main/test-00000-of-00001.parquet'


def main():
    import pyarrow.parquet as pq
    import soundfile as sf
    cache = ROOT / '.runtime/asr-eval'
    cache.mkdir(parents=True, exist_ok=True)
    source = cache / 'ascend-test.parquet'
    if not source.exists():
        print('Downloading ASCEND human speech test split (106 MB)', flush=True)
        partial = source.with_suffix('.partial')
        with urllib.request.urlopen(URL, timeout=60) as response, partial.open('wb') as output:
            while chunk := response.read(1024**2):
                output.write(chunk)
        with partial.open('rb') as handle:
            assert hashlib.file_digest(handle, 'sha256').hexdigest() == SHA256
        partial.replace(source)
    with source.open('rb') as handle:
        assert hashlib.file_digest(handle, 'sha256').hexdigest() == SHA256
    groups = {language: [] for language in ('zh', 'en', 'mixed')}
    for row in pq.read_table(source).to_pylist():
        info = sf.info(io.BytesIO(row['audio']['bytes']))
        if row['language'] in groups and 2 <= info.duration <= 15 and row['transcription'].strip():
            row['actual_seconds'] = info.duration
            groups[row['language']].append(row)
    samples = []
    audio_dir = cache / 'audio'
    audio_dir.mkdir(exist_ok=True)
    for language, rows in groups.items():
        rows.sort(key=lambda r: hashlib.sha256(('qwen-asr-pilot-v1:' + r['id']).encode()).hexdigest())
        if len(rows) < 20:
            raise RuntimeError(f'Not enough {language} samples: {len(rows)}')
        for index, row in enumerate(rows[:20]):
            data = row['audio']['bytes']
            path = audio_dir / f"{row['id']}.wav"
            path.write_bytes(data)
            samples.append({'id': row['id'], 'split': 'pilot' if index < 10 else 'holdout',
                            'language': language, 'speaker': row['original_speaker_id'],
                            'audio': path.relative_to(ROOT).as_posix(),
                            'audio_sha256': hashlib.sha256(data).hexdigest(),
                            'seconds': row['actual_seconds'], 'reference': row['transcription']})
    manifest = {'dataset': 'CAiRE/ASCEND', 'revision': REVISION, 'upstream_split': 'test',
                'source_sha256': SHA256, 'license': 'CC-BY-SA-4.0',
                'citation': 'Lovenia et al., ASCEND: A Spontaneous Chinese-English Dataset for Code-switching in Multi-turn Conversation, 2021, arXiv:2112.06223',
                'selection': 'Actual audio duration 2-15 seconds, nonempty reference. Per language sort SHA256(qwen-asr-pilot-v1: + id); first 10 pilot, next 10 holdout. No hypothesis-based filtering.',
                'samples': samples}
    destination = ROOT / 'experiments/qwen_asr/ascend-manifest.json'
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(f'Frozen {len(samples)} samples; {sum(r["seconds"] for r in samples):.1f} seconds; speakers {sorted({r["speaker"] for r in samples})}', flush=True)


if __name__ == '__main__':
    main()

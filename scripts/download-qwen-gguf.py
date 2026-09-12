"""Download the pinned Qwen3-ASR 1.7B Q8 decoder and BF16 audio encoder."""
import argparse
import hashlib
import json
from pathlib import Path
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / '.runtime/models/Qwen3-ASR-1.7B-GGUF'
REPO = 'ggml-org/Qwen3-ASR-1.7B-GGUF'
REVISION = '36a678687ba7d07a74ca70ccb0e36902e005fb80'
FILES = {
    'Qwen3-ASR-1.7B-Q8_0.gguf': (2165034944, '58e22d0532d4eacaf034cfac17a6fed159f37c41390c710186783be439d1fc57'),
    'mmproj-Qwen3-ASR-1.7B-bf16.gguf': (641773984, '8882e9ddab3186f9aa71b1417c847177913e1466655ac944cf86e9b846735d62'),
}
BF16 = ('Qwen3-ASR-1.7B-bf16.gguf', (4069674944, '1af18763dfafde2bbf071ef8a0952f7bee66f140c3565e5ccc5afb07dc1f9227'))


def download(item):
    name, (size, digest) = item
    target = DEST / name
    if not target.exists():
        partial = target.with_suffix('.partial')
        url = f'https://huggingface.co/{REPO}/resolve/{REVISION}/{name}?download=true&nocache={time.time_ns()}'
        print(f'Downloading {name} ({size / 1e9:.2f} GB)', flush=True)
        with urllib.request.urlopen(url, timeout=60) as response, partial.open('wb') as output:
            received = 0
            next_log = 128 * 1024**2
            while chunk := response.read(4 * 1024**2):
                output.write(chunk)
                received += len(chunk)
                if received >= next_log:
                    print(f'{name}: {received / size:.0%}', flush=True)
                    next_log += 128 * 1024**2
        if partial.stat().st_size != size:
            raise RuntimeError(f'Incomplete download: {name}')
        with partial.open('rb') as source:
            if hashlib.file_digest(source, 'sha256').hexdigest() != digest:
                raise RuntimeError(f'Checksum mismatch: {name}')
        partial.replace(target)
    with target.open('rb') as source:
        if target.stat().st_size != size or hashlib.file_digest(source, 'sha256').hexdigest() != digest:
            raise RuntimeError(f'Checksum mismatch: {name}')
    print(f'Verified {name}', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--with-bf16', action='store_true', help='Also download the source for local quantization.')
    args = parser.parse_args()
    if args.with_bf16:
        FILES[BF16[0]] = BF16[1]
    DEST.mkdir(parents=True, exist_ok=True)
    with ThreadPoolExecutor(max_workers=2) as pool:
        list(pool.map(download, FILES.items()))
    (DEST / 'manifest.json').write_text(json.dumps({
        'repo': REPO, 'revision': REVISION,
        'files': {name: {'bytes': size, 'sha256': sha} for name, (size, sha) in FILES.items()},
    }, indent=2) + '\n', encoding='utf-8')

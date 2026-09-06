#!/usr/bin/env bash
set -euo pipefail
[[ "${WSL_DISTRO_NAME:-}" == LocalVoice && "$EUID" != 0 ]] || exit 1
root=/home/voice/.local/share/local-voice-app
"$root/bootstrap/bin/uv" venv "$root/benchmark" --python /usr/bin/python3
"$root/bootstrap/bin/uv" pip install --python "$root/benchmark/bin/python" --index-url https://pypi.tuna.tsinghua.edu.cn/simple 'pyarrow==23.0.1' 'numpy==1.26.4' 'soundfile==0.13.1' 'httpx==0.28.1'
"$root/benchmark/bin/python" - <<'PY'
import hashlib
from pathlib import Path
import httpx
root=Path('/home/voice/.local/share/local-voice-app/benchmark-data')
root.mkdir(exist_ok=True)
target=root/'ascend-test.parquet'
expected='a4c81d2b5ed6124f052089a695972808c16e0ce0c365ec9773c5d1a8fcf043a7'
if not target.exists() or hashlib.sha256(target.read_bytes()).hexdigest()!=expected:
    with httpx.stream('GET','https://hf-mirror.com/datasets/CAiRE/ASCEND/resolve/main/main/test-00000-of-00001.parquet',follow_redirects=True,timeout=120) as response:
        response.raise_for_status()
        with target.with_suffix('.partial').open('wb') as file:
            for chunk in response.iter_bytes(): file.write(chunk)
    part=target.with_suffix('.partial')
    assert hashlib.sha256(part.read_bytes()).hexdigest()==expected, 'Dataset checksum mismatch'
    part.replace(target)
import pyarrow.parquet as pq
table=pq.read_table(target)
print(table.schema)
row=table.slice(0,1).to_pylist()[0]
print({k:v for k,v in row.items() if k!='audio'})
print('audio fields:', list(row['audio']))
print('rows:',len(table))
PY

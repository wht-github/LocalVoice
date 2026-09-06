"""Download the pinned official CPU wheel with verified, bounded range requests."""
import concurrent.futures
import hashlib
import os
import time
import urllib.request
from pathlib import Path

URL = "https://download.pytorch.org/whl/cpu/torch-2.9.1%2Bcpu-cp312-cp312-manylinux_2_28_x86_64.whl"
SIZE = 184378187
SHA256 = "7417d8c565f219d3455654cb431c6d892a3eb40246055e14d645422de13b9ea1"
target = Path(os.environ["VOICE_RUNTIME"]) / "downloads" / "torch-2.9.1+cpu-cp312-cp312-manylinux_2_28_x86_64.whl"
target.parent.mkdir(parents=True, exist_ok=True)
if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() == SHA256:
    print(target)
    raise SystemExit(0)
parts = target.with_suffix(".parts")
parts.mkdir(exist_ok=True)
chunk = 4 * 1024 * 1024


def fetch(start):
    end = min(start + chunk, SIZE) - 1
    part = parts / str(start)
    if part.exists() and part.stat().st_size == end - start + 1:
        return part
    for attempt in range(4):
        try:
            req = urllib.request.Request(URL + f"?part={start}", headers={"Range": f"bytes={start}-{end}"})
            with urllib.request.urlopen(req, timeout=60) as response:
                assert response.status == 206, response.status
                assert response.headers["Content-Range"] == f"bytes {start}-{end}/{SIZE}"
                data = response.read()
            assert len(data) == end - start + 1
            part.write_bytes(data)
            print(f"Downloaded {end + 1}/{SIZE}", flush=True)
            return part
        except Exception:
            if attempt == 3:
                raise
            time.sleep(1)


with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    files = list(pool.map(fetch, range(0, SIZE, chunk)))
with target.open("wb") as stream:
    for file in files:
        stream.write(file.read_bytes())
if hashlib.sha256(target.read_bytes()).hexdigest() != SHA256:
    raise RuntimeError("Official PyTorch wheel SHA256 mismatch")
print(f"Verified: {target}")

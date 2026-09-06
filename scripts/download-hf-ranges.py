"""Resume verified HF mirror downloads in bounded chunks, then seed the HF cache."""
import concurrent.futures
import hashlib
import os
from pathlib import Path
import sys
import time
import requests
from huggingface_hub import get_hf_file_metadata, hf_hub_url

repo, filename = sys.argv[1:3]
url = hf_hub_url(repo, filename, endpoint="https://hf-mirror.com")
meta = get_hf_file_metadata(url)
assert len(meta.etag) == 64, "Only SHA256-addressed LFS weights are supported"
cache = Path(os.environ["HF_HOME"]) / "hub" / ("models--" + repo.replace("/", "--"))
blob = cache / "blobs" / meta.etag
blob.parent.mkdir(parents=True, exist_ok=True)
if not blob.exists():
    parts = Path(os.environ["VOICE_RUNTIME"]) / "downloads" / (meta.etag + ".parts")
    parts.mkdir(exist_ok=True)
    step = 4 * 1024**2
    def fetch(start):
        end = min(meta.size, start + step) - 1
        path = parts / str(start)
        if path.exists() and path.stat().st_size == end - start + 1:
            return path
        for attempt in range(5):
            try:
                response = requests.get(url + f"?download=true&part={start}",
                    headers={"Range": f"bytes={start}-{end}"}, timeout=(15, 60))
                response.raise_for_status()
                assert response.status_code == 206
                assert response.headers["Content-Range"] == f"bytes {start}-{end}/{meta.size}"
                assert len(response.content) == end - start + 1
                path.write_bytes(response.content)
                return path
            except Exception:
                if attempt == 4: raise
                time.sleep(1 + attempt)
    starts = list(range(0, meta.size, step))
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        for index, _ in enumerate(pool.map(fetch, starts)):
            print(f"{repo}/{filename}: {index + 1}/{len(starts)} parts", flush=True)
    temporary = blob.with_suffix(".assembled")
    digest = hashlib.sha256()
    with temporary.open("wb") as output:
        for start in starts:
            data = (parts / str(start)).read_bytes()
            digest.update(data)
            output.write(data)
    assert temporary.stat().st_size == meta.size and digest.hexdigest() == meta.etag
    temporary.replace(blob)
snapshot = cache / "snapshots" / meta.commit_hash
snapshot.mkdir(parents=True, exist_ok=True)
link = snapshot / filename
if not link.exists(): link.symlink_to(Path("../../blobs") / meta.etag)
(cache / "refs").mkdir(exist_ok=True)
(cache / "refs/main").write_text(meta.commit_hash)
print(f"Verified SHA256 {meta.etag}", flush=True)

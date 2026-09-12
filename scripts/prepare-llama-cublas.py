"""Add NVIDIA's matching Windows import libraries, absent from the cuBLAS wheel."""

import hashlib
import shutil
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CUDA = Path(sys.prefix) / "Lib/site-packages/nvidia/cu13"
ARCHIVE = "libcublas-windows-x86_64-13.7.0.27-archive"
SHA256 = "fff93984ee8a85dd8568e4b9000f9e3ef7153f0f73b176756bcbad3c6622d1f0"
URL = f"https://developer.download.nvidia.com/compute/cuda/redist/libcublas/windows-x86_64/{ARCHIVE}.zip"


def main():
    if not (CUDA / "bin/nvcc.exe").is_file():
        raise SystemExit("Run with .venv-llama-build/Scripts/python.exe after installing requirements.")
    cache = ROOT / ".runtime/llama-downloads"
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / f"{ARCHIVE}.zip"
    if not archive.exists():
        print(f"Downloading official cuBLAS archive: {URL}", flush=True)
        partial = archive.with_suffix(".partial")
        with urllib.request.urlopen(URL, timeout=60) as response, partial.open("wb") as target:
            shutil.copyfileobj(response, target)
        with partial.open("rb") as source:
            if hashlib.file_digest(source, "sha256").hexdigest() != SHA256:
                raise RuntimeError("cuBLAS archive checksum mismatch")
        partial.replace(archive)
    with archive.open("rb") as source:
        if hashlib.file_digest(source, "sha256").hexdigest() != SHA256:
            raise RuntimeError(f"Checksum mismatch: {archive}")
    with zipfile.ZipFile(archive) as package:
        for name in ("cublas.lib", "cublasLt.lib"):
            destination = CUDA / "lib/x64" / name
            with package.open(f"{ARCHIVE}/lib/x64/{name}") as source, destination.open("wb") as target:
                shutil.copyfileobj(source, target)
            print(f"Installed {destination}")


if __name__ == "__main__":
    main()

"""Download a pinned, checksum-verified text model; Agentic is the app default."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
MODELS = {
    "agentic": ("2680cd2ec627e7ae61d9288b5d6f314cb6af2e6a", 688065792,
                "1dcbbbfde39742b213101c0d5fe645bfda18e5c9b693409205c481efbb487758"),
    "3.5-4B": ("b0bd07786f94b700012140fd9263b2de83d4f97b", 2740937888,
               "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4"),
    "0.6B": ("23749fefcc72300e3a2ad315e1317431b06b590a", 639446688,
             "9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031"),
    "1.7B": ("90862c4b9d2787eaed51d12237eafdfe7c5f6077", 1834426016,
             "061b54daade076b5d3362dac252678d17da8c68f07560be70818cace6590cb1a"),
}


def verify(path, size, digest):
    if path.stat().st_size != size:
        raise RuntimeError(f"Incomplete model: {path.name}")
    with path.open("rb") as source:
        if hashlib.file_digest(source, "sha256").hexdigest() != digest:
            raise RuntimeError(f"Checksum mismatch: {path.name}")


def download(model_size="agentic"):
    revision, size, digest = MODELS[model_size]
    if model_size == "agentic":
        family, repo, quantization = "AgenticASR-Refiner", "MigoXV/AgenticASR-Refiner-GGUF", "Q4_K_M"
    else:
        # Retained for reproducible older experiments, not selected by the app.
        family = "Qwen3.5-4B" if model_size == "3.5-4B" else f"Qwen3-{model_size}"
        repo = f"unsloth/{family}-GGUF" if model_size == "3.5-4B" else f"Qwen/{family}-GGUF"
        quantization = "Q4_K_M" if model_size == "3.5-4B" else "Q8_0"
    name = f"{family}-{quantization}.gguf"
    destination = ROOT / f".runtime/models/{family}-GGUF"
    destination.mkdir(parents=True, exist_ok=True)
    target = destination / name
    if not target.exists():
        partial = target.with_suffix(".partial")
        endpoint = os.environ.get("HF_ENDPOINT", "https://huggingface.co").rstrip("/")
        url = f"{endpoint}/{repo}/resolve/{revision}/{name}?download=true&nocache={time.time_ns()}"
        print(f"Downloading {name} ({size / 1e9:.2f} GB)", flush=True)
        deadline = time.monotonic() + 12 * 60
        with urllib.request.urlopen(url, timeout=30) as response, partial.open("wb") as output:
            received = 0
            next_log = 128 * 1024**2
            while chunk := response.read(4 * 1024**2):
                if time.monotonic() > deadline or received + len(chunk) > size:
                    raise RuntimeError("Text model download exceeded its time or size limit.")
                output.write(chunk)
                received += len(chunk)
                if received >= next_log:
                    print(f"{name}: {received / size:.0%}", flush=True)
                    next_log += 128 * 1024**2
        verify(partial, size, digest)
        partial.replace(target)
    verify(target, size, digest)
    (destination / "manifest.json").write_text(json.dumps({
        "repo": repo, "revision": revision,
        "files": {name: {"bytes": size, "sha256": digest}},
    }, indent=2) + "\n", encoding="utf-8")
    print(f"Verified {name}", flush=True)
    return target


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--size", choices=MODELS, default="agentic")
    download(parser.parse_args().size)

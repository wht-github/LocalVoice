"""Fetch only required model files through HF_ENDPOINT (hf-mirror.com)."""
import os
from pathlib import Path
import zipfile
import urllib.request
import socket
from huggingface_hub import snapshot_download

assert os.environ["HF_ENDPOINT"] == "https://hf-mirror.com"
snapshot_download("myshell-ai/MeloTTS-Chinese", allow_patterns=["config.json", "checkpoint.pth"])
snapshot_download("bert-base-multilingual-uncased", allow_patterns=["config.json", "tokenizer*", "vocab.txt", "model.safetensors"])
snapshot_download("bert-base-uncased", allow_patterns=["config.json", "tokenizer*", "vocab.txt"])
root = Path(os.environ["NLTK_DATA"])
socket.setdefaulttimeout(30)
for category, package in [("corpora", "cmudict"), ("taggers", "averaged_perceptron_tagger"), ("taggers", "averaged_perceptron_tagger_eng")]:
    target = root / category
    target.mkdir(parents=True, exist_ok=True)
    if not (target / package).exists():
        # Linguistic dictionaries, not model weights. Fixed upstream NLTK data.
        archive = target / (package + ".zip")
        if not zipfile.is_zipfile(archive):
            temporary = archive.with_suffix(".download")
            urllib.request.urlretrieve(f"https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/{category}/{package}.zip", temporary)
            assert zipfile.is_zipfile(temporary), package
            temporary.replace(archive)
        with zipfile.ZipFile(archive) as z:
            z.extractall(target)
print("MeloTTS model and linguistic resources cached.")

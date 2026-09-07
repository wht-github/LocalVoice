import json
from pathlib import Path

import httpx

with httpx.Client(timeout=90, trust_env=False) as client:
    for name in ['melo-mixed.wav', 'melo-mandarin.wav']:
        path = Path('outputs') / name
        if not path.is_file():
            continue
        with path.open('rb') as f:
            response = client.post('http://127.0.0.1:8001/v1/audio/transcriptions',
                                   files={'file': (name, f, 'audio/wav')},
                                   data={'language': 'auto'})
        print(name, json.dumps(response.json(), ensure_ascii=True), flush=True)

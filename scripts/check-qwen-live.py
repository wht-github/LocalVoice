import io
import json
from pathlib import Path

import httpx
import soundfile as sf

with httpx.Client(timeout=90, trust_env=False) as client:
    print('health:', client.get('http://127.0.0.1:8001/health').text, flush=True)
    root = Path('/home/voice/.local/share/local-voice-app/modelscope')
    source = next(root.rglob('example/en.mp3'))
    samples, rate = sf.read(source)
    wav = io.BytesIO()
    sf.write(wav, samples, rate, format='WAV')
    response = client.post('http://127.0.0.1:8001/v1/audio/transcriptions',
                           files={'file': ('en.wav', wav.getvalue(), 'audio/wav')},
                           data={'language': 'auto'})
    print('asr:', json.dumps(response.json(), ensure_ascii=True), flush=True)

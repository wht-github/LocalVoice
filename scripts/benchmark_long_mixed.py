"""Reproducible real-speech code-switching test; no model dependency changes."""
import collections
import io
import json
import random
import re
import subprocess
import sys
import time
import unicodedata
from pathlib import Path

import httpx
import numpy as np
import pyarrow.parquet as pq
import soundfile as sf

ROOT = Path('/home/voice/.local/share/local-voice-app/benchmark-data')
OUT = Path('/opt/local-voice-app/outputs/long-mixed')
OUT.mkdir(exist_ok=True)
RATE = 16000

def tokens(text):
    return re.findall(r"[\u3400-\u9fff]|[a-z]+(?:'[a-z]+)?|[0-9]+|[^\W\d_]+", unicodedata.normalize('NFKC', text).lower())

def distance(a, b):
    prev = list(range(len(b)+1))
    for i,x in enumerate(a,1):
        row=[i]
        for j,y in enumerate(b,1):
            row.append(min(row[-1]+1, prev[j]+1, prev[j-1]+(x!=y)))
        prev=row
    return prev[-1]

if '--rescore' in sys.argv:
    path=OUT/'results.json'
    result=json.loads(path.read_text())
    reference=(OUT/'reference.txt').read_text()
    for name,run in result['runs'].items():
        for item in run['measurements']:
            if 'reference' in item:
                item['errors']=distance(tokens(item['reference']),tokens(item['text']))
                item['reference_tokens']=len(tokens(item['reference']))
        run['errors']=sum(m['errors'] for m in run['measurements']) if name=='utterances_auto' else distance(tokens(reference),tokens(' '.join(m['text'] for m in run['measurements'])))
        run['reference_tokens']=len(tokens(reference))
        run['mer']=run['errors']/run['reference_tokens']
        print(name,run['errors'],run['mer'])
    result['scoring']='lowercase NFKC; Chinese characters + English words + number groups; other-script letter runs counted as tokens; punctuation ignored; MER is edit distance / reference tokens'
    path.write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    sys.exit(0)

pid=int(subprocess.check_output(['systemctl','show','local-voice-asr','-p','MainPID','--value'],text=True))
def mem():
    lines=Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines()
    return {line.split(':')[0]: int(line.split()[1])/1024 for line in lines if line.startswith(('Pss:','Rss:'))}

def wav(samples):
    buffer=io.BytesIO()
    sf.write(buffer,samples,RATE,format='WAV',subtype='PCM_16')
    return buffer.getvalue()

all_rows=pq.read_table(ROOT/'ascend-test.parquet').to_pylist()
eligible=[r for r in all_rows if re.search(r'[\u3400-\u9fff]',r['transcription']) and re.search(r'[A-Za-z]',r['transcription']) and 1 <= r['duration'] <= 20]
random.Random(20260906).shuffle(eligible)
selected=[]
duration=0
for row in eligible:
    audio,rate=sf.read(io.BytesIO(row['audio']['bytes']),dtype='float32')
    assert rate==RATE and audio.ndim==1
    row['samples']=audio
    selected.append(row)
    duration+=len(audio)/RATE
    if duration>=600 or len(selected)>=160: break
assert duration>180 and len(selected)>=30
print(f'Selected {len(selected)} real mixed utterances, {duration:.1f}s speech, {len(set(r["original_speaker_id"] for r in selected))} speakers',flush=True)
combined=np.concatenate([np.concatenate([r['samples'],np.zeros(int(.7*RATE),dtype='float32')]) for r in selected])
sf.write(OUT/'ascend-mixed-long.wav',combined,RATE,subtype='PCM_16')
reference=' '.join(r['transcription'] for r in selected)
(OUT/'reference.txt').write_text(reference,encoding='utf-8')
# Energy-based pause detector on the entire waveform; no transcript or clip
# boundaries are passed to the segmentation algorithm. This is not Silero VAD.
frame=320
padded=np.pad(combined,(0,(-len(combined))%frame))
rms=np.sqrt(np.mean(padded.reshape(-1,frame)**2,axis=1))
quiet=rms<.003
pauses=[]
start=None
for i, flag in enumerate(np.append(quiet,False)):
    if flag and start is None: start=i
    elif not flag and start is not None:
        if (i-start)*.02>=.45: pauses.append(int((start+i)/2)*frame)
        start=None
boundaries=[0]
forced=0
while len(combined)-boundaries[-1]>25*RATE:
    begin=boundaries[-1]
    candidates=[p for p in pauses if begin+8*RATE <= p <= begin+25*RATE]
    if candidates: end=candidates[0]
    else: end=begin+25*RATE; forced+=1
    boundaries.append(end)
boundaries.append(len(combined))

result={'dataset':'CAiRE/ASCEND test, CC-BY-SA-4.0','dataset_sha256':'a4c81d2b5ed6124f052089a695972808c16e0ce0c365ec9773c5d1a8fcf043a7',
        'seed':20260906,'eligible_mixed_utterances':len(eligible),'selected_utterances':len(selected),'speakers':len(set(r['original_speaker_id'] for r in selected)),
        'speech_seconds':duration,'assembled_seconds':len(combined)/RATE,'inserted_gap_seconds':.7,'forced_cuts':forced,
        'scoring':'lowercase NFKC; Chinese characters + English words + number groups; other-script letter runs counted as tokens; punctuation ignored; MER is edit distance / reference tokens',
        'asr_device':'cpu','asr_threads':4,'before_memory_mib':mem(),'runs':{}}

with httpx.Client(timeout=180,trust_env=False) as client:
    assert client.get('http://127.0.0.1:8001/health').json()['ready']
    for name,lang,items in [
        ('utterances_auto','auto',[(r['samples'],r['transcription'],r['id']) for r in selected]),
        ('long_pause_auto','auto',[(combined[a:b],None,f'{a/RATE:.2f}-{b/RATE:.2f}') for a,b in zip(boundaries,boundaries[1:])]),
        ('long_pause_zh','zh',[(combined[a:b],None,f'{a/RATE:.2f}-{b/RATE:.2f}') for a,b in zip(boundaries,boundaries[1:])]),
    ]:
        measurements=[]
        started=time.perf_counter()
        for index,(audio,ref,item_id) in enumerate(items):
            t=time.perf_counter()
            response=client.post('http://127.0.0.1:8001/v1/audio/transcriptions',files={'file':('segment.wav',wav(audio),'audio/wav')},data={'language':lang})
            response.raise_for_status()
            answer=response.json()
            entry={'id':item_id,'duration':len(audio)/RATE,'wall_seconds':time.perf_counter()-t,'inference_seconds':answer['inference_seconds'],'text':answer['text'],'detected_language':answer['language'],'memory_mib':mem()}
            if ref is not None:
                entry.update(reference=ref,errors=distance(tokens(ref),tokens(answer['text'])),reference_tokens=len(tokens(ref)))
            measurements.append(entry)
            if (index+1)%20==0: print(f'{name}: {index+1}/{len(items)}',flush=True)
        wall=time.perf_counter()-started
        text=' '.join(m['text'] for m in measurements)
        # Use per-utterance scores for oracle boundaries; global alignment for long audio.
        errors=sum(m['errors'] for m in measurements) if name=='utterances_auto' else distance(tokens(reference),tokens(text))
        length=len(tokens(reference))
        p95=float(np.percentile([m['wall_seconds'] for m in measurements],95))
        summary={'segments':len(items),'wall_seconds':wall,'rtf':wall/sum(m['duration'] for m in measurements),'p95_request_seconds':p95,
            'max_request_seconds':max(m['wall_seconds'] for m in measurements),'max_segment_rtf':max(m['wall_seconds']/m['duration'] for m in measurements),
            'errors':errors,'reference_tokens':length,'mer':errors/length,'empty_results':sum(not m['text'].strip() for m in measurements),
            'sampled_pss_peak_mib':max(m['memory_mib']['Pss'] for m in measurements),'measurements':measurements}
        result['runs'][name]=summary
        (OUT/f'{name}.txt').write_text(text,encoding='utf-8')
        (OUT/'results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
        print(name,json.dumps({k:v for k,v in summary.items() if k!='measurements'},ensure_ascii=False),flush=True)
result['after_memory_mib']=mem()
(OUT/'results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
print('Finished:',OUT,flush=True)

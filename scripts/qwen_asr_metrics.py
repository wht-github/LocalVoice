"""Explicit ASR scoring rules, without semantic corrections or number rewriting."""
import re
import unicodedata


def normalize(text):
    text = unicodedata.normalize('NFKC', text).lower().replace('\u2019', "'")
    return ''.join(c if c.isalnum() or c.isspace() else '' if c == "'" else ' ' for c in text)


def units(text, metric='mer'):
    text = normalize(text)
    if metric == 'cer':
        return [c for c in text if c.isalnum()]
    if metric == 'wer':
        return text.split()
    return re.findall(r'[\u3400-\u9fff]|[a-z0-9]+', text)


def distance(reference, hypothesis):
    previous = list(range(len(hypothesis) + 1))
    for i, left in enumerate(reference, 1):
        current = [i]
        for j, right in enumerate(hypothesis, 1):
            current.append(min(current[-1] + 1, previous[j] + 1,
                               previous[j - 1] + (left != right)))
        previous = current
    return previous[-1]


def score(reference, hypothesis, metric='mer'):
    ref, hyp = units(reference, metric), units(hypothesis, metric)
    return {'errors': distance(ref, hyp), 'reference_units': len(ref)}


def aggregate(rows, metric):
    values = [score(r['reference'], r['text'], metric) for r in rows]
    errors = sum(v['errors'] for v in values)
    count = sum(v['reference_units'] for v in values)
    return {'errors': errors, 'reference_units': count, 'rate': errors / count if count else None}

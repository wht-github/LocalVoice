"""Evaluate local dictation endpoints against an explicit, private JSONL manifest.

Only the Python standard library is needed. No services are started or stopped.
See docs/dictation-evaluation.md for metric definitions and manual review.
"""

import argparse
import csv
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import re
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
import uuid


TOKEN = re.compile(r"[a-z0-9]+(?:[._'-][a-z0-9]+)*|[^\W_]", re.UNICODE)
HAN = re.compile(r"[\u3400-\u9fff\U00020000-\U0002fa1f]")
# Ignore proxy environment variables: these requests are to local model services.
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def edit_distance(reference, actual):
    previous = list(range(len(actual) + 1))
    for row, expected in enumerate(reference, 1):
        current = [row]
        for col, observed in enumerate(actual, 1):
            current.append(min(current[-1] + 1, previous[col] + 1,
                               previous[col - 1] + (expected != observed)))
        previous = current
    return previous[-1]


def error_metric(reference, actual):
    errors = edit_distance(reference, actual)
    return {"errors": errors, "reference_units": len(reference),
            "rate": errors / len(reference) if reference else None}


def normalized(text):
    return unicodedata.normalize("NFKC", text).casefold()


def asr_metrics(reference, actual):
    reference, actual = normalized(reference), normalized(actual)
    chars = lambda text: [char for char in text if char.isalnum()]
    latin = lambda text: [token for token in TOKEN.findall(text)
                          if re.search("[a-z]", token)]
    return {
        "cer": error_metric(chars(reference), chars(actual)),
        "han_cer": error_metric(HAN.findall(reference), HAN.findall(actual)),
        "latin_wer": error_metric(latin(reference), latin(actual)),
        "mixed_token_error": error_metric(TOKEN.findall(reference), TOKEN.findall(actual)),
    }


def term_metrics(text, expected_terms, vocabulary):
    text = unicodedata.normalize("NFKC", text)
    scores = []
    for term in expected_terms:
        forms = [term["text"], *term.get("equivalents", [])]
        spans = set()
        for form in forms:
            # A Chinese neighbour is allowed; an ASCII word fragment is not.
            pattern = re.escape(unicodedata.normalize("NFKC", form))
            if form[0].isascii() and form[0].isalnum():
                pattern = r"(?<![A-Za-z0-9_])" + pattern
            if form[-1].isascii() and form[-1].isalnum():
                pattern += r"(?![A-Za-z0-9_])"
            spans.update(match.span() for match in re.finditer(pattern, text))
        # Equivalent spellings must not count the same occurrence twice.
        occurrences, end = 0, -1
        for start, stop in sorted(spans, key=lambda span: (span[0], -span[1])):
            if start >= end:
                occurrences += 1
                end = stop
        expected = term.get("count", 1)
        scores.append({"text": term["text"], "expected": expected,
                       "correct": min(occurrences, expected), "observed": occurrences,
                       "in_vocabulary": term["text"] in vocabulary})
    return scores


def read_manifest(path, split):
    cases, identifiers = [], set()
    for line_number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not line.strip():
            continue
        case = json.loads(line)
        case_id = case.get("id")
        if not isinstance(case_id, str) or not case_id or case_id in identifiers:
            raise ValueError(f"Line {line_number}: id must be a unique nonempty string")
        identifiers.add(case_id)
        if case.get("split") not in {"dev", "test"}:
            raise ValueError(f"{case_id}: split must be dev or test")
        if split != "all" and case["split"] != split:
            continue
        if ("audio" in case) == ("text" in case):
            raise ValueError(f"{case_id}: specify exactly one of audio or text")
        if "audio" in case:
            case["audio"] = str((path.parent / case["audio"]).resolve())
            if not Path(case["audio"]).is_file():
                raise ValueError(f"{case_id}: audio file does not exist: {case['audio']}")
            if not isinstance(case.get("reference"), str) or not case["reference"].strip():
                raise ValueError(f"{case_id}: audio needs a verbatim reference")
        elif not isinstance(case["text"], str) or not case["text"].strip():
            raise ValueError(f"{case_id}: text must be nonempty")
        for field in ("expected_terms", "final_expected_terms"):
            expected_names = set()
            for term in case.get(field, []):
                forms = [term.get("text"), *term.get("equivalents", [])]
                if any(not isinstance(form, str) or not form for form in forms):
                    raise ValueError(f"{case_id}: term forms must be nonempty strings")
                if term["text"] in expected_names:
                    raise ValueError(f"{case_id}: combine repeated expected terms using count")
                expected_names.add(term["text"])
                if type(term.get("count", 1)) is not int or term.get("count", 1) < 1:
                    raise ValueError(f"{case_id}: term count must be a positive integer")
        cases.append(case)
    if not cases:
        raise ValueError("No cases selected")
    return cases


def local_url(value):
    parts = urllib.parse.urlsplit(value)
    if (parts.scheme != "http" or parts.hostname not in {"127.0.0.1", "localhost", "::1"}
            or parts.username or parts.password or parts.query or parts.fragment
            or parts.path not in {"", "/"}):
        raise argparse.ArgumentTypeError("Use a local base URL such as http://127.0.0.1:8001")
    return value.rstrip("/")


def request(url, timeout, body=None, content_type=None):
    headers = {"Content-Type": content_type} if content_type else {}
    req = urllib.request.Request(url, data=body, headers=headers)
    with HTTP.open(req, timeout=timeout) as response:
        return json.load(response)


def transcribe(case, terms, args):
    boundary = uuid.uuid4().hex
    parts = []
    for name, value in {"language": case.get("language", "auto"),
                        "terms": json.dumps(terms, ensure_ascii=False)}.items():
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode())
    audio = Path(case["audio"])
    # A fixed filename avoids putting a private local path in an HTTP header.
    parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="sample{audio.suffix}"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode())
    parts.extend([audio.read_bytes(), f"\r\n--{boundary}--\r\n".encode()])
    return request(args.asr_url + "/v1/audio/transcriptions", args.timeout,
                   b"".join(parts), "multipart/form-data; boundary=" + boundary)


def evaluate(case, terms, args, repetition):
    result = {"id": case["id"], "split": case["split"], "repetition": repetition,
              "kind": "audio" if "audio" in case else "text", "status": "ok",
              "raw_text": None, "final_text": None, "timing_seconds": {}}
    started = time.perf_counter()
    try:
        if "audio" in case:
            response = transcribe(case, terms if args.asr_terms else [], args)
            result["raw_text"] = response["text"]
            result["duration"] = response.get("duration", case.get("duration_seconds"))
            result["timing_seconds"]["asr_request"] = time.perf_counter() - started
            result["timing_seconds"]["asr_server"] = response.get("inference_seconds")
        else:
            result["raw_text"] = case["text"]
        if args.mode == "raw":
            result["final_text"] = result["raw_text"]
        else:
            payload = {"text": result["raw_text"], "mode": args.mode,
                       "terms": terms if args.processing_terms else []}
            processing_started = time.perf_counter()
            response = request(args.text_url + "/v1/text/process", args.timeout,
                               json.dumps(payload, ensure_ascii=False).encode(), "application/json")
            result["timing_seconds"]["processing_request"] = time.perf_counter() - processing_started
            result["timing_seconds"]["processing_server"] = response.get("processing_seconds")
            result["final_text"] = response["text"]
    except (urllib.error.URLError, TimeoutError, OSError, ValueError, KeyError) as exc:
        result.update(status="error", error=str(exc))
    result["timing_seconds"]["total_request"] = time.perf_counter() - started
    vocabulary = {term["text"] for term in terms}
    expected = case.get("expected_terms", [])
    if result["raw_text"] is not None:
        result["raw_terms"] = term_metrics(result["raw_text"], expected, vocabulary)
        if "audio" in case:
            result["asr_metrics"] = asr_metrics(case["reference"], result["raw_text"])
    if result["final_text"] is not None:
        final_expected = expected if args.mode == "raw" else case.get("final_expected_terms", expected)
        result["final_terms"] = term_metrics(result["final_text"], final_expected, vocabulary)
    return result


def aggregate_errors(results):
    aggregate = {}
    for name in ("cer", "han_cer", "latin_wer", "mixed_token_error"):
        metrics = [result["asr_metrics"][name] for result in results if "asr_metrics" in result]
        errors = sum(metric["errors"] for metric in metrics)
        units = sum(metric["reference_units"] for metric in metrics)
        aggregate[name] = {"errors": errors, "reference_units": units,
                           "rate": errors / units if units else None}
    return aggregate


def aggregate_terms(results, key):
    scores = [term for result in results for term in result.get(key, [])]
    groups = {}
    for label, selected in (("all", scores),
                            ("in_vocabulary", [score for score in scores if score["in_vocabulary"]]),
                            ("outside_vocabulary", [score for score in scores if not score["in_vocabulary"]])):
        expected = sum(score["expected"] for score in selected)
        correct = sum(score["correct"] for score in selected)
        groups[label] = {"correct": correct, "expected": expected,
                         "rate": correct / expected if expected else None}
    return groups


def latency_summary(results):
    groups = {}
    for label in ("text", "audio_0_15s", "audio_15_30s", "audio_over_30s", "audio_unknown"):
        selected = []
        for result in results:
            duration = result.get("duration")
            group = ("text" if result["kind"] == "text" else "audio_unknown" if duration is None
                     else "audio_0_15s" if duration <= 15 else "audio_15_30s" if duration <= 30
                     else "audio_over_30s")
            if group == label and result["status"] == "ok":
                selected.append(result)
        if not selected:
            continue
        timings = {}
        for field in ("asr_request", "asr_server", "processing_request", "processing_server", "total_request"):
            values = sorted(result["timing_seconds"][field] for result in selected
                            if result["timing_seconds"].get(field) is not None)
            if values:
                timings[field] = {"n": len(values), "p50": values[math.ceil(len(values) * 0.50) - 1],
                                  "p95": values[math.ceil(len(values) * 0.95) - 1]}
        groups[label] = timings
    return groups


def write_review(output, cases, results):
    by_id = {case["id"]: case for case in cases}
    fields = ["id", "repetition", "status", "raw_text", "final_text", "review_checks",
              "raw_required_edits", "final_required_edits", "serious_semantic_error",
              "all_key_points_preserved", "notes_structure_usable", "reviewer_notes"]
    with (output / "review.csv").open("w", encoding="utf-8-sig", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        for result in results:
            writer.writerow({**{key: result.get(key, "") for key in fields},
                             "review_checks": " | ".join(by_id[result["id"]].get("review_checks", []))})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--output", type=Path, help="New directory; contains private transcripts")
    parser.add_argument("--validate-only", action="store_true")
    parser.add_argument("--split", choices=["dev", "test", "all"], default="dev")
    parser.add_argument("--mode", choices=["raw", "clean", "notes"], default="raw")
    parser.add_argument("--asr-url", type=local_url, default="http://127.0.0.1:8001")
    parser.add_argument("--text-url", type=local_url, help="Defaults to --asr-url")
    parser.add_argument("--terms", type=Path)
    parser.add_argument("--asr-terms", action="store_true")
    parser.add_argument("--processing-terms", action="store_true")
    parser.add_argument("--asr-model", help="Actual model, quantization and revision")
    parser.add_argument("--text-model", help="Actual model, quantization and revision")
    parser.add_argument("--config", help="Runtime/prompt version and relevant hardware/settings")
    parser.add_argument("--state", choices=["cold", "warm", "unknown"])
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=180)
    args = parser.parse_args()
    args.text_url = args.text_url or args.asr_url
    try:
        cases = read_manifest(args.manifest, args.split)
        terms = json.loads(args.terms.read_text(encoding="utf-8-sig")) if args.terms else []
        if not isinstance(terms, list) or any(not isinstance(term, dict)
                or not isinstance(term.get("text"), str) or not term["text"] for term in terms):
            raise ValueError("The terms file must be an array of objects with nonempty text")
        if args.repeat < 1 or args.timeout <= 0:
            raise ValueError("repeat and timeout must be positive")
        if args.validate_only:
            print(f"Validated {len(cases)} cases and {len(terms)} vocabulary entries; no requests sent.")
            return 0
        if not args.output or not args.config or not args.state:
            raise ValueError("A run needs explicit --output, --config and --state")
        if any("audio" in case for case in cases) and not args.asr_model:
            raise ValueError("Audio evaluation needs --asr-model")
        if args.mode != "raw" and not args.text_model:
            raise ValueError("Text processing needs --text-model")
        if (args.asr_terms or args.processing_terms) and not terms:
            raise ValueError("Term ablations need a nonempty --terms file")
        if args.state == "cold" and (len(cases) != 1 or args.repeat != 1):
            raise ValueError("A cold run must contain one case and one repetition; reset externally")
        asr_health = (request(args.asr_url + "/health", args.timeout)
                      if any("audio" in case for case in cases) else None)
        if args.mode != "raw":
            health = request(args.text_url + "/v1/text/health", args.timeout)
            if not health.get("installed"):
                raise ValueError("Text model is not installed; prepare it before evaluation")
            # ready may be false before lazy model loading. Preserve it as evidence.
        else:
            health = None
        args.output.mkdir(parents=True, exist_ok=False)
    except (ValueError, OSError, urllib.error.URLError) as exc:
        parser.error(str(exc))
    metadata = {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()}
    metadata.update(created_at=datetime.now(timezone.utc).isoformat(), text_health=health, asr_health=asr_health,
                    manifest_sha256=hashlib.sha256(args.manifest.read_bytes()).hexdigest(),
                    terms_sha256=hashlib.sha256(args.terms.read_bytes()).hexdigest() if args.terms else None,
                    semantic_review="pending", note="HTTP timing excludes desktop queue, focus checks and typing")
    (args.output / "run.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (args.output / "cases.json").write_text(json.dumps(cases, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (args.output / "terms.json").write_text(json.dumps(terms, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    results = []
    with (args.output / "results.jsonl").open("w", encoding="utf-8") as handle:
        for repetition in range(1, args.repeat + 1):
            for case in cases:
                result = evaluate(case, terms, args, repetition)
                results.append(result)
                handle.write(json.dumps(result, ensure_ascii=False) + "\n")
                handle.flush()
                print(f"{case['id']} [{repetition}/{args.repeat}]: {result['status']}", flush=True)
    summary = {"cases": len(cases), "requests": len(results),
               "failed": sum(result["status"] != "ok" for result in results),
               "raw_outputs": sum(result["raw_text"] is not None for result in results),
               "final_outputs": sum(result["final_text"] is not None for result in results),
               "raw_asr": aggregate_errors(results), "raw_terms": aggregate_terms(results, "raw_terms"),
               "final_terms": aggregate_terms(results, "final_terms"), "latency_seconds": latency_summary(results),
               "semantic_review": "pending", "term_scoring": "Exact forms and counts; positions require manual review"}
    (args.output / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    write_review(args.output, cases, results)
    print(f"Saved {len(results)} results ({summary['failed']} failed) to {args.output.resolve()}")
    return 1 if summary["failed"] else 0


if __name__ == "__main__":
    raise SystemExit(main())

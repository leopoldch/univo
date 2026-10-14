"""U-004 spike: compare embedding candidates on quality, speed, load and memory.

Quality is measured without any threshold: for each view of the critical
corpus, the AUC is the probability that a duplicate pair scores above a
distinct pair. The critical corpus is a regression set, not a tuning set, and
with about two hundred pairs AUC differences below a few points are noise.
Its negatives are mostly "same topic, other entity" traps, so the public
PAWS-X test sets (English and French, 2000 pairs each, licensed for any use)
complement it; PAWS-X pairs share most of their words and measure
sensitivity to word order and roles.

Every candidate runs in its own processes, once per thread count, so that
imports, loads and peak memory are not shared between candidates. One
process loads the model once and measures speed, quality and the peak
resident memory, read by the parent from the operating system; another
process only times a second load after releasing the first, so that the
peak never counts two models. Static models are also timed in Rust (rust/),
on the same texts.

Usage (dependencies in requirements.txt, outside the project):
    python evaluate.py --output results.json
"""

import argparse
import json
import os
import pathlib
import platform
import random
import resource
import statistics
import subprocess
import sys
import tempfile
import time
import unicodedata
from collections import defaultdict
from collections.abc import Callable, Sequence
from dataclasses import dataclass, field
from typing import Any

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
CORPUS = ROOT / "tests" / "fixtures" / "critical" / "cases.jsonl"
RUST_BENCH = HERE / "rust"
PAWS_X = ("google-research-datasets/paws-x", "4cd8187c40")
# Trigram Jaccard below which a duplicate is a rewording rather than a variant.
LOW_OVERLAP = 0.35
HARD_FAMILIES = (
    "semantic_competitor",
    "homonym",
    "negation",
    "version_environment",
    "dense_series",
)
THREAD_COUNTS = (1, 4)
WORKLOAD_SIZE = 2000

Encoder = Callable[[list[str]], Any]


@dataclass(frozen=True)
class Candidate:
    name: str
    family: str
    repo: str | None = None
    revision: str | None = None
    options: dict[str, Any] = field(default_factory=dict)
    prefix: str = ""


CANDIDATES = [
    Candidate("lexical char-3 Jaccard", "lexical"),
    Candidate("potion-base-2M", "static", "minishlab/potion-base-2M", "389b9f64be"),
    Candidate("potion-base-4M", "static", "minishlab/potion-base-4M", "9b3cff412d"),
    Candidate("potion-base-8M", "static", "minishlab/potion-base-8M", "bf8b056651"),
    Candidate("potion-retrieval-32M", "static", "minishlab/potion-retrieval-32M", "6fc8051fab"),
    Candidate(
        "potion-multilingual-128M", "static", "minishlab/potion-multilingual-128M", "73908c3438"
    ),
    Candidate(
        "potion-multilingual-128M int8",
        "static",
        "minishlab/potion-multilingual-128M",
        "73908c3438",
        {"quantize_to": "int8"},
    ),
    Candidate(
        "potion-multilingual-128M int8 128d",
        "static",
        "minishlab/potion-multilingual-128M",
        "73908c3438",
        {"quantize_to": "int8", "dimensionality": 128},
    ),
    Candidate(
        "all-MiniLM-L6-v2", "contextual", "sentence-transformers/all-MiniLM-L6-v2", "1110a243fd"
    ),
    Candidate("bge-small-en-v1.5", "contextual", "BAAI/bge-small-en-v1.5", "5c38ec7c40"),
    Candidate(
        "paraphrase-multilingual-MiniLM-L12-v2",
        "contextual",
        "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
        "e8f8c21122",
    ),
    Candidate(
        "multilingual-e5-small",
        "contextual",
        "intfloat/multilingual-e5-small",
        "614241f622",
        prefix="query: ",
    ),
]
# Static models measured in Rust: the backend Univo would ship.
RUST_CANDIDATES = ("potion-base-2M", "potion-base-8M", "potion-multilingual-128M")


@dataclass(frozen=True)
class Pair:
    case_id: str
    family: str
    lang: str
    view: str
    left: str
    right: str
    label: int


def load_pairs() -> list[Pair]:
    """Text pairs of the critical corpus whose label depends on wording."""
    pairs = []
    for line in CORPUS.read_text(encoding="utf-8").splitlines():
        case = json.loads(line)
        if case["label"] not in ("duplicate", "distinct") or case["constraint"]:
            continue
        label = int(case["label"] == "duplicate")
        left, right = case["left"], case["right"]
        views: list[tuple[str, Any, Any]] = []
        if case["profile"] == "strings":
            views.append(("short", left, right))
        else:
            views.append(("short", left.get("name"), right.get("name")))
            views.append(("description", left.get("description"), right.get("description")))
        for view, a, b in views:
            if isinstance(a, str) and isinstance(b, str) and a.strip() and b.strip():
                pairs.append(Pair(case["case_id"], case["family"], case["lang"], view, a, b, label))
    return pairs


def load_paws(language: str) -> list[Pair]:
    import pyarrow.parquet as pq
    from huggingface_hub import hf_hub_download

    repo, revision = PAWS_X
    path = hf_hub_download(
        repo, f"{language}/test-00000-of-00001.parquet", repo_type="dataset", revision=revision
    )
    return [
        Pair(
            f"paws-{language}-{row['id']}",
            "paws",
            language,
            "paws",
            row["sentence1"],
            row["sentence2"],
            int(row["label"]),
        )
        for row in pq.read_table(path).to_pylist()
        if row["sentence1"].strip() and row["sentence2"].strip()
    ]


def auc(scores: Sequence[float], labels: Sequence[int]) -> float | None:
    """Mann-Whitney AUC with ties counted as one half."""
    positives = [s for s, y in zip(scores, labels, strict=True) if y]
    negatives = [s for s, y in zip(scores, labels, strict=True) if not y]
    if not positives or not negatives:
        return None
    wins = 0.0
    for p in positives:
        for n in negatives:
            wins += 1.0 if p > n else 0.5 if p == n else 0.0
    return wins / (len(positives) * len(negatives))


def subset_auc(
    pairs: Sequence[Pair], scores: Sequence[float], selected: Sequence[bool]
) -> float | None:
    chosen = [(s, p.label) for s, p, keep in zip(scores, pairs, selected, strict=True) if keep]
    return auc([s for s, _ in chosen], [y for _, y in chosen])


def trigrams(text: str) -> set[str]:
    folded = f"  {unicodedata.normalize('NFC', text).casefold()}  "
    return {folded[i : i + 3] for i in range(len(folded) - 2)}


def lexical_similarity(left: str, right: str) -> float:
    a, b = trigrams(left), trigrams(right)
    return len(a & b) / len(a | b) if a | b else 0.0


def workload(count: int) -> tuple[list[str], list[str]]:
    """Names of about 20 characters and descriptions of about 30 words."""
    words = [
        *["online", "retail", "cloud", "services", "platform", "payments", "logistics", "software"],
        *["consulting", "energy", "Société", "française", "spécialisée", "distribution"],
        *["hypermarchés", "électronique", "européenne", "Zürich", "Straße", "café", "data"],
        *["analytics", "security"],
    ]
    rng = random.Random(7)
    names = [" ".join(rng.choices(words, k=2))[:20] for _ in range(count)]
    descriptions = [" ".join(rng.choices(words, k=30)) for _ in range(count)]
    return names, descriptions


def throughput(encode: Encoder, texts: list[str]) -> float:
    """Median texts per second over three runs after a warmup."""
    encode(texts[:64])
    runs = []
    for _ in range(3):
        start = time.perf_counter()
        encode(texts)
        runs.append(len(texts) / (time.perf_counter() - start))
    return statistics.median(runs)


def directory_size(path: pathlib.Path) -> int:
    return sum(file.stat().st_size for file in path.rglob("*") if file.is_file())


def snapshot(candidate: Candidate) -> pathlib.Path:
    from huggingface_hub import snapshot_download

    assert candidate.repo is not None
    return pathlib.Path(snapshot_download(candidate.repo, revision=candidate.revision))


def normalized(encode: Encoder) -> Encoder:
    import numpy as np

    def encode_normalized(texts: list[str]) -> Any:
        vectors = np.asarray(encode(texts), dtype=np.float32)
        norms = np.linalg.norm(vectors, axis=1, keepdims=True)
        return vectors / np.where(norms == 0, 1, norms)

    return encode_normalized


def load_model(candidate: Candidate, path: pathlib.Path, threads: int) -> tuple[Encoder, int]:
    """Loads a model and returns its encoder and dimension."""
    if candidate.family == "static":
        from model2vec import StaticModel

        model = StaticModel.from_pretrained(str(path), force_download=False, **candidate.options)

        def encode_static(texts: list[str]) -> Any:
            return model.encode(texts, use_multiprocessing=False)

        return encode_static, int(model.embedding.shape[1])

    import torch
    from sentence_transformers import SentenceTransformer

    torch.set_num_threads(threads)
    encoder = SentenceTransformer(str(path), device="cpu")

    def encode_contextual(texts: list[str]) -> Any:
        prefixed = [candidate.prefix + text for text in texts]
        return encoder.encode(prefixed, batch_size=64, normalize_embeddings=True)

    return encode_contextual, int(encoder.get_sentence_embedding_dimension() or 0)


def import_backend(candidate: Candidate) -> None:
    if candidate.family == "static":
        import model2vec  # noqa: F401
    elif candidate.family == "contextual":
        import sentence_transformers  # noqa: F401
        import torch  # noqa: F401


def similarities(encode: Encoder | None, pairs: Sequence[Pair]) -> list[float]:
    if encode is None:
        return [lexical_similarity(p.left, p.right) for p in pairs]
    import numpy as np

    left = encode([p.left for p in pairs])
    right = encode([p.right for p in pairs])
    return [float(x) for x in np.sum(left * right, axis=1)]


def quality(encode: Encoder | None) -> dict[str, Any]:
    pairs = load_pairs()
    scores = similarities(encode, pairs)
    lexical = [lexical_similarity(p.left, p.right) for p in pairs]
    hybrid = [(s + x) / 2 for s, x in zip(scores, lexical, strict=True)]
    low_overlap = [p.label == 1 and x < LOW_OVERLAP for p, x in zip(pairs, lexical, strict=True)]

    result: dict[str, Any] = {
        "auc_all": auc(scores, [p.label for p in pairs]),
        "auc_all_hybrid": auc(hybrid, [p.label for p in pairs]),
        "low_overlap_positives": sum(low_overlap),
        "auc_low_overlap": subset_auc(
            pairs, scores, [low or not p.label for p, low in zip(pairs, low_overlap, strict=True)]
        ),
    }
    for view in ("short", "description"):
        in_view = [p.view == view for p in pairs]
        result[f"auc_{view}"] = subset_auc(pairs, scores, in_view)
        result[f"auc_{view}_hybrid"] = subset_auc(pairs, hybrid, in_view)
    for lang in ("en", "fr"):
        result[f"auc_{lang}"] = subset_auc(pairs, scores, [p.lang == lang for p in pairs])

    positives = [s for s, p in zip(scores, pairs, strict=True) if p.label]
    median_positive = statistics.median(positives)
    hard: dict[str, list[float]] = defaultdict(list)
    for score, pair in zip(scores, pairs, strict=True):
        if pair.family in HARD_FAMILIES and not pair.label:
            hard[pair.family].append(score)
    result["hard_negatives_above_median_positive"] = {
        family: sum(s >= median_positive for s in values) / len(values)
        for family, values in sorted(hard.items())
    }

    for language in ("en", "fr"):
        paws = load_paws(language)
        result[f"auc_paws_{language}"] = auc(similarities(encode, paws), [p.label for p in paws])
    return result


def peak_rss_bytes() -> int:
    """Peak resident memory of this process; ru_maxrss is in KiB on Linux."""
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return peak if sys.platform == "darwin" else peak * 1024


def worker(name: str, threads: int, with_quality: bool) -> dict[str, Any]:
    """Loads one candidate once in a fresh process and measures it."""
    candidate = next(c for c in CANDIDATES if c.name == name)
    if candidate.family == "lexical":
        return {"name": name, "threads": threads, **(quality(None) if with_quality else {})}

    path = snapshot(candidate)
    start = time.perf_counter()
    import_backend(candidate)
    import_s = time.perf_counter() - start
    rss_after_import = peak_rss_bytes()

    start = time.perf_counter()
    encode, dimension = load_model(candidate, path, threads)
    first_load_s = time.perf_counter() - start

    encode = normalized(encode)
    names, descriptions = workload(WORKLOAD_SIZE)
    result: dict[str, Any] = {
        "name": name,
        "threads": threads,
        "dimension": dimension,
        "download_mb": directory_size(path) / 1e6,
        "import_s": import_s,
        "first_load_s": first_load_s,
        "names_per_s": throughput(encode, names),
        "descriptions_per_s": throughput(encode, descriptions),
        "rss_after_import_mb": rss_after_import / 1e6,
    }
    if with_quality:
        result.update(quality(encode))
    return result


def warm_worker(name: str, threads: int) -> dict[str, Any]:
    """Times a second load, after the first model is released."""
    import gc

    candidate = next(c for c in CANDIDATES if c.name == name)
    path = snapshot(candidate)
    import_backend(candidate)
    model = load_model(candidate, path, threads)
    del model
    gc.collect()
    start = time.perf_counter()
    load_model(candidate, path, threads)
    return {"warm_load_s": time.perf_counter() - start}


def run_child(arguments: list[str], threads: int) -> tuple[str, int]:
    """Runs a child process with `threads` threads; returns stdout and peak RSS."""
    environment = {
        **os.environ,
        "TOKENIZERS_PARALLELISM": "true" if threads > 1 else "false",
        "RAYON_NUM_THREADS": str(threads),
        "OMP_NUM_THREADS": str(threads),
    }
    with tempfile.TemporaryFile() as output:
        child = subprocess.Popen(arguments, stdout=output, env=environment)
        _, status, usage = os.wait4(child.pid, 0)
        if status != 0:
            raise RuntimeError(f"{arguments} failed with status {status}")
        output.seek(0)
        stdout = output.read().decode()
    peak = usage.ru_maxrss if sys.platform == "darwin" else usage.ru_maxrss * 1024
    return stdout, peak


def last_json(stdout: str) -> dict[str, Any]:
    result: dict[str, Any] = json.loads(stdout.strip().splitlines()[-1])
    return result


def measure_python(candidate: Candidate, threads: int) -> dict[str, Any]:
    arguments = [sys.executable, __file__, "--worker", candidate.name, "--threads", str(threads)]
    measured = [*arguments, "--quality"] if threads == 1 else arguments
    stdout, peak = run_child(measured, threads)
    result = last_json(stdout)
    result["peak_rss_mb"] = peak / 1e6
    if candidate.family != "lexical":
        stdout, _ = run_child([*arguments, "--warm"], threads)
        result.update(last_json(stdout))
    return result


def build_rust() -> pathlib.Path:
    subprocess.run(
        ["cargo", "build", "--release", "--locked", "--quiet"], cwd=RUST_BENCH, check=True
    )
    return RUST_BENCH / "target" / "release" / "model-selection-bench"


def measure_rust(
    binary: pathlib.Path, candidate: Candidate, threads: int, texts: pathlib.Path
) -> dict[str, Any]:
    arguments = [str(binary), str(snapshot(candidate)), str(texts / "names.txt")]
    arguments.append(str(texts / "descriptions.txt"))
    stdout, peak = run_child(arguments, threads)
    result = last_json(stdout)
    stdout, _ = run_child([*arguments, "--warm"], threads)
    result.update(last_json(stdout))
    return {"name": candidate.name, "threads": threads, **result, "peak_rss_mb": peak / 1e6}


def summary(result: dict[str, Any]) -> str:
    def value(key: str, pattern: str) -> str:
        return format(result[key], pattern) if result.get(key) is not None else "-"

    return (
        f"{result['name']:40} t={result['threads']} auc {value('auc_all', '.3f')} "
        f"low {value('auc_low_overlap', '.3f')} fr {value('auc_fr', '.3f')} "
        f"paws {value('auc_paws_en', '.3f')}/{value('auc_paws_fr', '.3f')} | "
        f"{value('names_per_s', ',.0f')} names/s {value('descriptions_per_s', ',.0f')} desc/s | "
        f"load {value('first_load_s', '.2f')}s warm {value('warm_load_s', '.2f')}s "
        f"peak {value('peak_rss_mb', '.0f')} MB"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, default=HERE / "results.json")
    parser.add_argument("--only", nargs="*", help="evaluate only these candidate names")
    parser.add_argument("--worker", help=argparse.SUPPRESS)
    parser.add_argument("--threads", type=int, default=1, help=argparse.SUPPRESS)
    parser.add_argument("--quality", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--warm", action="store_true", help=argparse.SUPPRESS)
    arguments = parser.parse_args()

    if arguments.worker and arguments.warm:
        print(json.dumps(warm_worker(arguments.worker, arguments.threads)))
        return
    if arguments.worker:
        print(json.dumps(worker(arguments.worker, arguments.threads, arguments.quality)))
        return

    selected = [c for c in CANDIDATES if not arguments.only or c.name in arguments.only]
    python_results = []
    for candidate in selected:
        for threads in THREAD_COUNTS if candidate.family != "lexical" else (1,):
            result = measure_python(candidate, threads)
            python_results.append(result)
            print(summary(result), flush=True)

    rust_results = []
    binary = build_rust()
    with tempfile.TemporaryDirectory() as directory:
        texts = pathlib.Path(directory)
        names, descriptions = workload(WORKLOAD_SIZE)
        (texts / "names.txt").write_text("\n".join(names) + "\n", encoding="utf-8")
        (texts / "descriptions.txt").write_text("\n".join(descriptions) + "\n", encoding="utf-8")
        for candidate in selected:
            if candidate.name not in RUST_CANDIDATES:
                continue
            for threads in THREAD_COUNTS:
                result = measure_rust(binary, candidate, threads, texts)
                rust_results.append(result)
                print("rust " + summary(result), flush=True)

    machine = {
        "platform": platform.platform(),
        "processor": platform.processor() or platform.machine(),
        "cpu_count": os.cpu_count(),
        "python": platform.python_version(),
        "workload": {"names": WORKLOAD_SIZE, "descriptions": WORKLOAD_SIZE},
    }
    report = {"machine": machine, "python": python_results, "rust": rust_results}
    arguments.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()

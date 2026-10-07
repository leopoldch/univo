"""U-004 spike: compare embedding candidates on quality, speed and size.

Quality is measured without any threshold: for each view of the critical
corpus, the AUC is the probability that a duplicate pair scores above a
distinct pair. Speed is the single-thread encoding throughput on the
standard workload of specs/07-performance.md. Size is the memory taken by
the weights and the files downloaded. Every model is pinned to a revision.

The critical corpus is a regression set, not a tuning set: nothing here
chooses a threshold. With about two hundred pairs, AUC differences below a
few points are within noise. Its negatives are mostly "same topic, other
entity" traps, so the public PAWS-X test sets (English and French, 2000
pairs each, licensed for any use) complement it; PAWS-X pairs share most of
their words, so it measures sensitivity to word order and roles.

Usage (dependencies in requirements.txt, outside the project):
    python evaluate.py --output results.json
"""

import argparse
import json
import os
import pathlib
import platform
import random
import statistics
import time
import unicodedata
from collections import defaultdict
from collections.abc import Callable, Sequence
from dataclasses import dataclass, field
from typing import Any

os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
os.environ.setdefault("RAYON_NUM_THREADS", "1")
os.environ.setdefault("OMP_NUM_THREADS", "1")

import numpy as np
import psutil
from huggingface_hub import snapshot_download

ROOT = pathlib.Path(__file__).resolve().parents[3]
CORPUS = ROOT / "tests" / "fixtures" / "critical" / "cases.jsonl"
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
    """Text pairs whose label depends on wording, not on structure."""
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
    table = pq.read_table(path).to_pylist()
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
        for row in table
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


def trigrams(text: str) -> set[str]:
    folded = f"  {unicodedata.normalize('NFC', text).casefold()}  "
    return {folded[i : i + 3] for i in range(len(folded) - 2)}


def lexical_similarity(left: str, right: str) -> float:
    a, b = trigrams(left), trigrams(right)
    return len(a & b) / len(a | b) if a | b else 0.0


def workload(count: int) -> tuple[list[str], list[str]]:
    """Names of about 20 characters and descriptions of about 30 tokens."""
    words = [
        "online",
        "retail",
        "cloud",
        "services",
        "platform",
        "payments",
        "logistics",
        "software",
        "consulting",
        "energy",
        "Société",
        "française",
        "spécialisée",
        "distribution",
        "hypermarchés",
        "électronique",
        "européenne",
        "Zürich",
        "Straße",
        "café",
        "data",
        "analytics",
        "security",
    ]
    rng = random.Random(7)
    names = [" ".join(rng.choices(words, k=2))[:20] for _ in range(count)]
    descriptions = [" ".join(rng.choices(words, k=30)) for _ in range(count)]
    return names, descriptions


def measure_speed(encode: Callable[[list[str]], Any], texts: list[str]) -> float:
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


def build(candidate: Candidate) -> tuple[Callable[[list[str]], Any] | None, dict[str, Any]]:
    """Returns an encoder producing L2-normalized rows, and size facts."""
    if candidate.family == "lexical":
        return None, {"weights_mb": 0.0, "download_mb": 0.0, "load_s": 0.0}
    path = pathlib.Path(snapshot_download(candidate.repo, revision=candidate.revision))
    download = directory_size(path)
    process = psutil.Process()
    rss_before = process.memory_info().rss
    start = time.perf_counter()
    if candidate.family == "static":
        from model2vec import StaticModel

        model = StaticModel.from_pretrained(str(path), force_download=False, **candidate.options)

        def encode(texts: list[str]) -> Any:
            return model.encode(texts, use_multiprocessing=False)

        mapping = getattr(model, "token_mapping", None)
        weights = model.embedding.nbytes + (mapping.nbytes if mapping is not None else 0)
        dimension = model.embedding.shape[1]
    else:
        import torch
        from sentence_transformers import SentenceTransformer

        torch.set_num_threads(1)
        model = SentenceTransformer(str(path), device="cpu")

        def encode(texts: list[str]) -> Any:
            prefixed = [candidate.prefix + text for text in texts]
            return model.encode(prefixed, batch_size=64, normalize_embeddings=True)

        weights = sum(p.numel() * p.element_size() for p in model.parameters())
        dimension = model.get_sentence_embedding_dimension()
    load = time.perf_counter() - start
    encode(["warmup"])
    facts = {
        "weights_mb": weights / 1e6,
        "download_mb": download / 1e6,
        "load_s": load,
        "rss_mb": (process.memory_info().rss - rss_before) / 1e6,
        "dimension": int(dimension),
    }

    def normalized(texts: list[str]) -> Any:
        vectors = np.asarray(encode(texts), dtype=np.float32)
        norms = np.linalg.norm(vectors, axis=1, keepdims=True)
        return vectors / np.where(norms == 0, 1, norms)

    return normalized, facts


def similarities(encode: Callable[[list[str]], Any] | None, pairs: list[Pair]) -> list[float]:
    if encode is None:
        return [lexical_similarity(p.left, p.right) for p in pairs]
    left = encode([p.left for p in pairs])
    right = encode([p.right for p in pairs])
    return [float(x) for x in np.sum(left * right, axis=1)]


def evaluate(
    candidate: Candidate, pairs: list[Pair], paws: dict[str, list[Pair]]
) -> dict[str, Any]:
    encode, facts = build(candidate)
    scores = similarities(encode, pairs)
    lexical = [lexical_similarity(p.left, p.right) for p in pairs]

    def subset_auc(select: Callable[[Pair], bool], values: Sequence[float]) -> float | None:
        chosen = [(v, p.label) for v, p in zip(values, pairs, strict=True) if select(p)]
        return auc([v for v, _ in chosen], [y for _, y in chosen])

    hybrid = [(s + x) / 2 for s, x in zip(scores, lexical, strict=True)]
    result: dict[str, Any] = {"name": candidate.name, "family": candidate.family, **facts}
    for view in ("short", "description"):
        result[f"auc_{view}"] = subset_auc(lambda p, v=view: p.view == v, scores)
        result[f"auc_{view}_hybrid"] = subset_auc(lambda p, v=view: p.view == v, hybrid)
    for lang in ("en", "fr"):
        result[f"auc_{lang}"] = subset_auc(lambda p, lang=lang: p.lang == lang, scores)
    result["auc_all"] = subset_auc(lambda p: True, scores)
    # What the model adds: duplicates whose wording differs (low lexical
    # overlap) against every distinct pair.
    low_overlap = {
        id(p) for p, x in zip(pairs, lexical, strict=True) if p.label and x < LOW_OVERLAP
    }
    result["low_overlap_positives"] = len(low_overlap)
    result["auc_low_overlap"] = subset_auc(lambda p: not p.label or id(p) in low_overlap, scores)
    result["auc_all_hybrid"] = subset_auc(lambda p: True, hybrid)

    by_family: dict[str, list[float]] = defaultdict(list)
    for value, pair in zip(scores, pairs, strict=True):
        if pair.family in HARD_FAMILIES and not pair.label:
            by_family[pair.family].append(value)
    positives = [v for v, p in zip(scores, pairs, strict=True) if p.label]
    median_positive = statistics.median(positives)
    result["hard_negatives_above_median_positive"] = {
        family: sum(v >= median_positive for v in values) / len(values)
        for family, values in sorted(by_family.items())
    }

    for language, paws_pairs in paws.items():
        paws_scores = similarities(encode, paws_pairs)
        result[f"auc_paws_{language}"] = auc(paws_scores, [p.label for p in paws_pairs])

    if encode is not None:
        names, descriptions = workload(2000)
        result["names_per_s"] = measure_speed(encode, names)
        result["descriptions_per_s"] = measure_speed(encode, descriptions)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, default=pathlib.Path("results.json"))
    parser.add_argument("--only", nargs="*", help="evaluate only these candidate names")
    arguments = parser.parse_args()

    pairs = load_pairs()
    counts = {
        view: (
            sum(p.view == view and p.label for p in pairs),
            sum(p.view == view and not p.label for p in pairs),
        )
        for view in ("short", "description")
    }
    print(f"pairs: {len(pairs)} (positives, negatives) per view: {counts}")
    paws = {language: load_paws(language) for language in ("en", "fr")}
    print({language: len(rows) for language, rows in paws.items()}, "PAWS-X pairs")
    results = []
    for candidate in CANDIDATES:
        if arguments.only and candidate.name not in arguments.only:
            continue
        result = evaluate(candidate, pairs, paws)
        results.append(result)
        print(
            f"{result['name']:42} auc short {result['auc_short']:.3f} "
            f"desc {result['auc_description']:.3f} en {result['auc_en']:.3f} "
            f"fr {result['auc_fr']:.3f} hybrid {result['auc_all_hybrid']:.3f} "
            f"low-overlap {result['auc_low_overlap']:.3f} ({result['low_overlap_positives']}) "
            f"paws en {result['auc_paws_en']:.3f} fr {result['auc_paws_fr']:.3f} | "
            f"{result.get('names_per_s', 0):,.0f} names/s "
            f"{result.get('descriptions_per_s', 0):,.0f} desc/s | "
            f"weights {result['weights_mb']:.1f} MB download {result['download_mb']:.1f} MB",
            flush=True,
        )
    machine = {
        "platform": platform.platform(),
        "processor": platform.processor(),
        "python": platform.python_version(),
        "threads": 1,
    }
    arguments.output.write_text(
        json.dumps(
            {
                "machine": machine,
                "pairs": counts,
                "paws_pairs": {language: len(rows) for language, rows in paws.items()},
                "results": results,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()

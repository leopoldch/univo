"""Native encoder: loading and encoding cost of a model, in one fresh process.

Run one process per measurement, alternating wheels in A B B A order:

    RAYON_NUM_THREADS=1 python -I python_model.py MODEL_DIR TEXTS.json

`TEXTS.json` is a JSON list of strings. Prints one JSON line: the time and
resident memory added by loading the model (files in the OS cache), the
best of three encodings of every text per text, and the best of 200
single-text encodings.
"""

import json
import os
import resource
import subprocess
import sys
import time

from univo import _univo


def rss_mib() -> float:
    return int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(os.getpid())])) / 1024


def main() -> None:
    model_dir, texts_file = sys.argv[1], sys.argv[2]
    with open(texts_file, encoding="utf-8") as file:
        texts = json.load(file)
    before = rss_mib()
    start = time.perf_counter()
    model = _univo.load_model(model_dir, transport=None, offline=True)
    load_ms = 1e3 * (time.perf_counter() - start)
    model_rss = rss_mib() - before
    model.encode(texts[:2000])
    batch = min(timed(model, texts) for _ in range(3))
    single = min(timed(model, texts[:1]) for _ in range(200))
    print(
        json.dumps(
            {
                "load_ms": round(load_ms, 1),
                "model_rss_mib": round(model_rss, 1),
                "encode_us_per_text": round(1e6 * batch / len(texts), 3),
                "single_text_us": round(1e6 * single, 1),
                "peak_mib": round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 2**20, 1),
            }
        )
    )


def timed(model: _univo.Model, texts: list[str]) -> float:
    start = time.perf_counter()
    model.encode(texts)
    return time.perf_counter() - start


if __name__ == "__main__":
    main()

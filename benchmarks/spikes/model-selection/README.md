# U-004 spike: default embedding model

Comparison of three architectural families from [specs/04-models-and-cache.md §11](../../../specs/04-models-and-cache.md): exact and lexical comparison without a model, static embeddings (Model2Vec) and small contextual encoders. The proposed decision is in [ADR 0007](../../../docs/adr/0007-default-embedding-model.md).

## Reproduce

```bash
python -m venv /tmp/model-selection && /tmp/model-selection/bin/pip install -r requirements.txt
/tmp/model-selection/bin/python evaluate.py --output results.json   # also builds rust/ with cargo
```

- Every model is pinned to a Hugging Face revision in `evaluate.py`.
- The Rust benchmark in [rust/](rust) has its own locked `Cargo.lock` and is outside Univo's workspace.
- Raw numbers are in [results.json](results.json).

## Method

- **Isolation.**
  - Each candidate runs in a fresh process, once with one thread and once with four.
  - Inside that process, the backend import, the first load and a second ("warm") load are timed separately.
  - The parent reads each process's peak resident memory from the operating system (`wait4`).
  - The operating system's file cache was not purged (that requires administrator rights), so "first load" means first load in a fresh process, with model files possibly already in the file cache.
- **Quality without thresholds.** AUC: probability that a duplicate pair scores above a distinct pair; 0.5 is chance.
  - Critical corpus: 160 short-text pairs (strings and names) and 74 description pairs. Cases decided by a veto, a partition or invalid input are excluded, since text does not decide them.
  - **Low-overlap duplicates**: the 32 duplicates whose character-trigram Jaccard is below 0.35, against every distinct pair. This isolates what a model adds over lexical comparison: rewordings, other scripts and acronyms.
  - [PAWS-X](https://huggingface.co/datasets/google-research-datasets/paws-x) test sets, English and French, 2000 pairs each, licensed for any use. Its pairs share most of their words, so it measures sensitivity to word order and roles.
- **Speed.** Standard workload of [specs/07-performance.md](../../../specs/07-performance.md): 2000 names of about 20 characters and 2000 descriptions of about 30 words, the same texts for Python and Rust. The figure is the median of three runs after a warmup, with 1 and 4 threads.
  - Python: `model2vec` (numpy) and `sentence-transformers` (PyTorch).
  - Rust: `model2vec-rs` 0.3.0 without network or C dependency (`local-only`, `fancy-regex`), which is the path Univo would ship.
- **Memory.** Peak resident memory of the measuring process, which loads the model once; the warm load is timed in a separate process that releases the first model before loading it again, so no peak counts two models. Python peaks are taken from the four-thread process, which only encodes; the one-thread process also evaluates quality and holds the PAWS-X data. In Python, the interpreter with the backend imported already takes about 69 MB (static) or 414 MB (PyTorch).

The critical corpus is a regression set: nothing here tunes a threshold, and with about two hundred pairs, differences below about 0.03 AUC are within noise.

Machine: Apple M2 (8 cores), 16 GiB, macOS 27.0.1, Python 3.12.14, Rust 1.99.0, 2026-10-07.

## Quality

| Candidate | Family | Corpus | Low overlap | French | PAWS-X EN / FR |
| --- | --- | --- | --- | --- | --- |
| char-3 Jaccard | lexical | **0.634** | 0.312 | 0.591 | 0.600 / 0.565 |
| potion-base-2M | static | 0.594 | 0.368 | 0.561 | 0.475 / 0.528 |
| potion-base-4M | static | 0.595 | 0.364 | 0.565 | 0.475 / 0.527 |
| potion-base-8M | static | 0.585 | 0.356 | 0.557 | 0.477 / 0.527 |
| potion-retrieval-32M | static | 0.571 | 0.330 | 0.530 | 0.476 / 0.531 |
| potion-multilingual-128M | static | 0.601 | 0.363 | 0.567 | 0.481 / 0.517 |
| … int8 | static | 0.595 | 0.364 | 0.564 | 0.481 / 0.518 |
| … int8, 128 dimensions | static | 0.592 | 0.352 | 0.554 | 0.479 / 0.517 |
| all-MiniLM-L6-v2 | contextual | 0.592 | 0.323 | 0.593 | **0.622** / 0.574 |
| bge-small-en-v1.5 | contextual | 0.605 | 0.373 | 0.592 | 0.599 / 0.556 |
| paraphrase-multilingual-MiniLM-L12-v2 | contextual | 0.588 | 0.367 | 0.577 | 0.600 / **0.584** |
| multilingual-e5-small | contextual | 0.631 | **0.432** | **0.606** | 0.575 / 0.550 |

Published averages from the model cards, measured by the model authors on public benchmarks rather than on deduplication:

| Model | Avg (All) | Avg (MTEB) | PairClass |
| --- | --- | --- | --- |
| all-MiniLM-L6-v2 | 55.80 | 55.93 | 82.37 |
| potion-base-8M | 51.32 | 51.08 | 76.62 |
| potion-base-2M | 47.55 | 47.49 | 73.72 |

## Speed, load and memory

Descriptions per second, the bottleneck of the workload; names are 4 to 10 times faster.

| Candidate | Python, 1 thread | Python, 4 threads | Import | First load | Warm load | Peak memory | Weights |
| --- | --- | --- | --- | --- | --- | --- | --- |
| potion-base-2M | 17 779 | 41 329 | 0.10 s | 0.07 s | 0.03 s | 110 MB | 7.6 MB |
| potion-base-4M | 17 863 | 39 171 | 0.06 s | 0.06 s | 0.03 s | 118 MB | 15.1 MB |
| potion-base-8M | 17 279 | 39 366 | 0.06 s | 0.06 s | 0.03 s | 144 MB | 30.2 MB |
| potion-retrieval-32M | 16 797 | 35 098 | 0.06 s | 0.38 s | 0.09 s | 357 MB | 129 MB |
| potion-multilingual-128M | 10 962 | 30 915 | 0.06 s | 2.29 s | 1.42 s | 1 860 MB | 512 MB |
| … int8 | 10 937 | 29 596 | 0.05 s | 2.67 s | 2.84 s | 2 250 MB | 128 MB |
| … int8, 128 dimensions | 11 386 | 32 082 | 0.05 s | 2.57 s | 2.74 s | 2 250 MB | 64 MB |
| all-MiniLM-L6-v2 | 348 | 539 | 4.31 s | 0.12 s | 0.04 s | 660 MB | 91 MB |
| bge-small-en-v1.5 | 188 | 315 | 2.71 s | 0.11 s | 0.04 s | 702 MB | 133 MB |
| paraphrase-multilingual-MiniLM-L12-v2 | 185 | 272 | 2.69 s | 0.78 s | 0.65 s | 1 400 MB | 471 MB |
| multilingual-e5-small | 170 | 259 | 2.72 s | 1.06 s | 0.98 s | 1 227 MB | 471 MB |

The quantized variants quantize at load from the 512 MB original, hence their load time and peak; a pre-quantized artifact would not.

Rust, `model2vec-rs`, same texts:

| Model | Names/s, 1 / 4 threads | Descriptions/s, 1 / 4 threads | First load | Warm load | Peak memory | Dimension |
| --- | --- | --- | --- | --- | --- | --- |
| potion-base-2M | 160 126 / 393 146 | 25 276 / 63 157 | 0.024 s | 0.011 s | 43 MB | 64 |
| potion-base-8M | 160 135 / 318 630 | 21 563 / 46 541 | 0.032 s | 0.015 s | 88 MB | 256 |
| potion-multilingual-128M | 168 944 / 276 124 | 14 747 / 38 365 | 1.008 s | 0.608 s | 1 587 MB | 256 |

## Findings

1. **No embedding model alone beats lexical comparison on the critical corpus** (0.634). Its negatives are mostly "same topic, other entity" traps, which embeddings fall into; only multilingual-e5-small comes close (0.631). Quality has to come from hybrid scoring with structured evidence and vetoes (U-008, U-010), with embeddings as one source of evidence.
2. **What models add is small.** On low-overlap duplicates, static models reach 0.35 to 0.37 against 0.31 for lexical; the best contextual model reaches 0.43. Every candidate stays below 0.5 there: these duplicates (other scripts, acronyms, rewordings) usually score below the traps.
3. **Static models are 30 to 150 times faster than contextual encoders on descriptions** (single thread, from the slowest static model in Python against the fastest contextual encoder, up to the fastest static model in Rust against the slowest encoder).
   - Descriptions per second, one thread: 14 747 to 25 276 in Rust and 10 937 to 17 863 in Python for static models, against 170 to 348 for contextual encoders.
   - Four threads: 38 365 to 63 157 in Rust for static models, against 259 to 539 for contextual encoders.
   - With a description per record, contextual encoders stay below the budget of 2000 objects per second at one and at four threads.
4. **Static models ignore word order** (PAWS-X at chance level), as an average of token vectors must. Contextual encoders reach 0.55 to 0.62.
5. **French**: no static model is measurably better than another on French pairs (0.53 to 0.57); the multilingual static model does not beat the English ones on this corpus. Its value would be cross-lingual pairs, too few here (3) to measure.
6. **potion-base-2M and potion-base-8M are indistinguishable on this corpus**, so it gives 8M no advantage. Published averages favour 8M (PairClass 76.62 against 73.72), but they are not deduplication measures. 2M has four times smaller vectors, half the peak memory in Rust (43 MB against 88 MB) and 17 % faster descriptions.
7. **Size levers work**: int8 and 128 dimensions cut the multilingual model from 512 MB to 64 MB for about 0.01 AUC. Vocabulary quantization was not measured: its k-means took more than 50 minutes on one core and was stopped; it belongs to building a model artifact, not to loading one.

## Limits

- A small, adversarial corpus decides little between close candidates. No dataset of entity duplicates with verified licenses exists yet in the repository; the quality benchmark of U-012 must confirm or revise the choice.
- Model files may have been in the operating system's file cache: first loads measure a fresh process, not a cold disk.
- Results come from one machine; other platforms are not measured.

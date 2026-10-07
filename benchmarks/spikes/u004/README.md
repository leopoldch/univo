# U-004 spike: default embedding model

Comparison of three architectural families from [specs/04-models-and-cache.md §11](../../../specs/04-models-and-cache.md): exact and lexical comparison without a model, static embeddings (Model2Vec) and small contextual encoders. The decision is recorded in [ADR 0007](../../../docs/adr/0007-default-embedding-model.md).

## Reproduce

```bash
python -m venv /tmp/u004 && /tmp/u004/bin/pip install -r requirements.txt
/tmp/u004/bin/python evaluate.py --output results.json
```

Every model is pinned to a Hugging Face revision in `evaluate.py`. Raw numbers are in [results.json](results.json).

## Method

- **Quality without thresholds.** AUC: probability that a duplicate pair scores above a distinct pair; 0.5 is chance.
  - Critical corpus: 160 short-text pairs (strings and names) and 74 description pairs. Cases decided by a veto, a partition or invalid input are excluded, since text does not decide them.
  - **Low-overlap duplicates**: the 32 duplicates whose character-trigram Jaccard is below 0.35, against every distinct pair. It isolates what a model adds over lexical comparison: rewordings, other scripts, acronyms.
  - [PAWS-X](https://huggingface.co/datasets/google-research-datasets/paws-x) test sets, English and French, 2000 pairs each, licensed for any use. Its pairs share most of their words, so it measures sensitivity to word order and roles.
- **Speed.** Single thread, standard workload of [specs/07-performance.md](../../../specs/07-performance.md): 2000 names of about 20 characters and 2000 descriptions of about 30 words. Python numbers come from `model2vec` (numpy) and `sentence-transformers` (PyTorch); static models were also measured through `model2vec-rs` in Rust, the path Univo would use.
- **Size.** Weights in memory; downloads in the table count only the files a backend needs.

The critical corpus is a regression set: nothing here tunes a threshold, and with about two hundred pairs, differences below about 0.03 AUC are within noise.

Machine: Apple M2, 16 GiB, macOS 27.0.1, Python 3.12, one thread, 2026-10-07.

## Results

| Candidate | Family | AUC corpus | AUC low overlap | AUC French | PAWS-X EN / FR | Descriptions/s | Load | Weights |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| char-3 Jaccard | lexical | **0.634** | 0.312 | 0.591 | 0.600 / 0.565 | — | — | 0 |
| potion-base-2M | static | 0.594 | 0.368 | 0.561 | 0.475 / 0.528 | 18 722 | 0.07 s | 7.6 MB |
| potion-base-4M | static | 0.595 | 0.364 | 0.565 | 0.475 / 0.527 | 18 573 | 0.06 s | 15.1 MB |
| potion-base-8M | static | 0.585 | 0.356 | 0.557 | 0.477 / 0.527 | 18 119 | 0.03 s | 30.2 MB |
| potion-retrieval-32M | static | 0.571 | 0.330 | 0.530 | 0.476 / 0.531 | 17 431 | 0.34 s | 129 MB |
| potion-multilingual-128M | static | 0.601 | 0.363 | 0.567 | 0.481 / 0.517 | 11 529 | 0.97 s | 512 MB |
| … int8 | static | 0.595 | 0.364 | 0.564 | 0.481 / 0.518 | 11 175 | — | 128 MB |
| … int8, 128 dimensions | static | 0.592 | 0.352 | 0.554 | 0.479 / 0.517 | 11 508 | — | 64 MB |
| all-MiniLM-L6-v2 | contextual | 0.592 | 0.323 | 0.593 | **0.622** / 0.574 | 393 | 3.75 s | 91 MB |
| bge-small-en-v1.5 | contextual | 0.605 | 0.373 | 0.592 | 0.599 / 0.556 | 199 | 0.08 s | 133 MB |
| paraphrase-multilingual-MiniLM-L12-v2 | contextual | 0.588 | 0.367 | 0.577 | 0.600 / **0.584** | 181 | 0.68 s | 471 MB |
| multilingual-e5-small | contextual | 0.631 | **0.432** | **0.606** | 0.575 / 0.550 | 155 | 1.02 s | 471 MB |

Load times of quantized variants include quantization at load and are not representative of a pre-quantized artifact.

Static models through `model2vec-rs` 0.3.0 in Rust, without network or C dependency (`local-only`, `fancy-regex`), release build, one thread:

| Model | Names/s | Descriptions/s | Load | Snapshot on disk | Tokenizer |
| --- | --- | --- | --- | --- | --- |
| potion-base-8M | 153 598 | 20 014 | 38 ms | 59 MB | 0.7 MB |
| potion-multilingual-128M | 164 074 | 13 888 | 1.27 s | 1.0 GB | 18.6 MB |

Published MTEB averages from the model cards, not measured here: potion-base-8M 51.32, potion-base-2M 47.55, against 55.8 for all-MiniLM-L6-v2 (92 % and 85 % of it).

## Findings

1. **No embedding model alone beats lexical comparison on the critical corpus** (0.634). Its negatives are mostly "same topic, other entity" traps, which embeddings fall into; only multilingual-e5-small comes close (0.631). Quality has to come from hybrid scoring with structured evidence and vetoes (U-008, U-010), with embeddings as one source of evidence.
2. **What models add is small.** On low-overlap duplicates, static models reach 0.35 to 0.37 against 0.31 for lexical; the best contextual model reaches 0.43. Every candidate stays below 0.5 there: these duplicates (other scripts, acronyms, rewordings) usually score below the traps.
3. **Static models are 50 to 120 times faster** than contextual encoders (11 000 to 20 000 descriptions per second against 155 to 393). Contextual encoders miss the budget of 2000 objects per second on one thread, and on four.
4. **Static models ignore word order** (PAWS-X at chance level), as an average of token vectors must. Contextual encoders reach 0.55 to 0.62.
5. **French**: no static model is measurably better than another on French pairs (0.53 to 0.57); the multilingual static model does not beat the English ones on this corpus. Its value would be cross-lingual pairs, too few here (3) to measure.
6. **Size levers work**: int8 and 128 dimensions cut the multilingual model from 512 MB to 64 MB for about 0.01 AUC. Vocabulary quantization was not measured: its k-means took more than 50 minutes on one core and was stopped; it belongs to building a model artifact, not to loading one.

## Limits

- A small, adversarial corpus decides little between close candidates; published MTEB averages are used to separate potion-base-2M and potion-base-8M.
- No dataset of entity duplicates with verified licenses exists yet in the repository; the quality benchmark of U-012 must confirm or revise the choice.
- Memory figures measured inside one Python process are cumulative and are not reported.

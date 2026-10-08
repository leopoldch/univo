# 0007. Default embedding model and model selection

- Status: Proposed, awaiting the maintainer's acceptance
- Date: 2026-10-07
- Roadmap: U-004

## Context

The maintainer requires a default model that is powerful, fast and small, and the ability to switch model with a string: a Hugging Face repository, a local directory or another source. The specification orders the criteria as minimal quality on the target case, then memory and size, latency, dependencies and maintenance ([04-models-and-cache.md §11](../../specs/04-models-and-cache.md)), and caps the default model download at 100 MiB ([07-performance.md](../../specs/07-performance.md)). The [U-004 spike](../../benchmarks/spikes/u004/README.md) measured twelve candidates from three families, each in its own process, at one and four threads, in Python and, for static models, in Rust.

## Decision

- **Backend**: static embeddings in the Model2Vec format (`config.json`, `model.safetensors`, `tokenizer.json`), computed in Rust. It is the only backend shipped in the wheels.
- **Default model, proposed**: `minishlab/potion-base-8M` at revision `bf8b056651a2c21b8d2565580b8569da283cab23`; MIT; 256 dimensions; 30.2 MB of weights and a 0.7 MB tokenizer to download; English.
  - The critical corpus does not separate it from `potion-base-2M`.
  - It is proposed because published benchmarks rank it higher (PairClass 76.62 against 73.72; Avg (MTEB) 51.08 against 47.49). These are public averages, not deduplication measures, so the choice remains open (see Alternatives).
- **Language claim**: the default is announced as an English model. French text is handled through shared subwords and was measured on the critical corpus at the level of the multilingual static model, but no French or cross-lingual quality is claimed until the quality benchmark (U-012) validates it.
- **Selection by string**, as specified in [02-python-api.md §5.5](../../specs/02-python-api.md):
  - `"default"`;
  - a Hugging Face repository `org/name`, optionally `@revision`;
  - a local directory (absolute, `./`, `../`, `~` or `file://`);
  - a Hugging Face compatible mirror through `hub_url` or `HF_ENDPOINT`.

  Any repository in the Model2Vec format works: for example `minishlab/potion-base-2M` for the smallest footprint, or `minishlab/potion-multilingual-128M` for 101 languages.
- **Other Hugging Face models**: a repository in another format, such as a sentence-transformers encoder, is refused with an error that explains how to convert it. Model2Vec distillation turns such an encoder into a static model in minutes; a helper may be offered as an optional extra that the default installation does not pull in. A contextual backend is not part of the defaults (see Alternatives).

## Alternatives

- **potion-base-2M** (7.6 MB, 64 dimensions): equal on the critical corpus; four times smaller vectors; half the peak memory in Rust (43 MB against 88 MB); 17 % more descriptions per second (25 276 against 21 563). It is weaker only on published averages. **It is the alternative to choose if the maintainer favours size and memory over published benchmark scores.**
- **potion-multilingual-128M** (512 MB; 64 MB after int8 and 128 dimensions): covers 101 languages, yet was not better than the English models on French pairs here. It loads in about 1 s and peaks at about 1.6 GB in Rust from the original files, and its original download (1 GB) exceeds the budget. A reduced artifact would have to be produced, published and maintained. Reconsider when cross-lingual pairs are measured.
- **Contextual encoders** (all-MiniLM-L6-v2, bge-small-en-v1.5, paraphrase-multilingual-MiniLM-L12-v2, multilingual-e5-small): better on word order (PAWS-X up to 0.62) and, for e5-small, on reworded duplicates (0.43 against 0.36). But they process 170 to 348 descriptions per second on one thread and 259 to 539 on four, below the budget of 2000 objects per second. They also weigh 91 to 471 MB, peak at 0.7 to 1.4 GB, and need an inference runtime that would dominate the wheel size. They could become an optional backend if a measured use case justifies it.
- **No model**: lexical comparison is the best single signal on the critical corpus (0.634) but recovers fewer reworded duplicates (0.31) and cannot handle other scripts; it remains a mandatory baseline in the hybrid score.

## Evidence

From the [spike](../../benchmarks/spikes/u004/README.md) (Apple M2, isolated processes, one model per process for memory):

| | potion-base-8M | potion-base-2M | multilingual-e5-small | lexical |
| --- | --- | --- | --- | --- |
| AUC critical corpus | 0.585 | 0.594 | 0.631 | 0.634 |
| AUC reworded duplicates | 0.356 | 0.368 | 0.432 | 0.312 |
| AUC French pairs | 0.557 | 0.561 | 0.606 | 0.591 |
| Descriptions/s, Rust, 1 / 4 threads | 21 563 / 46 541 | 25 276 / 63 157 | — | — |
| Descriptions/s, Python, 1 / 4 threads | 17 279 / 39 366 | 17 779 / 41 329 | 170 / 259 | — |
| First load, Rust | 0.032 s | 0.024 s | — | — |
| Peak memory, Rust | 88 MB | 43 MB | — | — |
| Weights | 30.2 MB | 7.6 MB | 471 MB | 0 |
| Published PairClass / Avg (MTEB) | 76.62 / 51.08 | 73.72 / 47.49 | — | — |

## Consequences

- Embeddings are one source of evidence. On the critical corpus no model beats lexical comparison alone, so quality depends on the hybrid score, vetoes and structured fields (U-008, U-010), which must be measured before any quality claim.
- Static models ignore word order: "Paris to Lyon" and "Lyon to Paris" embed identically. Lexical and structured evidence have to separate such pairs.
- U-009 integrates `model2vec-rs` without its default features (`local-only`, `fancy-regex`): no C library, no network crate in the inference path; the download module stays separate. The cost of the `tokenizers` crate on wheel size is measured there.
- Index memory scales with the dimension: 256 dimensions take 102 MB of float32 vectors per 100 000 records and text field, 64 dimensions 26 MB.

## Revisit when

- The maintainer prefers potion-base-2M's footprint, or the U-012 quality benchmark separates the candidates by more than its confidence interval.
- French or cross-lingual quality requires the multilingual model, or a reduced multilingual artifact fits the 100 MiB budget with measured gains.
- A use case requires word order or rewording recall that only a contextual encoder provides, at a measured and accepted cost.

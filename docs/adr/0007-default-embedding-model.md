# 0007. Default embedding model and model selection

- Status: Proposed, awaiting the maintainer's acceptance
- Date: 2026-10-07
- Roadmap: U-004

## Context

The maintainer requires a default model that is powerful, fast and small, and the ability to switch model with a string: a Hugging Face repository, a local directory or another source. The specification orders the criteria as minimal quality on the target case, then memory and size, latency, dependencies and maintenance ([04-models-and-cache.md §11](../../specs/04-models-and-cache.md)), and caps the default model download at 100 MiB ([07-performance.md](../../specs/07-performance.md)). The [U-004 spike](../../benchmarks/spikes/u004/README.md) measured twelve candidates from three families.

## Decision

- **Backend**: static embeddings in the Model2Vec format (`config.json`, `model.safetensors`, `tokenizer.json`), computed in Rust. It is the only backend shipped in the wheels.
- **Default model**: `minishlab/potion-base-8M` at revision `bf8b056651a2c21b8d2565580b8569da283cab23`; MIT; 256 dimensions; 30.2 MB of weights and a 0.7 MB tokenizer to download; English.
- **Language claim**: the default is announced as an English model. French text is handled through shared subwords and was measured on the critical corpus at the level of the multilingual static model, but no French or cross-lingual quality is claimed until the quality benchmark (U-012) validates it.
- **Selection by string**, as specified in [02-python-api.md §5.5](../../specs/02-python-api.md):
  - `"default"`;
  - a Hugging Face repository `org/name`, optionally `@revision`;
  - a local directory (absolute, `./`, `../`, `~` or `file://`);
  - a Hugging Face compatible mirror through `hub_url` or `HF_ENDPOINT`.

  Any repository in the Model2Vec format works: for example `minishlab/potion-base-2M` for the smallest memory footprint, or `minishlab/potion-multilingual-128M` for 101 languages.
- **Other Hugging Face models**: a repository in another format, such as a sentence-transformers encoder, is refused with an error that explains how to convert it. Model2Vec distillation turns such an encoder into a static model in minutes; a helper may be offered as an optional extra that the default installation does not pull in. A contextual backend is not part of the defaults (see Alternatives).

## Alternatives

- **potion-base-2M** (7.6 MB, 64 dimensions): equal on the critical corpus, four times smaller vectors, but weaker on published benchmarks (MTEB average 47.55 against 51.32). Kept as the recommended choice when memory dominates.
- **potion-multilingual-128M** (512 MB; 64 MB after int8 and 128 dimensions): covers 101 languages, yet was not better than the English models on French pairs here. The original download (1 GB) exceeds the budget, and a reduced artifact would have to be produced, published and maintained. Reconsider when cross-lingual pairs are measured.
- **Contextual encoders** (all-MiniLM-L6-v2, bge-small-en-v1.5, multilingual-e5-small): better on word order (PAWS-X up to 0.62) and, for e5-small, on reworded duplicates (0.43 against 0.36). They are 50 to 120 times slower (155 to 393 descriptions per second on one thread, below the 2000 objects per second budget), weigh 91 to 471 MB, and need an inference runtime that would dominate the wheel size. They could become an optional backend if a measured use case justifies it.
- **No model**: lexical comparison is the best single signal on the critical corpus (0.634) but recovers fewer reworded duplicates (0.31) and cannot handle other scripts; it remains a mandatory baseline in the hybrid score.

## Evidence

From the [spike](../../benchmarks/spikes/u004/README.md) (Apple M2, one thread):

| | potion-base-8M | potion-base-2M | multilingual-e5-small | lexical |
| --- | --- | --- | --- | --- |
| AUC critical corpus | 0.585 | 0.594 | 0.631 | 0.634 |
| AUC reworded duplicates | 0.356 | 0.368 | 0.432 | 0.312 |
| AUC French pairs | 0.557 | 0.561 | 0.606 | 0.591 |
| Descriptions per second, Rust | 20 014 | — | — | — |
| Descriptions per second, Python | 18 119 | 18 722 | 155 | — |
| Weights | 30.2 MB | 7.6 MB | 471 MB | 0 |
| Published MTEB average | 51.32 | 47.55 | — | — |

## Consequences

- Embeddings are one source of evidence. On the critical corpus no model beats lexical comparison alone, so quality depends on the hybrid score, vetoes and structured fields (U-008, U-010), which must be measured before any quality claim.
- Static models ignore word order: "Paris to Lyon" and "Lyon to Paris" embed identically. Lexical and structured evidence have to separate such pairs.
- U-009 integrates `model2vec-rs` without its default features (`local-only`, `fancy-regex`): no C library, no network crate in the inference path; the download module stays separate. The cost of the `tokenizers` crate on wheel size is measured there.
- Index memory scales with the dimension: 256 dimensions take 102 MB of float32 vectors per 100 000 records and text field.

## Revisit when

- The U-012 quality benchmark separates candidates by more than its confidence interval, or shows that French or cross-lingual quality requires the multilingual model.
- A reduced multilingual artifact fits the 100 MiB budget with measured French and cross-lingual gains.
- A use case requires word order or rewording recall that only a contextual encoder provides, at a measured and accepted cost.

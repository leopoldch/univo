---
license: apache-2.0
base_model: sentence-transformers/static-similarity-mrl-multilingual-v1
library_name: model2vec
pipeline_tag: sentence-similarity
tags:
- embeddings
- static-embeddings
- model2vec
- deduplication
language:
- multilingual
- ar
- bg
- ca
- cs
- da
- de
- el
- en
- es
- et
- fa
- fi
- fr
- gl
- gu
- he
- hi
- hr
- hu
- hy
- id
- it
- ja
- ka
- ko
- ku
- lt
- lv
- mk
- mn
- mr
- ms
- my
- nb
- nl
- pl
- pt
- ro
- ru
- sk
- sl
- sq
- sr
- sv
- th
- tr
- uk
- ur
- vi
- zh
---

# Univo default model: multilingual static embeddings, int8, 128 dimensions

This is the default embedding model of [Univo](https://github.com/leopoldch/univo), a local duplicate-detection library. It is a reduced copy of [`sentence-transformers/static-similarity-mrl-multilingual-v1`](https://huggingface.co/sentence-transformers/static-similarity-mrl-multilingual-v1) by the Sentence Transformers authors, in the [Model2Vec](https://github.com/MinishLab/model2vec) file layout.

A text's embedding is the mean of its token vectors, normalized to unit length: no neural network runs at inference. It covers the 50 languages of the original model and weighs 15 MB.

## Modifications

The original model is licensed under Apache-2.0, and so is this copy. The following changes were made to the original files of commit `b68f4122911bcffcd6e1f695f2d99cd6788972d8`:

- **Layout.** `0_StaticEmbedding/model.safetensors` and `0_StaticEmbedding/tokenizer.json` become `model.safetensors` and `tokenizer.json`, and a `config.json` is added. The tensor `embedding.weight` is renamed `embeddings`.
- **Quantization.** The float32 weights become int8, following Model2Vec's recipe. One scale is taken for the whole matrix (the largest magnitude divided by 127). Each value is rounded through float16 and clipped to [-127, 127]. The scale is not stored, since cosine similarity ignores it.
- **Truncation.** Only the first 128 of the 1,024 dimensions are kept. The original was trained with Matryoshka representation learning, so its leading dimensions carry the most information.
- **Tokenizer.** Truncation at 512 tokens is enabled. The vocabulary (105,879 tokens, multilingual BERT, uncased) and the normalization are unchanged.

[`tools/build_default_model.py`](https://github.com/leopoldch/univo/blob/main/tools/build_default_model.py) in the Univo repository reproduces these files byte for byte from the original, using the Python standard library only.

| File | Size | SHA-256 |
| --- | --- | --- |
| `config.json` | 79 B | `78f117ff0287650054a2a596313e72fe2e3dec4e4cc0d6348985a6e50cc19028` |
| `model.safetensors` | 13,552,600 B | `73e3c5e5496b5ddf9070238139b701839d763a479ab8023ec25bc951f780b6fc` |
| `tokenizer.json` | 1,715,270 B | `88d69c30a2a4395cf169631e47f3c4fbb95d300650028d541ec531ed18eb0ff1` |

## Usage

Univo downloads, verifies and caches this model on first semantic use. Afterwards it runs offline:

```python
from univo import Finder, Schema, Text

finder = Finder(schema=Schema(fields={"name": Text(mode="semantic")}))  # model="default"
report = finder.find([{"name": "Universität Wien"}, {"name": "University of Vienna"}])
```

Any Model2Vec runtime reads the files too, for example `StaticModel.from_pretrained(...)` in the `model2vec` Python package or `model2vec-rs` in Rust.

## Evaluation

All figures were measured by the Univo project on an Apple M2 with release builds. They are not the original authors' results.

- **Benchmark.** Organization names against their aliases and acronyms, from the public [ROR](https://ror.org) registry (CC0).
- **Selection.** This variant (int8, 128 dimensions) was chosen among several on a validation split: the highest recall at a threshold calibrated for at least 98% precision.
- **Test split.** The test split (1,341 duplicate and 1,173 distinct pairs) was also seen while comparing variants, so its figures are exploratory. Confirmation on an unseen set is pending.
- **Model compared.** The table compares this model with Univo's previous default, `minishlab/potion-base-8M`, which is English only.

| | This model | potion-base-8M |
| --- | --- | --- |
| Validation recall at ≥ 98% precision | 32.9% | 19.6% |
| Validation false matches (of 1,173) | 9 | 5 |
| Exploratory test recall, all languages | 30.9% | 17.9% |
| Exploratory test recall, English names | 54.6% | 44.0% |
| Exploratory test recall, other languages | 24.4% | 10.7% |
| Exploratory test precision (false matches of 1,173) | 95.6% (19) | 96.8% (8) |
| Files | 15 MB | 31 MB |
| Load time | 72 ms | 18 ms |
| Texts per second, 1 / 8 threads | 89k / 176k | 78k / 141k |

## Limitations

- **Word order is ignored.** The embedding averages token vectors, so "Paris to Lyon" and "Lyon to Paris" get the same vector.
- **Topic is not identity.** Texts about the same subject score high even when they name different entities. In Univo, embeddings are one source of evidence among lexical comparison, structured fields and veto rules, and never the only proof of a duplicate.
- **The 50 languages are those of the original training data.** Other languages and scripts fall back on shared subwords or unknown tokens.
- **Scores are cosine similarities, not probabilities.**

## License and attribution

Apache License 2.0; see `LICENSE`. The original model is © its authors, the Sentence Transformers team, published as `sentence-transformers/static-similarity-mrl-multilingual-v1`, and its approach is described in the post [Train 400x faster Static Embedding Models with Sentence Transformers](https://huggingface.co/blog/static-embeddings). Univo itself is MIT-licensed; this license covers only the files in this repository.

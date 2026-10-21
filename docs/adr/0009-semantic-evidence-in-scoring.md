# 0009. Semantic evidence in pairwise scoring

- Status: Accepted
- Date: 2026-10-08
- Roadmap: U-010

## Context

Pairwise scoring (U-008) compares text lexically. The model backend (U-009, [ADR 0008](0008-model-download-and-inference-backend.md)) now produces unit embeddings with the default static model. The schema declares three text modes, `lexical`, `semantic` and `hybrid` (the default), and the specifications describe hybrid fields only as "lexical and semantic", without a combination rule. The product specification requires a documented, versioned transformation of the cosine, warns that correlated signals are not independent evidence, and forbids tuning on test data.

## Decision

- A **semantic** field's similarity is the cosine of the two embeddings, clamped to `[0, 1]`: opposite and unrelated texts are equally dissimilar, and rounding cannot push identical texts above 1. Without both embeddings, because a text is missing or has no token the model knows, the field gives no evidence.
- A **hybrid** field is scored lexically, exactly like a lexical field, and is not embedded. Combining its cosine into the score waits for a combination that improves ranking on an annotated validation set (U-012).
- `SCORING_VERSION` becomes 2. `compare_with_vectors` takes the embeddings per field; `compare` keeps its behaviour for callers without embeddings.
- The engine embeds only semantic fields, one batch per field, encoding identical normalized texts once and storing them in one shared row.

## Alternatives

Measured on the critical corpus with `minishlab/potion-base-8M` (175 scored pairs: 93 duplicates, 82 distinct pairs without partition or veto constraint), AUC of the record score:

| Rule | All | Organizations | Products | Strings |
| --- | --- | --- | --- | --- |
| Lexical only (U-008) | 0.763 | 0.920 | 0.769 | 0.744 |
| **Semantic fields embedded, hybrid lexical (chosen)** | **0.776** | 0.959 | 0.675 | 0.744 |
| Hybrid: larger of lexical and semantic | 0.728 | 0.965 | 0.679 | 0.631 |
| Hybrid: mean of lexical and semantic | 0.738 | 0.956 | 0.674 | 0.631 |
| Hybrid: semantic only | 0.673 | 0.959 | 0.655 | 0.541 |

- **Larger of the two signals for hybrid fields**: the obvious reading of "lexical and semantic", but on names the cosine barely separates duplicates (AUC 0.571 on its own; 0.759 mean cosine for duplicates against 0.697 for distinct names). Taking the maximum mostly raises distinct pairs, and plain strings fall from 0.744 to 0.631.
- **Mean of the two**: the same loss, halved.
- **Rescaling the cosine** by the model's typical similarity of unrelated texts: plausible, since distinct descriptions average 0.575, but it is a calibration, to be fitted on validation data in U-012 rather than chosen on this corpus.

## Evidence

- The corpus is a regression set designed around hard cases, not a representative sample: these numbers rank rules, they do not estimate production quality.
- Descriptions gain most where they carry identity (organizations, 0.920 to 0.959) and lose where distinct items share a category (products, 0.769 to 0.675); constraint cases (vetoes, partitions) are excluded and keep their hard rules.
- `crates/univo-engine/tests/semantic_corpus.rs` measures the AUC with the real model in CI and fails below 0.756.

## Consequences

- With the default schemas, only fields declared `semantic` cost an embedding; plain strings and names are compared lexically, with no model call.
- `hybrid` and `lexical` currently score identically. The mode stays distinct in the schema, so that a validated combination can be introduced without changing schemas, with a new `SCORING_VERSION`.
- Results computed with version 1 are not comparable with version 2 for schemas with semantic fields.

## Revisit when

- U-012 provides an annotated validation set: fit the cosine transformation and the hybrid combination there, and measure on the held-out test set.
- Another default model, or a contextual one, changes how names embed.

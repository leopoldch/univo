# Duplicate definitions

A duplicate is defined per profile, never globally. Each profile fixes which fields are compared, how, and which contradictions forbid a match. These definitions are the labelling rules of the [critical corpus](../tests/fixtures/critical/README.md) and the reference for the matching contract in [specs/03-matching.md](../specs/03-matching.md).

## Shared rules

- **Missing values.** A field is *missing* when it is absent or `null`. A missing value is unknown: it is never a contradiction and never evidence of equality. It lowers coverage.
- **Present values.** Empty strings, `0` and `false` are present values. They are distinct from each other and from a missing value, and they can contradict another present value.
- **Types.** Values of different types are different values: `3` and `"3"` contradict, `0` and `false` contradict.
- **Normalization.** Unicode NFC is applied to text. Case is folded for text fields. Accents, digits, signs, punctuation and units are kept. No alias, unit conversion or legal-suffix rule applies unless the schema declares it.
- **Lists** keep their order. A reordered list is a different value unless the schema declares set semantics.
- **Partitions.** Records in different partitions are never compared. A record without its partition key is invalid.
- **Vetoes.** A field declared as discriminant (`conflict="reject"`) forbids a match when both values are present and differ, even if every other field is equal.
- **Informative evidence.** A pair whose informative fields are all missing or blank has no score and is not a match.
- **Exact matches.** A pair is `exact` when every selected field is equal after normalization, including the same missing state.

## Profiles

### strings

Plain strings, such as a list of names.

| Field | Comparison |
| --- | --- |
| the string | text, hybrid (lexical and semantic) |

Duplicate: the two strings designate the same thing despite spelling, spacing, accent, case, abbreviation or word-order variation. Different entities with related meanings are distinct.

### organizations

| Field | Comparison |
| --- | --- |
| `id` | record identifier, never compared |
| `tenant_id` | partition key |
| `name` | text, hybrid |
| `domain` | exact, `hostname` normalizer; evidence, not a veto |
| `description` | text, semantic |

Duplicate: the same real-world organization within the same tenant. A different domain lowers the score but does not veto: companies own several domains. Competitors, homonyms and subsidiaries are distinct.

### products

| Field | Comparison |
| --- | --- |
| `id` | record identifier, never compared |
| `name` | text, hybrid |
| `sku` | exact, discriminant |
| `description` | text, semantic |

Duplicate: the same sellable item. Different variants (size, colour, version, capacity, inclusion of an accessory) are distinct products. Two different SKUs are a veto.

### configurations

| Field | Comparison |
| --- | --- |
| `id` | record identifier, never compared |
| `service` | exact, discriminant |
| `environment` | exact, discriminant |
| `params.*` | each parameter exact and discriminant, typed |
| `description` | text, semantic |

Duplicate: two records that configure the same service in the same environment with the same parameter values. Any present parameter that differs is a veto. Unit conversion (`1 s` versus `1000 ms`) and environment aliases (`prod` versus `production`) apply only when declared in the schema; the default profile declares none.

## Validity

Inputs are rejected instead of being coerced. The critical corpus pins these errors:

| Input | Error |
| --- | --- |
| a record of the wrong shape, a text field holding a number or boolean, a missing `id` or partition key | `InvalidRecordError` |
| two records with the same `id` in one call | `DuplicateIdError` |
| nesting deeper than the configured limit | `ResourceLimitError` |

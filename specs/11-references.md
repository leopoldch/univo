# Sources techniques

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 30. Sources techniques consultées

Sources primaires consultées pour ce plan. Les API Univo, budgets, étapes et choix proposés ci-dessus sont notre spécification ; ils ne sont pas des fonctionnalités déjà fournies par ces projets. Figer les versions au démarrage de l’implémentation.

- [S1 — Model2Vec Rust](https://github.com/MinishLab/model2vec-rs) : existence d’un backend Rust pour embeddings statiques et chargement local.
- [S2 — PyO3, parallelism](https://pyo3.rs/main/parallelism) : détachement du runtime Python pour les calculs Rust.
- [S3 — maturin, distribution](https://www.maturin.rs/distribution.html) : distribution de wheels et conformité des bibliothèques natives.
- [S4 — maturin, bindings](https://www.maturin.rs/bindings) : choix de bindings et limites des variantes d’ABI.
- [S5 — SemHash](https://github.com/MinishLab/semhash) : concurrent direct pour la déduplication sémantique et les données multicolonnes.
- [S6 — Model2Vec](https://github.com/MinishLab/model2vec) : famille d’embeddings statiques à comparer.
- [S7 — Dedupe](https://github.com/dedupeio/dedupe) : référence pour la déduplication de données structurées.
- [S8 — Splink](https://github.com/moj-analytical-services/splink) : référence pour le rapprochement probabiliste d’enregistrements.
- [S9 — FastEmbed Rust](https://github.com/anush008/fastembed-rs) : autre piste de backend local, à comparer si nécessaire.

Les sources ne prouvent ni un gap de marché, ni les performances futures d’Univo. Ces deux points seront évalués par les jalons du plan.

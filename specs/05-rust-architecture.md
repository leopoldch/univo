# Architecture Rust et concurrence

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 18. Architecture du dépôt

Workspace proposé :

```text
Cargo.toml
Cargo.lock
pyproject.toml
crates/
  univo-core/          # types, schéma, normalisation, scoring, résultats
  univo-engine/        # orchestration, index, cache, mutations, snapshots
  univo-embed/         # trait et backend choisi ; réseau optionnel
  univo-python/        # PyO3, conversions et erreurs
python/univo/
  __init__.py
  _api.py             # ergonomie uniquement
  _types.pyi
  py.typed
tests/
  python/
  fixtures/
benchmarks/
  datasets/
  quality/
  performance/
  baselines/
docs/
  quickstart.md
  concepts.md
  offline.md
  performance.md
  limitations.md
  adr/
.github/workflows/
```

Le crate cœur ne dépend pas de PyO3, d’un client HTTP ou du runtime d’inférence. Le moteur orchestre les composants ; les bindings ne réimplémentent pas le scoring. Les utilisateurs Rust accèdent aux mêmes décisions via une façade publique stable.

Traits internes à limiter aux vrais points de substitution : `Embedder`, `CandidateIndex`, `SnapshotStore` si nécessaire. Ne pas créer un framework de plugins dynamique avant d’avoir deux implémentations utiles.

Erreurs publiques : `InvalidRecordError`, `InvalidSchemaError`, `DuplicateIdError`, `ModelUnavailableError`, `ModelIntegrityError`, `IndexCompatibilityError`, `IndexCorruptionError`, `ResourceLimitError`, `IndexLockedError`, `SchemaMismatchError`, sous une base commune `UnivoError`. Conversion Rust vers exceptions Python stable et documentée. Pas de panic à travers la FFI.

Modèle d’erreur commun à tous les langages ([catalogue](../docs/reference/errors.md), [ADR 0004](../docs/adr/0004-error-model.md)) : chaque erreur porte une catégorie (un type d’exception par catégorie et par langage), un code stable (`record.wrong_type`), un message complet, une indication de correction, le chemin du champ, des détails structurés et, pour un lot, la position de l’enregistrement. Tout est construit dans le cœur Rust ; les bindings ne composent aucun message. Les messages nomment chemins, types et tailles, jamais les valeurs des champs.

## 16. Mutations, concurrence et check_or_add

Séparer IDs externes stables, slots internes et génération. `upsert` construit les nouvelles représentations avant publication. `remove` marque les slots supprimés et les masque immédiatement dans les résultats ; compaction/reconstruction récupère ensuite l’espace.

Le remplacement et l’exclusion des tombstones doivent concerner tous les index : exact, lexical, ANN et partitions. Un index ANN obsolète ne doit pas exposer un objet supprimé ni la précédente version d’un objet.

Lectures sur une génération stable ; mutations sérialisées par instance. Dans la première implémentation, préférer un verrou simple correct à une structure lock-free complexe. Libérer le runtime Python pendant les longs calculs Rust sans manipuler d’objets Python dans cette zone. PyO3 documente ce mécanisme de détachement. [S2](11-references.md)

`check_or_add`, version ultérieure : effectuer recherche et décision d’insertion dans une même section atomique locale. Encoder hors verrou si possible, puis revalider la génération/rechercher sous verrou avant commit.

Si une correspondance dépasse le seuil configuré, retourner `review` ou `matched` selon les règles ; sinon insérer et retourner `inserted`. L’absence de candidat dans un mode borné n’est pas une preuve d’absence de doublon.

**Limite essentielle : l’atomicité locale ne couvre pas une DB externe ni plusieurs processus indépendants.** Une application distribuée doit organiser sa transaction, son idempotence et ses contraintes d’unicité. Ne jamais présenter `check_or_add` comme une protection globale contre les courses entre services.

`threads` contrôle le budget global. Éviter de multiplier les pools Rust, tokenizer et runtime d’inférence. Tester un thread, puis plusieurs ; publier le nombre utilisé. Ne pas faire dépendre le bon fonctionnement d’un fork après initialisation du runtime.

U-016 garde l'intégration HNSW, le filtrage et le reranking dans Rust,
sans nouvelle thread pool. Le kernel Euclidien à huit accumulateurs est
safe et portable. Les graphes copient les vecteurs distincts ; la recherche
alloue un visited set par graphe parcouru. Construction de graphes seulement
pour le profil explicite et les partitions assez grandes ; compaction et
restauration reconstruisent depuis les vecteurs, sans inférence.
Voir [ANN](../docs/guide/ann.md).

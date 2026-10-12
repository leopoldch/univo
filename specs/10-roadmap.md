# Backlog, jalons, risques et extensions

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 23. Backlog exécutable et ordre des PR

Chaque PR indique son contrat, ses tests, les chiffres pertinents et les limites. Pas de fonctionnalité Java ou connecteur avant la validation du moteur.

| ID | Travail | Dépendance | Critère de fin |
| --- | --- | --- | --- |
| U-001 | Vérifier nom PyPI/crates/GitHub, licence proposée, objectifs | aucune | décision écrite ; aucune disponibilité supposée |
| U-002 | Constituer corpus et définition des doublons par profil | aucune | fixtures versionnées et labels revus |
| U-003 | Implémenter baselines lexicales et lancer SemHash | U-002 | rapport reproductible qualité/temps/RAM |
| U-004 | Spike statique vs contextuel local | U-002 | modèle candidat choisi avec compromis et empreinte |
| U-005 | Workspace Rust, package Python, wheel minimale | U-001 | import fonctionne sans Rust sur plateformes pilotes |
| U-006 | Types, schémas, normaliseurs, erreurs | U-002,U-005 | invariants et cas ambigus couverts |
| U-007 | Exact hash vérifié, partitions, IDs | U-006 | aucun faux exact, aucune fuite inter-tenant |
| U-008 | Comparateurs lexicaux et score pairwise | U-006 | résultats déterministes sur fixtures |
| U-009 | Backend modèle local + registry + offline | U-004,U-005 | encodage batch, téléchargement contrôlé, réseau interdit testé |
| U-010 | Recherche exacte et union des candidats | U-007,U-008,U-009 | oracle exhaustif sans matrice N×N |
| U-011 | API Python find/add/search et rapports | U-010 | quickstart exécutable, erreurs stables |
| U-012 | Calibration du profil et benchmark 0.1 | U-003,U-011 | publication alpha avec chiffres et limites |
| U-013 | Cache borné et invalidation | U-009,U-011 | zéro réencodage des données inchangées hors éviction |
| U-014 | Snapshot versionné et load/save | U-011 | round-trip et injection de panne réussis |
| U-015 | upsert/remove et génération | U-013,U-014 | résultats cohérents après mutations |
| U-016 | Choix ANN, intégration et rappel | U-010,U-015 | budgets mesurés, oracle conservé |
| U-017 | Concurrence et check_or_add local | U-015,U-016 | course à deux appels testée ; limite DB externe documentée |
| U-018 | Groupes optionnels conservateurs | U-012 | pas de transitive closure présentée comme identité |
| U-019 | Wheels complètes et documentation | U-011,U-014 | installation neuve sur toutes plateformes supportées |
| U-020 | Benchmarks comparatifs et release candidate | U-016,U-017,U-019 | gates acceptées ou écarts explicitement traités |
| U-021 | Stabilisation 1.0 | U-020 | contrats/compatibilité/format et release checklist signés |

Chemin critique : corpus → choix modèle → moteur exact → API → qualité → persistance/mutations → ANN → packaging final. La portabilité des wheels est testée tôt pour éviter une refonte tardive.

U-004 status: the [spike](../benchmarks/spikes/u004/README.md) compared twelve candidates; [ADR 0007](../docs/adr/0007-default-embedding-model.md) proposes the static `potion-base-8M` model and records the model-selection contract. The choice awaits the maintainer's acceptance and the U-012 quality benchmark.

U-012 status: offline calibration/quality tooling implemented on a pinned public ROR name-alias corpus with disjoint entities. Validation thresholds, held-out metrics, candidate/oracle agreement, full critical profile gates and language/model footprints are recorded in the [benchmark](../benchmarks/quality/README.md) and [ADR 0011](../docs/adr/0011-offline-quality-calibration.md). The declared quality gates fail on name aliases; no calibrated production profile, default-model change, alpha publication or completion of U-012 is claimed. Reviewed ambiguous labels, out-of-domain/cross-language support, SemHash comparison and full semantic Finder performance remain pending.

U-011 status: merged in PR #15. Python exposes `Finder`, `Schema`, `Text`, `Exact`, `Match` and `Report`, with batch conversion of selected paths, atomic add, temporary find, read-only search, typed errors, lazy/offline model resolution and separate output/candidate truncation ([quickstart](../docs/quickstart.md), [ADR 0010](../docs/adr/0010-python-finder-and-exact-fallback.md)). The macOS arm64 wheel is tested outside the source tree without Rust, and end-to-end lexical measurements are recorded in [performance](../docs/performance.md). No publication, calibration, persistent snapshots or multilingual default-model decision is included.

U-010 status: done. Semantic text fields are compared by the cosine of their embeddings, while hybrid fields stay lexical on the evidence of the critical corpus ([ADR 0009](../docs/adr/0009-semantic-evidence-in-scoring.md)). `SearchIndex` gathers candidates through one lane per field: the trigram index for lexical and hybrid text ([lexical candidates](../docs/lexical-search.md)), exhaustive vector search for semantic text ([vector search](../docs/vector-search.md)), equal values and object leaves for exact fields, and the exact index for records equal on every field. A score of at least `s` needs one field of at least `s`, so the result equals the exhaustive oracle, which scores every record of the partition without a pairwise matrix ([search](../docs/search.md)). At 100,000 records the p95 is 9.9 ms in one partition and 8.0 ms for queries from a group of 10,000 identical records, on one thread.

U-009 status: the Rust [`univo-embed`](../crates/univo-embed) crate parses model references, pins the default model's checksums, plans, verifies and caches downloads, and encodes with static Model2Vec models; Python downloads through the standard library, exposes `univo.models` and `python -m univo.models`, and is tested against a local mirror and with the network forbidden ([models](../docs/models.md), [ADR 0008](../docs/adr/0008-model-download-and-inference-backend.md)). The model is not yet used for matching: U-010 integrates it.

U-008 status: [pairwise scoring](../docs/scoring.md) compares prepared records field by field with lexical signals (bit-parallel edit distance for short text, trigram Jaccard), exact and key-by-key comparison, vetoes, coverage and explanations. On the critical corpus it respects every hard rule and ranks duplicates with an AUC of 0.763; semantic similarity waits for U-009 and thresholds for U-012.

U-007 status: the [Rust normalized-equality index](../docs/exact-index.md) is implemented, with batches validated before insertion, typed IDs, verified partitions and observable output limits. Tests compare it with the exhaustive `exact_match` oracle and inject collisions. This does not complete U-010 (candidate union and scoring oracle) or U-011 (Python `Finder`).

Les tâches indépendantes peuvent être développées séparément, mais les contrats de schéma, modèle et résultats doivent être fixés avant leur intégration.

## 24. Jalons et estimation de charge

Estimations de planification pour un développeur expérimenté, non engagement de délai. Les backends et plateformes peuvent déplacer fortement ces chiffres.

| Jalon | Charge indicative | Livrable |
| --- | --- | --- |
| J0 — cadrage, corpus initial et concurrence | 2–4 jours | définition de réussite et baselines |
| J1 — spike modèle et wheel native | 3–5 jours | choix technique et installation prouvée |
| J2 — cœur exact, lexical, sémantique | 5–8 jours | moteur correct sans ANN |
| J3 — API et alpha 0.1 | 3–5 jours | utilisation réelle en quelques lignes |
| J4 — cache, snapshots et mutations | 5–8 jours | beta locale persistante |
| J5 — ANN et profilage | 4–7 jours | passage à l’échelle mesuré |
| J6 — durcissement, wheels, docs et RC | 5–8 jours | publication testable et reproductible |

Total indicatif : 27–45 jours de travail, plus éventuelle collecte/annotation substantielle. Un prototype peut être plus rapide ; cela ne réduit pas les vérifications nécessaires pour annoncer « rapide, fiable et léger ».

Décisions de sortie : après J1, choisir le backend ; après J3, vérifier la différenciation ; après J5, figer le défaut exact/ANN et ses budgets ; après J6, décider si la 1.0 est justifiée.

## 25. Risques et réponses prévues

| Risque | Signal | Réponse |
| --- | --- | --- |
| Embeddings confondent thème et identité | compétiteurs renvoyés comme doublons | signaux structurés, négatifs difficiles, décision prudente |
| Modèle statique manque des détails | erreurs de négation/version | veto typés ou backend contextuel optionnel |
| Trop de configuration | quickstart inutilisable sans expertise | raccourci transparent et profils testés |
| Défaut universel trompeur | qualité très variable par domaine | périmètre/languages annoncés, calibration locale |
| ANN perd des doublons | rappel candidats insuffisant | union lexicale, paramètres, fallback exact |
| Dépendances gonflent la wheel | taille/runtime dépasse le budget | mesurer features et choix backend, pas compter seulement packages |
| Mutations laissent des vecteurs obsolètes | anciens objets visibles | générations, tombstones et tests de cohérence |
| Nouveaux tenants masqués par top-k global | rappel mauvais sur partitions rares | partitionner ou filtrer au retrieval |
| Téléchargement impossible en entreprise | premier usage bloqué | modèle préchargeable, chemin local, erreur claire |
| Concurrence avec SemHash sans avantage | résultats identiques, API équivalente | recentrer sur intégration incrémentale mesurée |
| Scope trop grand | Java/CLI/connecteurs avant stabilité | reporter après preuve d’usage Python/Rust |

## 28. Extensions après validation

Java : mesurer d’abord la demande et les cas d’intégration. Réutiliser le cœur via une interface stable appropriée, avec artefacts natifs par plateforme. Tester les mêmes fixtures, pas une réécriture du scoring.

Autres extensions possibles : adaptateurs pandas/Polars optionnels, vecteurs fournis par l’utilisateur, politiques apprises compactes, export portable avec modèle, quantification, connecteurs de lecture, outil de revue des candidats. Chacune doit résoudre un besoin observé et conserver le budget de dépendances du chemin par défaut.

Les images/audio, l’équivalence de programmes et la résolution distribuée d’identité sont des projets distincts tant que leur modèle de données et leur évaluation ne sont pas spécifiés.

## 29. Première séquence de travail concrète

1. Confirmer le nom et écrire les règles des trois cas d’usage.
2. Créer les 200 cas critiques avant toute optimisation.
3. Exécuter lexical seul, SemHash et deux backends d’embeddings locaux.
4. Construire une wheel Rust/Python minimale sur Linux et macOS ; ajouter Windows immédiatement après.
5. Figer schéma, erreurs, Match et Report ; documenter les profils.
6. Construire le moteur exact et le quickstart, avec téléchargement paresseux et offline.
7. Mesurer qualité et poids ; décider si le positionnement est suffisamment distinct.
8. Ajouter persistance et mutations, puis ANN uniquement quand le volume le justifie.

Le premier résultat à rechercher n’est pas « une API qui retourne des voisins ». C’est un exemple métier reproductible où Univo détecte les bonnes paires, explique les contradictions et reste simple à installer.

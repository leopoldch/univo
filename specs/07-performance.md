# Mémoire, performances et budgets

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 14. Mémoire et budgets de ressources

Pour N objets, m champs sémantiques, dimension d et stockage f32 :

\[
M_{vecteurs}=4Nmd\text{ octets}.
\]

À d=256, N=100 000, m=1 : 102,4 MB décimaux, soit environ 97,7 MiB, avant modèle, objets, index, cache et buffers. À un million : environ 0,954 GiB pour les seuls vecteurs. Trois champs triplent ce poste.

Donc ne pas promettre « très léger à un million d’objets » sans préciser dimension et matériel.

Principes : slots internes u64, IDs externes séparés, buffers réutilisés, lots bornés, éviter les copies temporaires de tous les textes, pas de tableaux Python pour chaque vecteur. Conserver uniquement les champs sélectionnés et métadonnées nécessaires ; les autres champs restent chez l’appelant.

La quantification et mmap viennent après profilage. Le gain en mémoire doit être comparé à la perte de qualité, au coût de décodage et à la portabilité. Ne pas annoncer « zero-copy » tant que les conversions et la durée de vie des buffers ne le prouvent pas.

## 21. Protocole de performance et objectifs

Les nombres suivants sont des budgets proposés. Ils ne sont ni des résultats acquis ni des promesses sur tout ordinateur. Les confirmer ou les réviser explicitement après le premier spike.

Machine de référence à enregistrer : CPU et instructions, RAM, OS, Python, Rust, versions, nombre de threads, alimentation et stockage. Mesurer release builds, sans téléchargement et avec modèle déjà présent pour les mesures chaudes ; mesurer séparément l’installation et le premier téléchargement.

Workload standard proposé : 100 000 objets, ID + tenant + nom de 20 caractères + description d’environ 30 tokens ; publier aussi distributions de longueur et nombre de partitions. Évaluer 1k/10k/100k, puis 1M si la RAM le permet.

| Mesure | Budget initial proposé |
| --- | --- |
| Import Python sans modèle | p95 ≤ 20 ms (resserré de 100 ms le 2026-10-07 : l’import doit rester imperceptible ; mesuré à 0,85 ms en U-005) |
| Construction Finder sans modèle | p95 ≤ 20 ms |
| Premier encodage, modèle local en cache | p95 ≤ 2 s |
| Recherche chaude complète à 100k, 4 threads | p95 ≤ 50 ms ; p99 publié |
| Throughput initial du backend compact | ≥ 2 000 objets/s, selon workload ci-dessus |
| Wheel compressée, sans modèle | ≤ 30 MiB |
| Téléchargement du modèle par défaut | ≤ 100 MiB |
| Installation hors modèle | ≤ 100 MiB |
| RSS moteur chargé, sans corpus | ≤ 300 MiB |
| Surcoût mémoire total à 100k | ≤ 512 MiB, incluant champs/index/vecteurs/buffers |

Si plusieurs champs sémantiques ou le backend retenu rendent le budget irréaliste, documenter le compromis et la décision avant release. Ne pas prétendre satisfaire un budget en excluant un runtime téléchargé séparément.

Mesurer : normalisation, tokenization/encodage, retrieval, reranking, conversion FFI, coût total, add/upsert/remove, save/load et RSS peak. Distinguer requête qui réutilise un vecteur du cache et requête réellement nouvelle.

Protocole : warmup, au moins 1 000 requêtes pour p95, volume supérieur pour p99 stable ; cinq répétitions indépendantes ; mêmes corpus, threads et conditions pour les baselines. Publier médiane, dispersion et taux d’erreur. Mesurer un seul thread puis quatre, pas seulement tous les cœurs.

La vitesse ANN est comparée à rappel équivalent. Pour un profil ANN viser recall@k ≥ 99 % contre l’oracle exact sur les requêtes d’évaluation, et mesurer séparément le rappel final des doublons hybrides. Un bon recall vectoriel ne prouve pas un bon rappel métier.

Les budgets appliqués par les tests sont centralisés dans [benchmarks/budgets.toml](../benchmarks/budgets.toml) ; la politique de mesure est décrite dans [docs/performance.md](../docs/performance.md).

U-007 adds a dense-insertion regression budget: 40,000 records in one equality group, one thread, median of five after one warmup. Full `ExactIndex.add` must take at most 25 ms on the reference machine, for both one batch and single-record batches, with no IDs or decreasing integer IDs. Preparation and index destruction are measured outside this interval. The test also reports 10k and 20k scaling and verifies complete counts and ordered search results. The previous full-bucket sort took about 718 ms at 40k without IDs; incremental postings bring this to approximately 2 ms, or 7 ms with IDs. Shared CI applies the documented time slack. This budget covers insertion only, not end-to-end raw-input ingestion, model inference or search latency.

Gates de non-régression proposés sur runner stable : alerte dès +10 % de p95, +10 % de RSS ou +10 % de taille ; confirmation par répétition avant blocage. GitHub runners partagés servent aux smoke tests, pas aux petites différences de performance.


U-011 enforces the existing Finder construction budget (p95 ≤ 20 ms, 30 fresh interpreters, import of the facade outside the construction timer) in `python.finder_construction`. The [Python benchmark](../benchmarks/performance/python_finder.py) measures full add/search conversion and reports index estimates and process peak RSS on synthetic lexical records. It does not substitute for the semantic standard workload or U-012 quality benchmark. Exhaustive find defaults to a declared maximum of 1,000,000 partition pair-count upper bounds; callers may raise it explicitly. Candidate find can still need quadratic work on dense matches; the output limit is not a CPU limit.

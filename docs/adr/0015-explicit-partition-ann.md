# 0015. HNSW explicite par partition

- Status: Accepted for experimental U-016 implementation; release gates remain open
- Date: 2026-10-12
- Roadmap: U-016

## Context

Le parcours sémantique U-010 est complet mais linéaire par requête. U-015
fournit masquage des versions et compaction. Un backend ANN doit préserver
le scoring, l'oracle, les partitions et les wheels portables, avec un coût
mesuré et une approximation observable. Un gain vectoriel isolé ne suffit
pas à changer le défaut ou à garantir le rappel des doublons.

## Decision

Ajouter `retrieval="hnsw"` opt-in, conserver `retrieval="exact"` par défaut
et refuser `auto` tant qu'un crossover n'est pas démontré sur plusieurs
workloads. Utiliser `fast-hnsw` **1.0.1 épinglé**, M=16,
efConstruction=200, seed=16, `PruneStrategy::Heuristic`. La distance
est l'Euclidienne réelle, avec accumulation portable sur huit lanes ;
la shortcut triangulaire du pruning ne convient pas à la distance carrée.
Le cosinus et la décision demeurent dans `univo-core`.

Construire un graphe par champ sémantique et partition réelle, seulement
à partir de 10 000 slots retenus. Ce seuil est une protection des petits
workloads, pas un profil automatique optimal. Dédupliquer les vecteurs
égaux après vérification de hash, préserver les slots et filtrer tombstones
avant clipping. Garder l'union exacte/lexicale et le reranking Rust.
Paramètres initiaux efSearch=512, plafond de 1024 records par lane.
Les doubles demandes jusqu'au plafond sont explicites ; leur coût entre
dans le majorant `max_semantic_comparisons` de find. Un contrôle préalable
du minimum dense refuse les lots hors budget avant encodage et construction ;
le contrôle final conserve le majorant complet. Le find temporaire exhaustif
ne construit aucun graphe ANN.

Ne pas utiliser le format persistant/mmap de la dépendance : snapshots
Univo validés, vecteurs conservés, identité ANN versionnée dans le manifest,
reconstruction sans inférence. Les paramètres entrent dans le fingerprint.
Le [contrat](../ann.md) décrit flags, plafonds, mutations, coûts et
variations du rappel après reconstruction.

## Alternatives

- Parcours complet : conservé comme défaut et fallback. Il est plus rapide
  pour plusieurs petits workloads et apporte la complétude vectorielle.
- [hnsw_rs 0.3.5](https://github.com/jean-pierreBoth/hnswlib-rs) : référence
  Rust MIT/Apache-2.0 plus établie. Le spike l'a comparée à même M et ef.
  Dépendances obligatoires supplémentaires, seed non configurable et travail
  Rayon dans Drop ne conviennent pas aussi bien au contrôle léger des pools.
- [fast-hnsw 1.0.1](https://github.com/nataliyakosmyna/fast-hnsw) : seulement
  `rand`/`memmap2` directs, seed configurable, aucune thread interne de
  recherche/destruction ; projet récent, moins éprouvé. Accepté avec tests
  locaux, version épinglée et réexamen avant release. Sa déclaration MIT
  est dans Cargo/README ; l'archive 1.0.1 n'inclut pas de fichier LICENSE.
  La distribution des notices est à traiter dans U-019.
- Pruning `Simple` du même backend : moins cher à construire mais rappel
  limité à **74,45 %** sur les clusters synthétiques à 10k, même à ef=1024.
  Rejeté ; le pruning Heuristic atteint 100 % sur ce fixture.
- Implémenter notre propre HNSW, quantifier ou utiliser IVF : complexité et
  validation supplémentaires sans preuve qu'il faille les introduire ici.

## Evidence

Le [spike et ses résultats bruts](../../benchmarks/spikes/ann-backends/README.md)
comparent les backends, le kernel et les formes de données. Le screening
synthétique comprend 200 requêtes par point ; ce n'est pas le protocole
final de p95. Les paramètres retenus sont figés avant le test ROR.

Avec les embeddings locaux potion-base-8M, 8 707 noms train/validation et
2 048 noms test distincts après normalisation : recall@10 strict par ID
**99,9951 %** à ef512, cinq processus indépendants seed fixe ; p95 vectoriel
médian 0,320 ms, construction 2,458 s, RSS processus maximal 38,813 Mio.
Le parcours exact optimisé prend environ 0,227 ms de moyenne : aucune
accélération démontrée sur cette petite base, inférieure au seuil de
construction des graphes en production. Ce test mesure les voisins
vectoriels, pas le rappel des alias métier ; les gates U-012 restent échouées.

Les tests couvrent collisions de hash, séparation des partitions avant
retrieval, absence de deleted IDs, réutilisation des vecteurs, compaction,
manifest incompatible, reconstruction sans inférence et oracle conservé.
La wheel arm64 macOS mesure 1,789 Mio contre 1,762 Mio U-015 : environ
27 Kio supplémentaires compressés. Aucun runtime Python ni C++ ajouté ;
115 noms de crates liés contre 112, sept versions de packages supplémentaires.
Les mesures complètes installed-wheel sont dans [performance](../performance.md).

## Consequences

Aucun graphe alloué par défaut. HNSW copie les vecteurs distincts et ses
buffers visited ont un coût O(nœuds) par requête. Le nombre de candidats
borné n'est ni une garantie de recall, ni une borne du RSS global. L'ANN
peut être plus lent que le parcours dense et consomme plus de mémoire.
Retrait logique immédiat ; compaction/load reconstruisent la topologie
et peuvent modifier la récupération approximative sans changement de
génération. Le budget incrémental de 512 Mio reste inchangé et ouvert.
Pas de version de scoring ni de changement du modèle par défaut.

## Revisit when

Les corpus métiers ou grands jeux vectoriels échouent à recall@k ≥99 %,
les tombstones dégradent la qualité, le coût de copie/reconstruction excède
les budgets, la maturité/licence du backend pose problème, ou un profil
`auto` peut être choisi avec evidence d'un crossover fiable. Toute évolution
du kernel/backend/paramètres de construction doit changer l'identité ANN
et rejeter explicitement les snapshots incompatibles.

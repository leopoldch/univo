# U-016 : choix ANN et mesures

Machine locale : Apple M2, 16 Gio RAM, macOS 27.0.1 arm64, Rust 1.99.0,
Python 3.14.7 ; builds release fat LTO, CPU portable, disque/modèle chauds,
aucun réseau pendant la mesure. ANN et scoring à un thread ; encodage
contrôlé par le Finder. Les RSS couvrent le processus complet.
Les backends n'ont pas exactement le même kernel SIMD ni un seed commun :
la comparaison décrit leurs implémentations réelles, pas un effet isolé de
l'algorithme. Voir [ADR 0015](../../../docs/adr/0015-explicit-partition-ann.md).

## Screening synthétique

```sh
cargo build --release --manifest-path benchmarks/spikes/u016/rust/Cargo.toml
RAYON_NUM_THREADS=1 benchmarks/spikes/u016/rust/target/release/u016-ann-spike fast 10000 256 uniform
RAYON_NUM_THREADS=1 benchmarks/spikes/u016/rust/target/release/u016-ann-spike reference 10000 256 clustered
```

Vecteurs unitaires, seed données 41, 32 centres pour les clusters, 200
requêtes proches de records avec bruit indépendant ; top-10 oracle complet.
M=16, efConstruction=200, efSearch=32/128/512/1024, fast seed16,
hnsw_rs seed aléatoire. Un processus par point du screening, pas cinq :
ces latences ne sont pas des gates de release.
[Screening](results/screening.json) : comparaison initiale avec kernel
Euclidien serial et oracle alloué. [Kernel](results/kernel.json) :
accumulation portable huit lanes côté fast ; ne pas mélanger les latences
exactes de ces anciennes mesures avec le programme final, qui utilise
maintenant le même kernel optimisé et un tas borné pour l'oracle.

À 10k uniformes, ef512, fast Heuristic atteint recall@10=99,75 % et p95
0,464 ms avec le kernel optimisé. Construction 5,782 s contre 27,691 s avec
le kernel serial. Sur clusters, Heuristic atteint 100 % contre 74,45 %
pour Simple quel que soit ef : le dernier est exclu de la production.
Ces nombres ne démontrent pas un rappel universel à 100k ou sur tout corpus.

## Test ROR vectoriel figé

Préparer le corpus U-012 selon [quality](../../quality/README.md), puis,
avec une wheel installée et le modèle local :

```sh
python benchmarks/spikes/u016/export_names.py pairs.jsonl /path/to/model /tmp/ann-vectors
benchmarks/spikes/u016/rust/target/release/u016-ann-spike fast 0 256 ror /tmp/ann-vectors/background.bin /tmp/ann-vectors/queries.bin
```

Le binaire d'entrée est un export de benchmark de confiance (header
u64 nombre/dimension, f32 little-endian), pas un format public de snapshot.
Noms NFC/casefold uniques, background train+validation, queries test excluant
les textes normalisés identiques au background ; les entités U-012 sont
disjointes. Vecteurs absents retirés et comptés dans le [manifest](results/ror-input.json).
Modèle potion-base-8M, révision bf8b056651a2c21b8d2565580b8569da283cab23,
256 dimensions, trois fichiers de 29,49 Mio. Seuil/params choisis avant test ;
aucun entraînement ou ajustement sur held-out test.

Cinq processus indépendants avec graphe seeded identique, 2 048 queries,
8 707 vecteurs background. `recall_at_10` compte les IDs des dix voisins
Euclidiens exacts ; pas de correction de ties ni de changement de métrique.
Les scores finaux Univo utilisent le cosinus ; ce test ne mesure pas la
classification des doublons ni les lanes hybrides.

| efSearch | recall@10 (chaque répétition) | médiane p95 (ms) |
| --- | --- | --- |
| 32 | 97,34375 % | 0,0392 |
| 128 | 99,80469 % | 0,1128 |
| 512 | 99,99512 % | 0,3203 |
| 1024 | 100 % | 0,5260 |

Construction médiane 2,458 s ; RSS maximal 38,813 Mio ; oracle exact
optimisé environ 0,227 ms **moyens**, différent d'un p95. Ce N est inférieur
au fallback de production à 10k, donc le résultat est un test du backend
brut, pas la latence Finder. [Résultats bruts](results/ror.json).

## Finder complet

[python_ann.py](../../performance/python_ann.py) utilise la wheel release
installée hors dépôt, modèle local, cache d'embeddings désactivé, seed16.
Le dataset synthétique contient ID/tenant, nom aléatoire de 20 caractères
et description de 32 mots ASCII du vocabulaire du modèle. Query : ID neuf,
nom avec un caractère modifié, description identique. Une positive connue,
seuil0,9 figé ; aucun profil métier calibré. Génération des inputs hors timers.
Recherche complète : nouvelle inférence de query, récupération, scoring,
conversion Python. Une warmup puis 1 000 queries, cinq processus par config,
N=1k/10k/100k, 1/4 threads, modes dense/HNSW ; 100 tenants à 100k en complément.

Le rappel de la positive connue et les autres records retournés sont
comptés séparément ; dix queries sont comparées à l'oracle exhaustif.
Ce fixture favorable, notamment sa voie lexicale, ne remplace pas recall@k
vectoriel ni une mesure de doublons métiers. Upsert sans réencodage et remove sont mesurés après les queries.
L'option `--snapshot` mesure aussi save/load et reconstruction ; les résultats
bruts indiquent les processus qui incluent ces phases. Leur RSS maximal inclut
l'ancien Finder et le nouveau pendant load, les inputs Python et le modèle ;
il ne démontre pas le budget incrémental moteur de 512 Mio.
Les [mesures](../../../docs/performance.md) publient médiane et dispersion.

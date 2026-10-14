# Recherche sémantique approximative

U-016 ajoute un profil explicite ; le défaut reste `retrieval="exact"`.
Ce nom concerne la récupération vectorielle complète, pas une garantie
que les objets désignent la même entité. L'oracle `exhaustive=True` score
chaque record admissible, indépendamment des lanes.

```python
finder = Finder(
    schema=schema,
    retrieval="hnsw",
    ann_ef_search=512,
    ann_max_candidates=1024,
    offline=True,
)
```

Les champs `Text(mode="semantic")` utilisent un graphe HNSW par champ et
partition typée à partir de 10 000 slots retenus dans la partition.
Les partitions plus petites utilisent le parcours vectoriel complet.
Ce seuil évite des graphes pour les petits tenants ; il ne constitue pas
un choix `auto` ni une garantie de gain à 10 000 records. Les mesures
[U-016](../benchmarks/spikes/ann-backends/README.md) montrent justement des workloads
où le parcours complet reste plus rapide. Les champs hybrid restent lexicaux.
Les partitions réelles sont vérifiées avant la traversal, même en cas de
collision d'empreintes. Sans embeddings ou sans graphes, aucun index ANN
n'est alloué. Import et construction ne chargent aucun modèle.

`ann_ef_search` est un entier de 1 à 8192 ; `ann_max_candidates` de 1 à
100 000. Ils sont validés avant résolution du modèle. Le backend peut
augmenter `ef` jusqu'au nombre de voisins demandés. La recherche demande
initialement au plus 32 vecteurs distincts, puis double tant que le dernier
voisin passe le seuil sémantique, jusqu'au plafond configuré. Les vecteurs
identiques partagent un nœud mais conservent leurs slots distincts.
Le plafond s'applique aux records de chaque lane ANN après exclusion des
versions inactives et de l'ID de la requête. La voie exacte complète,
les voies lexicales et les valeurs exactes rejoignent toujours l'union ;
elles peuvent donc porter le nombre total de candidats au-delà du plafond ANN.
Le scoring et les vetoes restent ceux du cœur Rust. Un veto peut réduire
les résultats d'un top-k ; aucune complétude sous contraintes n'est promise.

Dès qu'un graphe est utilisé, `Report.search_profile == "approximate"`
et un avertissement signale les omissions possibles, **même si
`candidate_truncated` est faux**. Ce dernier drapeau signale une saturation
observée, distincte de l'approximation intrinsèque et de `output_truncated`.
`total_matches` ne compte que les matches effectivement trouvés.
`exhaustive=True` ignore les graphes et conserve ses limites explicites.
La génération, l'empreinte de configuration, les paramètres et l'identité
versionnée du backend rendent le profil observable.

Le garde-fou `find(max_semantic_comparisons=...)` demeure obligatoire.
Un contrôle préalable par partitions refuse le minimum dense (queries ×
slots × champs semantic) avant encodage et construction ANN ; le contrôle
final ajoute les niveaux et retries observés après construction. Ce budget
ne plafonne pas le coût de résolution/préparation/construction des index.
Un find temporaire exhaustif ne construit aucun graphe ANN.
Il compte conservativement les queries actives × les entrées du parcours
complet ou un majorant HNSW couvrant tous les nœuds, niveaux, vérifications
cosinus et retries adaptatifs. Le maximum avec les slots retenus préserve
le budget dense. `limit` ne borne pas le calcul. Une augmentation explicite
du budget autorise les gros rapports ; ANN ne contourne pas cette protection.

## Mutations et snapshots

`add`/`upsert` ajoutent les nouveaux vecteurs aux graphes concernés ;
`remove` masque les anciens slots dans toutes les lanes. Des nœuds obsolètes
peuvent consommer des voisins et réduire le rappel avant compaction, mais
ne peuvent produire de record supprimé. La compaction U-015 reconstruit les
graphes depuis la vue active sans inférence, avec coexistence temporaire
des anciens et nouveaux index.

Les snapshots conservent les vecteurs et une identité `ann` optionnelle
dans le manifest v1 : backend/kernel, métrique, seed, paramètres de
construction, seuil de partition et options de recherche. Ils ne sérialisent
pas le graphe de la dépendance. Le chargement valide cette identité puis
reconstruit depuis les vecteurs, sans réencoder les records. Une incompatibilité
est une erreur typée. Les snapshots du défaut dense gardent leur manifest
sans cette propriété et sont compatibles avec les lecteurs antérieurs.
Un ancien lecteur refuse un snapshot ANN qu'il ne connaît pas.

La reconstruction après mutations ou restauration peut modifier le rappel
approximatif, la topologie et le recours au fallback petite partition,
sans changer la génération logique. Les résultats exhaustifs restent stables.
Les graphes utilisent un seed fixe, mais on ne promet pas un résultat ANN
bit-à-bit entre plateformes ou dispositions physiques différentes.

## Coût

Le backend est `fast-hnsw` 1.0.1, M=16, efConstruction=200, pruning
`Heuristic`, seed=16, distance euclidienne réelle avec accumulation portable
sur huit lanes. Le score final utilise le cosinus du cœur. Les vecteurs
validés sont unitaires. Aucune instruction CPU particulière, code unsafe
Univo, worker pool supplémentaire ou persistance de la dépendance n'est requis.
Une recherche isolée est séquentielle ; `threads` borne l'inférence et les requêtes parallèles de `find` et `group` ([ADR 0018](adr/0018-parallel-all-pairs.md)). La construction des graphes reste séquentielle.

Le graphe copie les vecteurs distincts et stocke liens, hashes et slots.
`stats().ann_nodes` compte les vecteurs distincts, tombstones compris ;
`ann_bytes` estime grossièrement la mémoire retenue des graphes, comprise
dans `index_bytes`. Ce n'est pas un plafond RAM : modèle, records sélectionnés,
inputs Python, allocation temporaire des visited sets et reconstruction sont
exclus de cette estimation. Le budget total incrémental de 512 Mio demeure
à démontrer. Les mesures de wheel et RSS complet sont publiées séparément.
Voir [ADR 0015](adr/0015-explicit-partition-ann.md).

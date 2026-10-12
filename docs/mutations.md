# Mutations par ID

`upsert(records)` remplace les IDs présents et insère les nouveaux ; il renvoie
les IDs dans l'ordre du lot. `remove(ids)` masque immédiatement les IDs actifs
et renvoie ceux absents au début du lot, dans l'ordre d'entrée. Un ID répété
présent est supprimé une seule fois ; les répétitions d'un ID absent sont
conservées dans la réponse. Les IDs restent typés : `1` et `"1"` sont distincts,
sans normalisation des chaînes.

Les deux méthodes nécessitent `Schema(id_field=...)`. Les IDs implicites des
schémas de chaînes restent réservés à `add`. Les lots et générateurs sont
bornés à `max_records` éléments ; cette capacité limite les records **actifs**,
pas le RSS. Un lot qui dépasse le budget, contient un ID invalide ou échoue
pendant l'encodage ne publie aucun remplacement ni suppression. Deux IDs
identiques dans un lot d'upsert sont refusés ; l'ordre ne choisit pas un gagnant.

```python
from univo import Finder, Schema, Text

finder = Finder(schema=Schema(id_field="id", fields={"name": Text()}))
finder.add([{"id": "a", "name": "Amazon"}])
finder.upsert([{"id": "a", "name": "Microsoft"}])
assert finder.remove(["a", "missing"]) == ["missing"]
```

Le `find(records)` Python reste temporaire : il compare le lot fourni et ne lit
pas l’index persistant. Les mutations changent les données utilisées par search.

## Préparation, générations et recherche

Conversion, validation des IDs/capacité et encodage précèdent la publication.
Le moteur Rust réutilise les vecteurs des champs sémantiques dont l'état
normalisé est inchangé, même avec le cache désactivé ou après load. Cela inclut
les résultats sans tokens connus. Les autres champs modifiés ne déclenchent
pas cet encodage. Les explications utilisent les nouvelles étapes de
normalisation, même lorsque le texte normalisé et le vecteur sont identiques.
La compatibilité du modèle et du schéma est contrôlée avant la copie des
vecteurs existants, puis la capacité est vérifiée avec les lignes réutilisées.

Une génération augmente une seule fois par `add` ou `upsert` non vide réussi,
et par `remove` qui supprime au moins un ID actif. Upsert identique compte comme
une publication ; lot vide et suppression d'IDs tous absents ne l'incrémentent
pas. Une génération épuisée est refusée avant mutation. Rapports et snapshots
portent la génération courante. Les appels Python sur une même instance sont
sérialisés ; le runtime Python est détaché pendant les calculs Rust. L'API Rust
requiert `&mut SearchIndex` pour les mutations ; le caller fournit encore la
génération à `save`, comme dans le contrat U-014.

Chaque version remplacée ou supprimée devient un tombstone. Les nouvelles
versions sont ajoutées en fin de store ; l’ordre de la vue active suit donc
ces publications, tandis que le tri de search reste par score puis ID. Exact, lexical,
valeurs, vecteurs, oracle exhaustif et rapports filtrent les slots inactifs,
y compris après changement de partition ou de veto. Le budget exhaustif compte les paires actives. Le garde-fou du rapport
sémantique compte champs semantic × somme, par partition, de
`queries actives * slots retenus`, y compris les tombstones parcourus ; il
reste conservateur même si ces slots sont filtrés avant le calcul vectoriel. Les limites lexicales et de
valeurs comptent les entrées réellement lues, tombstones compris : une
troncature reste explicite et peut perdre des candidats. Aucun ANN n'est encore
intégré.

## Espace retenu et compaction

Avant un nouvel add/upsert, le moteur reconstruit les lanes si les tombstones
atteignent `max(64, ceil(active / 2))`, ou si le lot dépasserait deux fois la
capacité en slots retenus. Il reconstruit à partir des données préparées et
vecteurs existants, sans inférence. Le nouvel index est construit avant de
remplacer l'ancien. Les slots internes peuvent changer ; les IDs externes
restent stables. Les publications restent au plus à `2 * max_records` slots
retenus, sous la limite u32 du moteur.

`stats()` expose `records` actifs, `retained_slots` et `tombstones` ;
`index_bytes` inclut masque actif, IDs, postings et vecteurs retenus, mais exclut
les records et le modèle. La compaction change ces compteurs physiques sans
incrémenter la génération logique ; elle peut précéder un échec d'encodage.
Les entrées compatibles déjà produites dans le cache peuvent également rester
après un échec, comme pour add. Aucun état logique partiel n'est visible.

La reconstruction est amortie mais peut produire un pic de latence et de
mémoire : ancien index et nouvel index coexistent temporairement. Suppression
seule ne libère pas immédiatement les données ; le cache borné peut aussi
retenir leurs textes. Aucune promesse d'effacement sécurisé ni de plafond RSS
n'est faite. Le budget de surcoût mémoire de 512 Mio à 100k reste à démontrer.

## Snapshots

Save écrit seulement les records actifs et leurs lignes vectorielles encore
référencées, avec remapping des lignes et écriture des floats en streaming.
Le format v1 reste compatible : son contenu est une vue dense des records
actifs ; aucune table de tombstones n'est nécessaire. Load restaure cette vue
avec sa génération, sans inférence ; les mutations peuvent reprendre.
Les modifications en mémoire ne deviennent durables qu'après save réussi.
Les anciennes générations sur disque conservent leur contenu, y compris les
données supprimées ensuite en mémoire : voir [persistance](persistence.md).

Choix et alternatives : [ADR 0014](adr/0014-id-mutations.md).

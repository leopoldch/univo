# Qualité, jeux de données et tests

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 20. Dataset de validation et protocole qualité

Construire avant d’optimiser : un petit corpus déterministe de cas difficiles, puis un benchmark public plus large avec labels et licences vérifiés.

Corpus initial proposé : au moins 200 cas ciblés couvrant les branches critiques. Suite qualité visée avant 1.0 : plusieurs milliers de paires annotées, incluant au moins 500 vrais doublons et 1 000 négatifs difficiles, à ajuster selon disponibilité. Doubler l’annotation d’un sous-ensemble ambigu et documenter les désaccords.

Familles de tests :

| Famille | Exemple / attente |
| --- | --- |
| Faute de frappe | Amazon / Amiazon : candidat possible |
| Compétiteurs sémantiques | Amazon / Microsoft : pas même entreprise par simple proximité |
| Description reformulée | formulations différentes d’un même objet |
| Homonymes | mêmes noms, domaines/identifiants incompatibles |
| Négation | enabled / disabled ; with / without |
| Paramètres critiques | retries 0 / 3 ; timeout 1 s / 1000 ms selon schéma |
| Versions et environnements | v1 / v2 ; prod / staging |
| Valeurs manquantes | absent, null, vide, zéro, faux |
| Unicode | accents, caractères combinés, alphabets distincts |
| Ordre | dictionnaire réordonné ; liste réordonnée non équivalente par défaut |
| Tenants | aucune correspondance inter-partition |
| Densité | centaines de doublons proches, saturation du top-k |
| Chaînes courtes | un ou deux caractères, codes, acronymes |
| Cas adverses | objet trop grand, encodage vide, vecteur non fini |

Séparer train/validation/test par entité ou source, pas aléatoirement par paire : sinon les mêmes entreprises fuient entre les ensembles. Toutes les variantes synthétiques d’une entité restent dans un même split. Garder un test hors domaine et un test FR/EN séparés.

Mesures : précision TP/(TP+FP), rappel TP/(TP+FN), F1, courbe précision-rappel, rappel des candidats avant reranking et résultats par famille. Mesurer les valeurs manquantes, la couverture et le taux de revue.

Comparer aux baselines : exact normalisé, lexical seul, embeddings seuls, SemHash ; Dedupe/Splink lorsque le protocole et les données structurées rendent la comparaison pertinente. Si une baseline nécessite des labels, les compter et les lui fournir équitablement.

Gates qualité proposés sur le benchmark nommé, pas garanties universelles : précision candidats ≥ 98 % et rappel ≥ 90 % ; rappel de génération des candidats ≥ 99 % ; zéro violation des veto/partitions sur la suite déterministe. Pour les profils autorisant une décision automatique, viser ≥ 99,5 % de précision et publier intervalle de confiance et support. Un petit test sans erreur ne suffit pas à démontrer ce dernier chiffre.

Si ces gates échouent : publier une alpha expérimentale ou réduire les profils supportés. Ne pas simplement baisser le seuil pour afficher davantage de doublons.

## 22. Tests de correction et CI

### Rust

Tests unitaires des normaliseurs, comparateurs et règles ; propriétés d’idempotence, symétrie du score pairwise, stabilité du tri, distinction des types ; tests différentiels exact vs ANN ; fuzzing ciblé des formats persistants, chemins de champs et limites.

Tests de collisions simulées sur l’empreinte exacte : égalité du hash ne doit pas suffire. Tests de sérialisation : round-trip IDs, données sélectionnées et scores à tolérance documentée.

### Python et FFI

Tester types acceptés/rejetés, exceptions, références libérées, absence de mutation des entrées, appels multithreads, interruption des longs lots à des frontières sûres. Les longues tâches Rust se détachent de Python ; ne pas invoquer de callback Python par paire candidate.

### Persistance et concurrence

Pannes injectées aux étapes de save ; répertoire en lecture seule ; disque plein ; snapshot incomplet ou corrompu ; mauvais modèle ; writer concurrent ; lecteur pendant une mutation ; upsert/remove en présence d’ANN. Vérifier absence d’IDs supprimés et absence de mélange entre générations.

### Offline et packaging

Exécuter import/load/search avec réseau interdit et modèle préchargé. Tester le chemin avec modèle absent : erreur immédiate, aucun essai HTTP. Installation wheel sans compilateur, hors workspace, sur chaque plateforme annoncée.

### Jobs CI

Sur PR : format Rust, clippy, tests unitaires, pytest, type checking de l’API, corpus critique, build wheel pilote, audit dépendances/licences. Nightly : fuzzing borné, qualité complète, plateforme élargie. Release : toutes les wheels, tests hors ligne, benchmarks dédiés, artefacts source et notices.

Éviter des tests qui répètent simplement les mêmes formules que l’implémentation ; privilégier oracles indépendants et invariants métier.

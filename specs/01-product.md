# Produit, périmètre et décisions

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 1. Objectif et contraintes fondatrices

Construire une bibliothèque locale de recherche de doublons d’objets, avec un cœur Rust réutilisable et une interface Python simple. L’utilisateur fournit ses données et les champs pertinents ; il ne manipule ni vecteurs, ni tokenizer, ni index vectoriel.

Promesse visée : **Fast, local duplicate detection for your data.**

Contraintes issues de la discussion :

1. Très rapide à l’usage, avec démarrage et consommation mémoire maîtrisés.
2. Fiable : distinguer la similarité de l’identité et rendre les incertitudes visibles.
3. Peu de dépendances, évaluées transitivement et en taille installée.
4. Aucune API d’inférence distante ; aucune donnée métier envoyée au fournisseur du modèle.
5. Installation Python standard, sans toolchain Rust sur les plateformes fournies en wheel.
6. Modèle compact par défaut, téléchargé automatiquement puis mis en cache ; chemin local et mode hors ligne disponibles.
7. Cœur utilisable directement en Rust ; Java est une extension ultérieure.
8. Interface intuitive, avec configuration avancée possible sans l’imposer au premier usage.

Le projet ne promet pas de prouver l’équivalence arbitraire de n’importe quel objet. Il applique une politique explicite aux champs choisis. Reconnaître une entreprise, rapprocher des descriptions et vérifier deux configurations nécessitent des politiques différentes.

## 2. Positionnement et preuve de valeur

La génération d’embeddings locaux et la déduplication sémantique existent déjà. SemHash est un concurrent direct ; Dedupe et Splink sont des références pour les données structurées. Ne pas vendre la seule présence de Rust ou d’embeddings comme une innovation.

Hypothèse de différenciation : **un moteur intégrable dans une application, combinant signaux structurés et sémantiques, avec recherche incrémentale, index local et API courte**.

Trois cas d’usage de référence :

| Cas | Données | Résultat attendu |
| --- | --- | --- |
| Catalogue d’organisations | nom, domaine, description, tenant | Amazon / Amiazon proposés comme candidats selon les autres champs |
| Objets métier | noms courts, identifiants, descriptions | proximité utile sans fusionner des identifiants incompatibles |
| Configurations | environnement, paramètres typés, description | descriptions proches, mais contradictions de paramètres signalées |

Les versions initiales détectent et expliquent. Elles ne suppriment rien dans une DB externe. Le moteur n’est ni un ORM ni une base de données distribuée.

Avant de généraliser : exécuter le même problème avec SemHash et une baseline lexicale. Mesurer qualité, effort d’intégration, temps et mémoire. Si Univo ne simplifie rien ou n’améliore aucun compromis concret, revoir le périmètre avant les bindings Java.

## 3. Périmètre des versions

| Version | Inclus | Exclus |
| --- | --- | --- |
| Prototype interne | corpus annoté, exact + lexical, comparaison de backends, extension Python minimale | publication et promesse de performance |
| 0.1 alpha | listes de chaînes / dictionnaires, schéma, modèle local, recherche exacte des candidats, scores expliqués, add/search/find, wheels pilotes | ANN, persistance, fusion, accès multiprocessus |
| 0.2 beta | sauvegarde/chargement, upsert/remove, partitions, index ANN optionnel, cache borné, rapports reproductibles | garantie distribuée, connecteurs DB automatiques |
| 0.3 release candidate | groupes prudents optionnels, check_or_add local, tests de concurrence, matrice de wheels complète, documentation | résolution universelle d’identité |
| 1.0 | contrats stabilisés, gates qualité/performance atteints sur les profils supportés, migration documentée des formats ; périmètre fixé par l’[ADR 0024](../docs/adr/0024-version-1-scope.md) | toute fonction non validée par les gates, dont la décision automatique |

Le mode exact doit rester disponible après l’ajout d’ANN. La 0.1 limite volontairement la taille des tâches exhaustives ; elle refuse les tâches dépassant un budget annoncé plutôt que de devenir silencieusement approximative.

## 4. Décisions acquises et décisions à mesurer

| Sujet | Décision |
| --- | --- |
| Langage cœur | Rust |
| Première interface | Python via PyO3 et maturin |
| Traitement des données | local, sans API d’inférence |
| Types initiaux | chaînes et dictionnaires à valeurs supportées |
| Représentation vectorielle interne | f32 contiguë et normalisée, sauf backend documenté autrement |
| Exactitude | aucune suppression implicite ; correspondances expliquées |
| Modèle par défaut | modèle statique multilingue (50 langues) `univo-models/static-similarity-mrl-multilingual-int8-128`, int8, 128 dimensions, 15 Mo, changeable par une chaîne ([ADR 0007](../docs/adr/0007-default-embedding-model.md), [ADR 0021](../docs/adr/0021-multilingual-default-model.md)) |
| Index ANN | à choisir après benchmark de rappel, mémoire, licence et mutations |
| Stockage persistant | snapshots immuables versionnés initialement |
| Licence du projet | MIT, décidée dans l’[ADR 0001](../docs/adr/0001-project-name-and-license.md) ; vérifier la compatibilité de chaque dépendance et modèle |
| Nom | `univo` (PyPI) et `univo-*` (crates.io), libres au 2026-10-07 selon l’[ADR 0001](../docs/adr/0001-project-name-and-license.md) ; réservation à la première publication |

Ne pas figer prématurément un modèle, une dimension, une crate ANN ou un seuil global. En revanche, chaque release doit fixer et exposer les choix effectivement retenus.

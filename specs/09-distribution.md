# Dépendances, packaging, documentation et release

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 19. Dépendances et distribution

Budget de départ : zéro dépendance Python tierce obligatoire si l’extension et la bibliothèque standard suffisent. C’est un objectif à vérifier, pas une obligation justifiant une réimplémentation risquée. NumPy, pandas, torch et transformers ne doivent pas être imposés par l’API de base.

Familles de dépendances Rust envisagées : sérialisation, erreurs typées, normalisation Unicode, hash, tokenizer/backend choisi, PyO3 pour les bindings ; HTTP uniquement dans le module de téléchargement. Chaque ajout doit documenter utilité, licence, features, coût transitif et effet sur les wheels.

Désactiver les features inutiles après vérification. Ne pas exiger tokio pour un téléchargement bloquant unique si une solution plus simple convient. Parallélisme explicite seulement si le benchmark justifie son coût.

PyO3 + maturin gèrent l’extension Python et la construction des wheels ; les exigences natives et tags de compatibilité doivent être vérifiés. [S3](11-references.md)

Matrice proposée pour la 1.0 :

| Plateforme | Architecture | Niveau |
| --- | --- | --- |
| Linux glibc | x86_64 | obligatoire |
| Linux glibc | aarch64 | obligatoire |
| macOS | arm64 | obligatoire |
| macOS | x86_64 | obligatoire |
| Windows | x86_64 | obligatoire |
| Windows | arm64 | obligatoire |
| Linux musl, PyPy, Python free-threaded | divers | différés jusqu’à tests dédiés |

Décision du mainteneur du 2026-10-07 : amd64 et arm64 sont pris en charge à égalité. Chaque wheel est construite et testée en CI sur un runner natif de son architecture, avec vérification de l’architecture de l’interpréteur ([ADR 0002](../docs/adr/0002-python-abi-and-wheel-strategy.md)).

Proposition : CPython 3.11 à 3.14, à confirmer au démarrage sur les versions effectivement disponibles et dépendances choisies. Évaluer abi3 pour réduire le nombre de wheels ; ne pas présumer que cela couvre les variantes free-threaded ou tous les backends.

Compiler avec un niveau CPU portable ; pas de `target-cpu=native` dans les wheels publiques. Optimisations SIMD avec détection d’instructions à l’exécution et fallback validé.

En CI : installer la wheel dans un environnement propre sans Rust, hors dépôt source ; exécuter import, encodage et recherche. Tester aussi compilation depuis sdist sur une plateforme représentative. La sdist n’a pas la même promesse d’installation sans compilateur.

## 26. Documentation à livrer

README court avec installation, exemple testé, absence d’API distante, plateformes supportées, téléchargement initial et lien vers limites. Pas de vitesse annoncée sans référence au benchmark.

Pages nécessaires : définition de doublon ; choix des champs ; scores vs probabilités ; règles exactes et veto ; offline et préchargement ; cache ; persistance/durabilité ; migration ; paramètres de recherche et rappel ; limites de concurrence ; mesures et comparaison concurrentielle.

ADR minimum : modèle choisi, représentation des champs, dépendance ANN, politique de scoring, format persistant, ABI Python et stratégie de wheel. Chaque ADR contient alternatives, mesures, décision et condition de réexamen.

Exemples exécutables : liste de noms ; organisations ; configuration avec environnement discriminant ; ajout incrémental ; application hors ligne. Les données synthétiques doivent être identifiées comme telles.

## 27. Checklist de release 1.0

- [ ] Nom de package et crate confirmé, licence et notices présentes.
- [ ] API Python installable par pip sur toutes les plateformes annoncées.
- [ ] Aucun Rust/compilateur nécessaire pour les wheels supportées.
- [ ] Modèle par défaut fixé par révision/checksums et documenté par langue.
- [ ] Mode offline testé avec réseau réellement interdit.
- [ ] Aucun envoi de contenu, télémétrie ou exécution de code distant par défaut.
- [ ] Précision/rappel publiés par benchmark et par famille difficile.
- [ ] Rappel de génération de candidats et approximation ANN documentés.
- [ ] Latences froides/chaudes, débit, RSS et tailles publiés sur matériel nommé.
- [ ] Aucun champ critique ignoré silencieusement ; limites et troncatures observables.
- [ ] Cache borné et invalidation testée lors des changements de modèle/schéma.
- [ ] Sauvegarde cohérente, corruption détectée, incompatibilités refusées clairement.
- [ ] upsert/remove ne laissent pas de résultats obsolètes.
- [ ] API et cœur Rust partagent les mêmes politiques et tests de référence.
- [ ] Aucune suppression DB ni fusion automatique implicite.
- [ ] Documentation de la portée locale de check_or_add.
- [x] Rapport de différenciation comparé à SemHash et aux baselines lexicales ([U-003](../benchmarks/comparison/README.md)).

U-016 ajoute `fast-hnsw=1.0.1` au moteur ; aucune dépendance Python ou
bibliothèque native C/C++, aucun client réseau. Sept versions de packages
supplémentaires, 115 noms liés contre 112. Wheel macOS arm64 : 1,789 Mio
contre 1,762 Mio U-015. Licence MIT déclarée par Cargo/README, notices à
traiter avec les autres dépendances en U-019 ; la petite distribution locale
ne remplace pas les validations des six plateformes CI.
[Registre](../docs/dependencies.md), [ADR 0015](../docs/adr/0015-explicit-partition-ann.md).

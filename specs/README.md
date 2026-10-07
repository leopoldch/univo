# Spécifications Univo

Ces fichiers sont la référence de construction d’Univo : cœur Rust, interface Python, détection locale des doublons et intégration simple. Ils répartissent intégralement le plan de construction du 6 octobre 2026 en contrats thématiques.

**État du projet : spécification initiale.** Aucun moteur, benchmark ou package publié n’est fourni par cette PR. Les numéros des 30 sections du plan sont conservés pour la traçabilité ; l’ordre des fichiers sert de parcours de lecture.

## Lecture et portée des instructions

1. Lire les [instructions du dépôt](../AGENTS.md) et le [périmètre](01-product.md).
2. Lire le contrat du composant concerné avant de coder.
3. Choisir la tâche dans le [backlog](10-roadmap.md), respecter ses dépendances et vérifier son critère de fin.
4. Accompagner toute évolution de contrat d’une mise à jour de la spécification et, pour un choix architectural, d’un ADR avec preuves et compromis.

| Fichier | Sections du plan | Contenu |
| --- | --- | --- |
| [Produit, périmètre et décisions](01-product.md) | 1, 2, 3, 4 | Produit, périmètre et décisions |
| [Contrats de l’API Python et résultats](02-python-api.md) | 5, 6, 7 | Contrats de l’API Python et résultats |
| [Données, comparaison, recherche et groupes](03-matching.md) | 8, 9, 10, 12, 17 | Données, comparaison, recherche et groupes |
| [Embeddings, cache et fonctionnement hors ligne](04-models-and-cache.md) | 11, 13 | Embeddings, cache et fonctionnement hors ligne |
| [Architecture Rust et concurrence](05-rust-architecture.md) | 18, 16 | Architecture Rust et concurrence |
| [Snapshots, formats et cohérence](06-persistence.md) | 15 | Snapshots, formats et cohérence |
| [Mémoire, performances et budgets](07-performance.md) | 14, 21 | Mémoire, performances et budgets |
| [Qualité, jeux de données et tests](08-validation.md) | 20, 22 | Qualité, jeux de données et tests |
| [Dépendances, packaging, documentation et release](09-distribution.md) | 19, 26, 27 | Dépendances, packaging, documentation et release |
| [Backlog, jalons, risques et extensions](10-roadmap.md) | 23, 24, 25, 28, 29 | Backlog, jalons, risques et extensions |
| [Sources techniques](11-references.md) | 30 | Sources techniques |

## Comment interpréter les décisions

- **Contraintes acquises** : cœur Rust, première interface Python, inférence locale, aucune suppression implicite, faible coût de dépendances, installation par wheels sur les plateformes annoncées.
- **Contrats initiaux** : API, schémas, erreurs, traitement des données et sémantique des mutations. L’implémentation suit ces contrats ; tout changement doit être explicite et cohérent dans les fichiers concernés.
- **Hypothèses à valider** : modèle par défaut, backend ANN, dimension, profils et seuils. Les spikes et benchmarks tranchent ces choix ; ne pas les présenter comme déjà acquis.
- **Objectifs proposés** : latence, RAM, taille, précision et rappel. Ils deviennent des gates de release sur un matériel et un dataset nommés après validation. Un échec doit être signalé ou faire l’objet d’une décision documentée ; ne pas changer silencieusement le périmètre de mesure.
- **Fonctions futures** : persistance, mutations, ANN, groupes et check_or_add suivent les versions du périmètre. Les extraits d’API avancée ne rendent pas ces fonctions obligatoires dès la 0.1.

Les décisions prises par le mainteneur pour une tâche priment sur ce plan initial. Les répercuter dans les contrats concernés pour garder une seule source de vérité. Les sources techniques servent à documenter des possibilités ; elles ne remplacent pas les exigences du projet.

## Clarifications transversales

- Une correspondance exacte porte sur les champs sélectionnés et normalisés, pas nécessairement sur l’identité réelle de l’objet.
- Une similarité ou un score élevé ne constitue ni une probabilité calibrée ni une autorisation de supprimer un objet.
- Une recherche exhaustive couvre les paires admissibles définies par le schéma et les partitions. Une recherche bornée ou ANN ne garantit pas l’absence de doublons manqués.
- Une opération atomique locale ne couvre pas une DB externe ni plusieurs services indépendants.
- L’identifiant d’un modèle, sa révision, son tokenizer et ses paramètres font partie du contrat de compatibilité des vecteurs.
- Le nom de distribution `univo` était libre sur PyPI et crates.io au 2026-10-07 ([ADR 0001](../docs/adr/0001-project-name-and-license.md)) ; il n’est réservé qu’à la première publication et PyPI peut encore refuser un nom jugé trop proche d’un projet existant.

## Traçabilité

Toutes les sections 1 à 30 du plan sont présentes une fois dans les fichiers thématiques. Il n’existe pas de seconde copie monolithique à maintenir. Les tâches U-001 à U-021 et la checklist 1.0 sont conservées, sans les marquer comme réalisées.

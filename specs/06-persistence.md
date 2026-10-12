# Snapshots, formats et cohérence

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 15. Persistance et cohérence

La 0.2 utilise des snapshots explicites. Les modifications en mémoire ne sont durables qu’après `save()` réussi. Pas de WAL ni de promesse de récupération de chaque insertion initialement.

Contenu logique :

- `manifest.json` : version de format, génération, schéma, profils, modèle, checksums, nombre d’objets, dimensions.
- Table des IDs et champs sélectionnés.
- Matrices vectorielles avec header typé, endianness et limites.
- Index secondaires ou informations suffisantes pour les reconstruire.
- Marqueur de snapshot complet.

Les textes sélectionnés peuvent être sensibles ; les vecteurs ne sont pas une anonymisation. Ne pas sauvegarder les champs non utilisés. Permissions fichiers adaptées à l’environnement ; chiffrement au repos confié au système ou à une extension ultérieure, sans prétendre qu’il existe.

Écriture dans une nouvelle génération immuable, vérification, puis remplacement atomique d’un pointeur `CURRENT` selon les garanties de la plateforme. Flush/sync nécessaires pour le contrat de durabilité documenté. Tests de panne avant et après chaque étape. Un chargement ne voit que l’ancien snapshot complet ou le nouveau, jamais un mélange.

Lecteurs ouvrent une génération stable. Un seul writer par chemin avec verrou explicite ; deuxième writer refusé. Les partages réseau non testés ne sont pas annoncés comme supportés.

Changement de modèle, schéma ou normaliseur : refus de réutiliser des vecteurs incompatibles, avec message indiquant la nécessité d’une reconstruction. Changement de seuil seul : reranking/recherche possible sans réencodage, mais ne pas prétendre récupérer des candidats précédemment tronqués à partir d’un rapport incomplet.

Le format est borné et validé : tailles, offsets, dimensions, nombres d’enregistrements, checksums, versions. Pas de désérialisation aveugle de structures internes de dépendances sans version et stratégie de migration.

## U-014 : format implémenté

Le [contrat version 1](../docs/persistence.md) implémente les snapshots explicites avec générations immuables, verrou OS non bloquant, marqueur complet et pointeur CURRENT remplacé dans le même répertoire. Données binaires little-endian typées, manifest JSON borné, SHA-256 des octets décodés, validation des formes/valeurs et reconstruction des index sans réencodage des vecteurs. Les tests couvrent erreurs injectées et arrêt du processus avant/après publication. Le cache d'embeddings n'est pas persisté. Les anciennes générations restent conservées pour les lecteurs ; le nettoyage automatique est différé.

Budget explicite : 512 Mio par génération chargée/sauvegardée, manifest ≤ 1 Mio, limite de chargement de 100 000 records par défaut ; ces budgets ne bornent pas le RSS. Unix synchronise aussi les répertoires. Sous Windows, la bibliothèque standard synchronise les fichiers mais n'expose pas de sync de répertoire : aucune garantie de survie des entrées de répertoire après coupure électrique n'est annoncée. Aucun test de coupure électrique réelle ou de partage réseau n'est revendiqué. Voir les alternatives et conditions de réexamen dans l'[ADR 0013](../docs/adr/0013-versioned-snapshots.md).

U-016 ajoute une propriété manifest v1 `ann` optionnelle avec identité
backend/kernel, métrique, seed, paramètres de construction et recherche.
Absente pour le défaut dense, dont les snapshots gardent leur compatibilité.
Les anciens lecteurs refusent les manifests ANN inconnus. Les graphes ne
sont pas persistés : reconstruits depuis les vecteurs validés après contrôle
de compatibilité. Reconstruction pouvant modifier le rappel approximatif
sans changer la génération logique ; oracle exhaustif stable.
Voir [contrat ANN](../docs/ann.md).

# Embeddings, cache et fonctionnement hors ligne

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 11. Choix du modèle d’embeddings

Évaluer trois candidats architecturaux, pas dix modèles au hasard :

| Candidat | Intérêt | Risque à mesurer |
| --- | --- | --- |
| Exact + lexical, sans modèle | référence légère et rapide | manque les reformulations |
| Embeddings statiques compacts, type Model2Vec | inférence Rust existante ; candidat prioritaire pour le poids/vitesse | ordre, négation, détails numériques |
| Petit encodeur contextuel via runtime local | compréhension potentiellement meilleure de certaines formulations | runtime, poids, démarrage et dépendances |

Model2Vec dispose d’une implémentation Rust publique. C’est une piste d’intégration, pas une preuve de qualité sur les objets Univo. [S1](11-references.md)

Le spike choisit un modèle public par famille, compatible avec la licence de distribution. Pour chaque candidat enregistrer : repo, révision immuable, fichiers/checksums, licence, langues annoncées et testées, dimension, longueur maximale, pooling, normalisation et taille réelle téléchargée.

Critères de choix, dans cet ordre : qualité minimale du cas cible ; mémoire/taille ; latence ; dépendances et support plateforme ; maintenance. Une mauvaise précision ne se compense pas par un benchmark de débit impressionnant.

Le défaut doit être explicitement monolingue ou multilingue. Mesurer anglais et français séparément. Ne pas annoncer le multilingue si seul l’anglais est validé.

Pour la 0.1, un vecteur sémantique par champ actif constitue l’oracle de qualité. Mesurer ensuite un vecteur agrégé de retrieval, ou un nombre réduit d’index par champ. Ne pas concaténer tous les champs sans mesurer la perte de rappel.

Interface interne proposée :

```rust
pub trait Embedder: Send + Sync {
    fn descriptor(&self) -> &ModelDescriptor;
    fn encode_batch(&self, texts: &[&str]) -> Result<DenseMatrix>;
}
```

Dimension, limites et empreinte proviennent du descriptor. Aucun accès Python dans ce chemin pour le backend par défaut.

## 13. Cache et gestion des modèles

### Cache d’embeddings

Clé : hash du contenu normalisé + type de champ + version de normalisation + empreinte complète du modèle et du tokenizer + paramètres d’encodage.

Un changement de description n’invalide pas les vecteurs des autres objets. Un changement de modèle invalide tous les vecteurs concernés. Une modification d’un champ non sélectionné ne déclenche pas de réencodage.

Cache en mémoire borné en octets, avec éviction documentée ; cache disque optionnel ensuite. Compteurs hits/misses et octets estimés. Ne pas accumuler des références Python pour conserver ce cache.

### Téléchargement

- Aucun réseau à l’import ni à la construction du Finder.
- Téléchargement au premier besoin ; révision et checksums fixés par le profil de release.
- Fichier temporaire, vérification, puis publication atomique dans le cache.
- Verrou par modèle pour éviter deux téléchargements concurrents.
- Délai maximal, erreurs explicites, pas de boucle de retry sans borne.
- Chemin de cache configurable ; fonctionnement sur répertoire en lecture seule si le modèle existe.
- Poids dans un format de données sans exécution de code ; pas de pickle ni de code distant téléchargé.
- Aucun télémétrie par défaut, aucune inclusion du contenu utilisateur dans une requête HTTP ou un message de log.

Prévoir `python -m univo.models download default` et `python -m univo.models inspect` pour le préchargement des images d’entreprise. Ce CLI minimal sert le déploiement, pas un produit séparé.

### Modèles choisis par l’utilisateur

Exigence du mainteneur (2026-10-07) : le modèle se change par une chaîne — dépôt Hugging Face, chemin local ou miroir — selon les règles de [l’API Python](02-python-api.md). Pour tout modèle, y compris hors du défaut :

- seuls les fichiers attendus par le backend sont téléchargés (configuration, poids `safetensors`, tokenizer JSON), avec une taille maximale annoncée ;
- l’intégrité est vérifiée par les empreintes SHA-256 publiées par le hub pour les fichiers LFS, par l’identifiant de blob Git (SHA-1) publié pour les petits fichiers hors LFS, et par les checksums SHA-256 de la release pour le défaut ; les fichiers sont fixés au commit que rapportent les métadonnées ([ADR 0008](../docs/adr/0008-model-download-and-inference-backend.md)) ;
- l’empreinte complète du modèle (dépôt, révision, fichiers, dimension, normalisation, version du backend) entre dans la clé du cache d’embeddings et dans la compatibilité des index ;
- un format non pris en charge produit une erreur `model.unsupported_format` indiquant la conversion possible, par exemple la distillation d’un modèle sentence-transformers au format statique (voir [ADR 0007](../docs/adr/0007-default-embedding-model.md)).

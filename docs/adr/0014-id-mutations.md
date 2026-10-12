# 0014. Mutations par ID et compaction amortie

- Status: Accepted for U-015 implementation; release budgets remain open
- Date: 2026-10-12
- Roadmap: U-015

## Context

U-013 fournit le cache borné ; U-014 fournit la persistance de données préparées
et vecteurs. Les lanes utilisent des postings append-only en Vec, les matrices
partagent des lignes dans chaque lot. Upsert/remove doivent masquer toutes les
anciennes représentations, préserver partitions/vetoes et éviter l'inférence
pour les champs inchangés, sans ajouter de dépendance.

## Decision

Conserver des slots de versions append-only, un masque actif et une table
ID typé → slot actif dans Rust. Valider tout le lot avant publication ; encoder
seulement les nouveaux états sémantiques puis réutiliser les vecteurs inchangés.
Les versions précédentes deviennent inactives dans toutes les lanes. Les
postings inchangés ne sont pas retirés : le nouveau slot doit être accessible
indépendamment de l'ancien. Les représentations lexicales préparées du nouveau
record restent produites lors de la conversion/normalisation bornée.

Reconstruire périodiquement les lanes et matrices sans inference, selon le
[contrat de mutations](../mutations.md). Une mutation courante travaille sur son
lot et ses champs ; la reconstruction amortit le coût global et borne les slots
à deux fois la capacité active. Exposer les tombstones et slots retenus.
Les snapshots écrivent une vue active dense au format v1, sans copie de toute
la matrice pour sérialiser. Les IDs externes sont requis pour muter ; les slots
implicites append-only restent inchangés pour les schémas sans ID.

## Alternatives

- Reconstruire tout l'index à chaque remplacement : simple mais coût O(N) par
  appel, même pour un record ; rejeté pour le chemin courant.
- Retirer chaque ancien slot de chaque Vec : coût proportionnel à la longueur
  des postings denses et rollback multi-lane plus complexe.
- Remplacer les postings par des ensembles/arborescences : facilite le retrait,
  mais augmente l'empreinte et change tous les parcours de recherche ; différé
  jusqu'à des mesures qui justifient cette structure.
- Tombstones sans compaction : appels courts mais mémoire et lectures obsolètes
  non bornées ; rejeté.

## Evidence

L'oracle reconstruit un index neuf et compare les recherches après 320 mutations
avec changements de partitions, champs lexicaux/sémantiques, vetoes et
suppressions ; les rapports candidats sont comparés à l'oracle exhaustif.
Des encodeurs comptants vérifient l'absence d'inférence sur champs inchangés,
résultats sans tokens, compaction et restauration, avec cache désactivé.
Les tests couvrent capacité, IDs répétés/typés, échec d'inférence, snapshots
actifs et génération épuisée. Les mesures release de mutation et les résultats
de validation sont consignés dans [performance](../performance.md).

## Consequences

Pas de dépendance ajoutée ni nouvelle version du format. Les anciens records
restent en mémoire jusqu'à compaction ; les générations historiques sur disque
restent conservées. Le garde-fou sémantique des rapports compte aussi les slots inactifs parcourus
par les queries actives. Les limites de lecture des lanes incluent les entrées
obsolètes effectivement lues et signalent leur troncature. Une compaction peut
bloquer les autres appels de l'instance et augmenter temporairement le RSS.
Elle change la disposition physique sans changer la génération logique.
Le budget mémoire de 512 Mio reste inchangé et non démontré.

## Revisit when

Les pics de reconstruction pénalisent le workload réel, le ratio de tombstones
réduit le rappel sous max_entries, les mesures mémoire excèdent les budgets de
release, ou U-016 introduit un ANN nécessitant une autre stratégie de retrait.

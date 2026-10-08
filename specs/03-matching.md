# Données, comparaison, recherche et groupes

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 8. Modèle de données et normalisation

Types initiaux : chaîne Unicode, booléen, entier borné supporté, float fini, null. Dictionnaires imbriqués accessibles par chemins explicites. Listes seulement avec politique déclarée ; objets Python personnalisés convertis explicitement par l’appelant.

Règles obligatoires :

- Distinguer champ absent, null, chaîne vide, zéro et faux.
- Rejeter NaN et infinis ; traiter les entiers hors plage explicitement.
- Sérialisation canonique typée et avec longueurs : éviter les collisions dues à une simple concaténation.
- Conserver l’ordre des listes par défaut. Le mode ensemble est explicite.
- Normalisation Unicode conservatrice NFC par défaut ; case folding configurable pour champs textuels.
- Ne pas supprimer globalement chiffres, signes moins, ponctuation, accents ou unités.
- `hostname` normalise un nom d’hôte ; il ne ramène pas automatiquement `amazon.fr` et `amazon.com` à la même société.
- Les alias de champs, conversions d’unités et valeurs par défaut viennent d’un schéma explicite.
- Ne pas retirer les suffixes juridiques d’une organisation sans profil choisi et évalué.
- Plafonds proposés : 64 champs sélectionnés, profondeur 16, objet sélectionné 1 MiB ; paramétrables et testés. La profondeur paramétrable est plafonnée à 128 : validation et normalisation sont itératives, et les traitements encore récursifs (empreintes, comparaison, libération) tiennent à cette profondeur dans moins de 64 KiB de pile, y compris en debug. Les limites sont vérifiées par un parcours itératif avant toute copie ou normalisation, identifiant et clés de partition compris.
- Dépassement du budget sémantique : erreur par défaut ; troncature explicitement activée et signalée. Les règles exactes vérifient toujours les valeurs complètes.

Chaque normaliseur possède une version. L’invariant principal est l’idempotence : `normalize(normalize(x)) == normalize(x)`.

## 9. Pipeline de comparaison

Ordre de traitement :

1. Valider les types, IDs, limites et champs requis.
2. Extraire les champs et calculer les clés de partition.
3. Appliquer les normaliseurs déclarés.
4. Calculer une empreinte canonique ; vérifier le contenu en cas de hit pour ne pas dépendre uniquement du hash.
5. Réutiliser les embeddings du cache ou encoder les valeurs nécessaires.
6. Générer les candidats par union des voies exacte, lexicale et sémantique.
7. Appliquer les veto métier et les contraintes de partition.
8. Calculer les caractéristiques par champ sur les candidats.
9. Appliquer le score et la politique versionnés.
10. Trier de manière stable, expliquer et retourner les résultats.

Un échec de la voie lexicale ne doit pas supprimer à lui seul les candidats sémantiques. Une égalité exacte peut court-circuiter la vérification d’existence d’un match, mais pas une recherche demandant toutes les correspondances.

Les contraintes strictes doivent être prises en compte dans la récupération des candidats. Filtrer uniquement après un top-k global peut masquer tous les voisins valides d’un petit tenant.

## 10. Scoring, seuils et contradictions

Caractéristiques initiales : égalité typée, distance d’édition normalisée pour chaînes courtes, recouvrement de tokens ou n-grammes, cosinus sémantique, présence des champs, contradictions explicites. Ajouter des comparateurs numériques/unités après validation d’un besoin réel.

Pour deux embeddings normalisés :

\[
\cos(u,v)=u^Tv,\qquad \|u-v\|_2^2=2-2u^Tv.
\]

Les vecteurs nuls ou non finis sont des erreurs d’encodage ou un signal absent explicitement traité ; ne pas les normaliser silencieusement.

Score initial explicable, par exemple :

\[
S(a,b)=\frac{\sum_{j\in O(a,b)}w_j s_j(a,b)}{\sum_{j\in O(a,b)}w_j},
\qquad
C(a,b)=\frac{\sum_{j\in O(a,b)}w_j}{\sum_j w_j}.
\]

`O` contient les champs informatifs présents des deux côtés ; `C` mesure la couverture. Un champ objet comparé clé par clé ne pèse que pour sa part d’information présente des deux côtés (voir [scoring](../docs/scoring.md)) : des clés présentes d’un seul côté réduisent la couverture. Refuser la décision automatique lorsque la couverture est insuffisante. Si le dénominateur est nul, le score est absent, jamais égal à 1.

La transformation éventuelle du cosinus en score de champ est documentée et versionnée. Les signaux corrélés ne sont pas des preuves indépendantes. Les poids et seuils se règlent sur validation, jamais sur test.

Un veto s’applique avant toute décision, y compris `exact` ou règle d’identité. Exemple : `environment=prod` contre `environment=staging` lorsque ce champ est configuré comme discriminant.

Un profil générique peut proposer un seuil de candidats validé sur ses benchmarks ; il ne garantit aucune précision sur un domaine inédit. Pour une automatisation réelle, demander des exemples annotés et calibrer une politique locale. Ajouter une régression logistique compacte seulement si les règles pondérées plafonnent et si le gain mesuré justifie l’apprentissage.

Ne pas multiplier des scores pour fabriquer une probabilité. Une probabilité affichée exigerait une calibration spécifique, une évaluation hors échantillon et un suivi du changement de distribution.

## 12. Recherche exacte, lexicale et ANN

### Normalized-equality index (U-007)

`univo-engine` provides `ExactIndex`, bound to a schema and an explicit record-count capacity. It accepts finite batches of `PreparedRecord`: compatibility, capacity and ID uniqueness are checked before insertion. IDs are typed, never normalized and unique across the whole instance, including different partitions. Without an external ID, results name a temporary slot; this slot is not a persistent ID generated by `Finder.add`.

Candidate keys include the partition and every normalized field state, with type tags and lengths, excluding IDs and explanation traces. Each hit is verified by `exact_match`, the authority for equality, informative evidence and partitions. A hash collision never establishes a match.

`search(query, limit)` is read-only and excludes the query's typed ID. Output sorts by ID (strings before integers, lexicographic then numeric order), or by slot without IDs. Every bucket candidate is verified: `total_matches` is complete for normalized equality, `candidates_examined` counts visited entries including the excluded ID, and `truncated()` reports only the output limit. This result does not claim exhaustive lexical or semantic search from U-010.

The capacity bounds record count, not total RAM. The index owns prepared values; engine byte estimates and memory budgeting remain to be implemented. Fingerprints are internal and non-persistent. See the [detailed contract](../docs/exact-index.md).

### Référence exacte

Matrice f32 contiguë. Produit scalaire pour vecteurs normalisés. Top-k avec mémoire O(k), traitement par blocs, départage stable par ID. Pour une requête, coût O(Nd) ; pour toutes les paires, O(N²d). Ne pas allouer une matrice N×N.

La baseline exhaustive est conservée comme oracle des tests, même si les gros workloads utilisent ensuite un index.

### Candidats lexicaux

Index inversé de n-grammes ou tokens ; fréquence des termes conservée ; intersections/recouvrements bornés et reranking par distance d’édition. Les chaînes très courtes ont une voie dédiée, car les trigrams sont insuffisants.

Mesurer les listes de postings très fréquentes. Un plafond de candidats est observable ; il peut réduire le rappel. Les correspondances exactes et les règles d’identité ne doivent pas disparaître dans ce plafond.

### ANN, seulement à partir de 0.2

Comparer une implémentation HNSW maintenue à la référence exacte ; éventuellement une autre famille si HNSW ne satisfait pas le budget mémoire. Choisir une seule dépendance initiale selon licence, wheels, persistance, mutations et rappel mesuré.

Paramètres de construction et recherche enregistrés dans le manifeste. Le mode `auto` choisit selon un seuil déterminé par benchmark, pas un nombre arbitraire copié d’un autre projet.

Union des candidats des index par champ actif, puis déduplication des IDs et reranking complet. Évaluer séparément un index agrégé si le coût des index par champ devient trop élevé.

Un top-k n’énumère pas nécessairement tous les doublons au-dessus d’un seuil. Pour les groupes très denses, augmenter adaptativement le budget jusqu’à la limite déclarée et signaler la saturation. Le mode exhaustif reste le seul contrat de complétude pour l’univers des paires admissibles.

## 17. Groupes et fusion

Le résultat de base est une paire, pas un cluster supposé exact. La similarité n’est pas transitive : A proche de B et B proche de C n’implique pas A proche de C.

Groupes optionnels, après stabilisation des paires :

- égalités exactes vérifiées : groupes par clé canonique ;
- groupes candidats : graphe explicite, sans statut « entité unique » ;
- regroupement conservateur : ajout seulement si toutes les paires nécessaires satisfont la politique, avec ordre stable et limite de taille ; documenter la dépendance possible à l’ordre de construction et le coût O(g²).

Pas de fusion métier automatique en 1.0. Une future fonction `plan_merge` doit conserver la provenance par champ, exposer les conflits, choisir un représentant de façon déterministe et produire un plan réversible. Ne pas utiliser union-find sur toutes les paires similaires comme une preuve d’identité.

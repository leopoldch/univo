# Contrats de l’API Python et résultats

[Retour à l’index des spécifications](README.md)

> Statut : contrats à implémenter. Les exemples ne décrivent pas une bibliothèque déjà disponible. Les budgets chiffrés sont des objectifs à valider, pas des résultats mesurés.

## 5. Expérience Python cible

### 5.1 Installation

```bash
python -m pip install univo
```

Nom de package illustratif jusqu’à vérification. La wheel embarque l’extension native, pas les poids du modèle. Aucun téléchargement au simple `import univo` et aucun chargement du modèle à l’import.

### 5.2 Premier exemple : noms courts

```python
from univo import Finder

finder = Finder()
report = finder.find(["Amazon", "Amiazon", "Microsoft"])

for match in report.matches:
    print(match.left_id, match.right_id, match.score, match.reasons)
```

Le modèle par défaut est résolu paresseusement à la première opération nécessitant le backend sémantique. Le score montré est un score de rapprochement, pas une probabilité. Aucune paire ni valeur numérique précise ne doit être promise dans le README avant l’exécution du test.

### 5.3 Objets structurés, usage simple

```python
records = [
    {"id": "org-1", "name": "Amazon", "description": "Online retail"},
    {"id": "org-2", "name": "Amiazon", "description": "Online retailer"},
]

finder = Finder(fields=["name", "description"], id_field="id")
report = finder.find(records)
```

Ce raccourci sélectionne des champs textuels et une politique générique versionnée. Il n’infère pas qu’un champ `name` est juridiquement unique. Le schéma résolu est consultable avec `finder.config()`.

### 5.4 Configuration explicite

```python
from univo import Finder, Schema, Text, Exact

schema = Schema(
    id_field="id",
    partition_by=["tenant_id"],
    fields={
        "name": Text(mode="hybrid", weight=0.6),
        "description": Text(mode="semantic", weight=0.4),
        "domain": Exact(normalizer="hostname", required=False),
        "environment": Exact(required=False, conflict="reject"),
    },
)

finder = Finder(schema=schema, model="default", threads=4)
finder.add(records)
result = finder.search(new_record, limit=10)
```

`Text(..., case_fold=False)` désactive le pliage de casse d’un champ textuel lorsque la casse porte du sens (`mA` / `MA`) ; il est activé par défaut.

Ces poids illustrent la syntaxe ; ils ne constituent pas des valeurs validées. `domain` n’est ni un identifiant unique ni un veto par défaut. Le champ environnement ne bloque que si les deux valeurs sont présentes et contradictoires. Un champ requis absent provoque une erreur d’entrée, pas une égalité fictive.

### 5.5 Choix du modèle, mode entreprise et hors ligne

L’utilisateur choisit le modèle par une simple chaîne, sans changer d’API :

```python
Finder(model="default")                                   # défaut de la release, révision et checksums fixés
Finder(model="minishlab/potion-base-8M")                  # dépôt Hugging Face ; révision résolue au téléchargement
Finder(model="minishlab/potion-base-8M@bf8b0566")          # dépôt Hugging Face à une révision fixée (recommandé)
Finder(model="/opt/models/univo/fr-v1")                   # répertoire local (chemin absolu, ./, ../, ~ ou file://)
Finder(model="org/model", hub_url="https://hub.internal")  # miroir compatible avec l’API Hugging Face
```

```python
finder = Finder(
    schema=schema,
    model="/opt/models/univo/default-v1",
    offline=True,
    threads=4,
)
```

Règles :

- Une chaîne est un chemin local si elle est absolue, commence par `./`, `../`, `~` ou `file://` ; sinon `org/nom[@révision]` désigne un dépôt Hugging Face. Aucune autre devinette : `org/nom` n’est jamais interprété comme un chemin relatif.
- `model_path` reste accepté comme forme explicite d’un chemin local ; `model` et `model_path` sont mutuellement exclusifs.
- `hub_url` (ou la variable d’environnement `HF_ENDPOINT`) désigne un miroir compatible ; aucune donnée utilisateur n’est envoyée, seulement les noms de fichiers du dépôt.
- Une révision non fixée est résolue en hash de commit au premier téléchargement, puis enregistrée dans le cache et dans les index ; les index refusent ensuite un autre modèle ou une autre révision.
- En mode offline, aucun appel réseau n’est tenté, même pour vérifier des métadonnées. Si un fichier manque, lever `ModelUnavailableError` avec l’action nécessaire (chemin attendu, commande de préchargement).
- Formats acceptés : ceux du backend livré (voir [modèles](04-models-and-cache.md)). Un dépôt dans un autre format est refusé avec une erreur expliquant comment le convertir ; aucun code téléchargé n’est exécuté.

### 5.6 Persistance et mises à jour, à partir de 0.2

```python
finder.save("./company-index")
finder = Finder.load("./company-index", offline=True)

finder.upsert(updated_records)
finder.remove(["org-2"])
finder.save("./company-index")
```

Le modèle doit être présent dans le cache, fourni par chemin, ou inclus explicitement dans un export portable. La sauvegarde standard n’embarque pas automatiquement ses poids.

## 6. Contrat précis des opérations

| Méthode | Sémantique et erreurs |
| --- | --- |
| `Finder(...)` | valide configuration ; n’effectue ni réseau ni indexation |
| `find(records)` | compare au sein d’un jeu temporaire ; ne modifie pas l’index persistant du Finder |
| `add(records)` | insère dans l’index ; refuse un ID déjà présent et valide le lot avant commit |
| `search(record, limit=10)` | recherche en lecture seule ; exclut l’ID de requête s’il existe déjà |
| `upsert(records)` | remplace par ID ; invalide seulement les éléments dépendant des champs modifiés |
| `remove(ids)` | supprime de la vue active ; renvoie les IDs absents sans supprimer d’autres objets |
| `save(path)` | publie un snapshot cohérent de la génération courante |
| `load(path)` | vérifie format, politique, modèle et intégrité avant exposition |
| `stats()` | compteurs, générations, mémoire estimée, cache et configuration, sans données métier |
| `config()` | configuration effective et versions résolues |

Pour un objet sans ID : `find` attribue des positions temporaires ; `add` retourne des IDs générés. La mise à jour exige l’ID retourné. Ne pas identifier un objet par son contenu mutable.

Une mutation de lot est atomique à l’échelle de l’instance : validation et encodage en staging, puis commit. En cas de budget mémoire dépassé, erreur avant commit. Pour de très gros flux, l’utilisateur traite des lots et reçoit un accusé par lot ; ne pas prétendre à une transaction sur un générateur illimité.

Les entrées ne sont jamais modifiées en place. Ne pas appeler arbitrairement `str(obj)` pour accepter un type inconnu : erreur typée, avec chemin du champ concerné.

## 7. Résultats et vocabulaire

Une correspondance contient au minimum :

```python
Match(
    left_id="org-1",
    right_id="org-2",
    score=...,                    # score de classement, pas une probabilité
    kind="candidate",            # exact | rule_match | candidate
    field_scores={...},
    reasons=[...],
    conflicts=[...],
    coverage=...,
)
```

`exact` signifie égalité vérifiée des champs sélectionnés après leur normalisation documentée. Il ne signifie pas « même entité réelle » lorsque des champs ont été ignorés. La normalisation ne construit que des clés de comparaison et ne modifie jamais les données : un match `exact` indique, pour chaque champ dont les valeurs d’origine diffèrent, les étapes nécessaires pour les rendre égales (forme Unicode, casse, point final, ordre des clés).

`rule_match` signifie qu’une règle d’identité configurée a été satisfaite, sous réserve des veto et des partitions. Exemple : même identifiant fournisseur déclaré unique par l’utilisateur.

`candidate` signifie ressemblance suffisante selon le profil, nécessitant la politique métier de l’application pour fusionner ou rejeter.

Le rapport expose : liste ou itérateur borné de matches, nombre d’objets traités, avertissements de troncature, génération d’index, empreinte de configuration, profil de recherche et compteurs de candidats. Un résultat ANN ne porte jamais un indicateur trompeur « aucun doublon possible ».

État de recherche : `exhaustive` seulement si toutes les paires admissibles ont été scorées ; sinon `bounded` ou `approximate`. Distinguer troncature de sortie (`limit`) et limitation des candidats internes.

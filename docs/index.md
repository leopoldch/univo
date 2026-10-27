---
hide:
  - navigation
  - toc
---

<div class="hero" markdown>

# Univo

<p class="lead">Find duplicate records without crossing your rules. A different customer, environment or tax number rules a match out, whatever the score. Fuzzy and semantic matching, in Python, on your machine.</p>

[Get started](guide.md){ .md-button .md-button--primary }
[See a real example](examples/chicago.md){ .md-button }

</div>

```bash
pip install univo
```

<div class="demo" markdown>

<p class="demo-title">Five clients named Northwind. One is a duplicate.</p>

<table class="demo-table">
<thead><tr><th>#</th><th>Tenant</th><th>Name</th><th>Activity</th><th>VAT</th><th>Compared with client 1</th></tr></thead>
<tbody>
<tr><td>1</td><td>shop</td><td>Northwind Traders</td><td>online clothing store</td><td>FR401</td><td class="muted">the record we check</td></tr>
<tr><td>2</td><td>shop</td><td>North<mark>wnd</mark> Traders</td><td><mark>shop selling clothes on the internet</mark></td><td>FR401</td><td><span class="badge good">✓ Duplicate</span> score 0.91: a typo, and the same activity in other words</td></tr>
<tr><td>3</td><td>shop</td><td>Northwind Traders</td><td><mark>steel manufacturer</mark></td><td class="muted">none</td><td><span class="badge bad">✕ Not a duplicate</span> score 0.53: same name, another business</td></tr>
<tr><td>4</td><td>shop</td><td>Northwind Traders</td><td>online clothing store</td><td><mark>FR999</mark></td><td><span class="badge bad">✕ Ruled out</span> another VAT number, whatever the score</td></tr>
<tr><td>5</td><td><mark>bank</mark></td><td>Northwind Traders</td><td>online clothing store</td><td>FR401</td><td><span class="badge bad">✕ Never compared</span> another tenant</td></tr>
</tbody>
</table>

</div>

The code, and what it prints:

```python
from univo import Exact, Finder, Schema, Text

schema = Schema(
    id_field="id",
    partition_by=["tenant"],                    # tenants are never compared
    fields={
        "name": Text(),                         # spelling: typos and case
        "activity": Text(mode="semantic"),      # meaning, with a local model
        "vat": Exact(conflict="reject"),        # a different VAT number rules a match out
    },
)
clients = [
    {"id": 1, "tenant": "shop", "name": "Northwind Traders", "activity": "online clothing store", "vat": "FR401"},
    {"id": 2, "tenant": "shop", "name": "Northwnd Traders", "activity": "shop selling clothes on the internet", "vat": "FR401"},
    {"id": 3, "tenant": "shop", "name": "Northwind Traders", "activity": "steel manufacturer", "vat": None},
    {"id": 4, "tenant": "shop", "name": "Northwind Traders", "activity": "online clothing store", "vat": "FR999"},
    {"id": 5, "tenant": "bank", "name": "Northwind Traders", "activity": "online clothing store", "vat": "FR401"},
]
for match in Finder(schema=schema).find(clients).matches:
    print(match.left_id, match.right_id, round(match.score, 2), match.reasons)
```

```text
1 2 0.91 ['name: edit_distance', 'activity: semantic', 'vat: exact']
```

Spelling catches the typo, the local model reads the meaning of the activity, and the rules keep clients 4 and 5 out, even with `min_score=0.01`. The first run downloads the 15 MB model once; it then works offline.

No training, no API key, no server: records never leave your machine.

## What it is for

=== "Clean a list"

    Supplier lists, customer exports, scraped catalogs: `group` proposes which rows to merge. A veto keeps apart records that must never match.

    ```python
    from univo import Exact, Finder, Schema, Text

    suppliers = [
        {"id": "A-17", "name": "Northwind Traders SAS", "city": "Lyon", "vat": "FR401"},
        {"id": "B-03", "name": "NORTHWIND TRADERS", "city": "LYON", "vat": "FR401"},
        {"id": "B-09", "name": "Northwind Trading", "city": "Paris", "vat": "FR777"},
        {"id": "A-21", "name": "Contoso Ltd", "city": "Leeds", "vat": None},
    ]
    schema = Schema(id_field="id", fields={"name": Text(), "city": Text(), "vat": Exact(conflict="reject")})
    groups = Finder(schema=schema).group(suppliers, min_score=0.7)
    print([group.members for group in groups.groups])
    ```

    ```text
    [('A-17', 'B-03')]
    ```

    `B-09` has another VAT number, so it stays apart. On real data: [nine lists of Chicago's early childhood sites](examples/chicago.md).

=== "Check each new record"

    Sign-ups, leads, tickets: check a record before storing it. In the [Chicago example](https://github.com/leopoldch/univo-examples/tree/main/check-new-records), a check against about 1,300 to 2,000 stored records took 0.12 ms at the median on a laptop ([measured](../benchmarks/chicago/README.md)); the time grows with the records of a partition.

    ```python
    from univo import Exact, Finder, Schema, Text

    finder = Finder(schema=Schema(id_field="id", fields={"name": Text(), "email": Exact()}))
    finder.add([{"id": 1, "name": "Ada Lovelace", "email": "ada@example.org"}])

    for signup in [
        {"id": 2, "name": "Ada Lovelace", "email": "ada@example.org"},
        {"id": 3, "name": "Ada Lovelce", "email": None},
        {"id": 4, "name": "Grace Hopper", "email": "grace@example.org"},
    ]:
        print(signup["id"], finder.check_or_add(signup).status)
    ```

    ```text
    2 matched
    3 review
    4 inserted
    ```

    `review` means a similar record exists and nothing was stored: your application decides.

=== "Keep tenants apart"

    In a multi-tenant application, a record only matches records of its own tenant, and search only reads that tenant.

    ```python
    from univo import Finder, Schema, Text

    schema = Schema(id_field="id", partition_by=["tenant"], fields={"name": Text()})
    finder = Finder(schema=schema)
    finder.add([
        {"id": 1, "tenant": "acme", "name": "Globex Corporation"},
        {"id": 2, "tenant": "initech", "name": "Globex Corporation"},
    ])
    report = finder.search({"id": 3, "tenant": "acme", "name": "Globex Corporaton"})
    print([match.right_id for match in report.matches])
    ```

    ```text
    [1]
    ```

=== "Match by meaning"

    Questions, descriptions, product titles: semantic fields compare meaning with a local model, downloaded once and run offline. The small default model catches close rewordings like this one, not every paraphrase.

    ```python
    from univo import Finder, Schema, Text

    questions = [
        "How do I reset my password?",
        "What is the way to reset my password?",
        "How can I delete my account?",
        "Steps to change the password I forgot",
    ]
    finder = Finder(schema=Schema(fields={"text": Text(mode="semantic")}))
    report = finder.find([{"text": q} for q in questions], min_score=0.7)
    for match in report.matches:
        print(questions[match.left_id], "~", questions[match.right_id], round(match.score, 2))
    ```

    ```text
    How do I reset my password? ~ What is the way to reset my password? 0.74
    ```

=== "Compare configurations"

    Services, hosts, settings: nested fields by path, hostnames normalized, environments as vetoes.

    ```python
    from univo import Exact, Finder, Schema, Text

    services = [
        {"id": "svc-1", "name": "billing-api", "env": "prod", "db": {"host": "db1.internal", "port": 5432}},
        {"id": "svc-2", "name": "billing-api", "env": "prod", "db": {"host": "DB1.internal.", "port": 5432}},
        {"id": "svc-3", "name": "billing-api", "env": "staging", "db": {"host": "db1.internal", "port": 5432}},
    ]
    schema = Schema(
        id_field="id",
        fields={
            "name": Text(),
            "env": Exact(conflict="reject"),
            "db.host": Exact(normalizer="hostname"),
            "db.port": Exact(),
        },
    )
    report = Finder(schema=schema).find(services)
    print([(m.left_id, m.right_id, m.kind, round(m.score, 2)) for m in report.matches])
    ```

    ```text
    [('svc-1', 'svc-2', 'exact', 1.0)]
    ```

    `DB1.internal.` and `db1.internal` are one hostname; `svc-3` runs in staging and never matches production.

## Rules hold where similarity fails

Two configurations that differ only by `prod` and `staging` look almost identical to an embedding model. Univo checks partitions and vetoes before any score.

<div class="viz-root" data-chart="bars"><script type="application/json">{"title": "Pairs that must never match, matched anyway", "subtitle": "44 forbidden pairs among 222 structured record pairs: different tenants, or prod against staging. Each tool at its default threshold. Lower is better.", "format": "int", "valueName": "Forbidden matches", "labelWidth": 150, "items": [{"label": "Univo, semantic", "value": 0, "color": 1, "note": "none at any threshold from 0.5 to 1"}, {"label": "SemHash 0.5.0", "value": 35, "color": "ref", "note": "6 still at 0.99"}]}</script></div>

SemHash does not claim to handle partitions or discriminant fields; its users filter them in their own code. Method and every result: [benchmarks](benchmarks.md#structured-records).

## On real data

Nine public lists of Chicago's early childhood sites, the dataset of the [Dedupe](https://github.com/dedupeio/dedupe-examples) example:

<div class="tiles">
<div class="tile"><div class="figure">3,337</div><div class="caption">rows from 9 sources, for 1,162 real sites</div></div>
<div class="tile"><div class="figure">889</div><div class="caption">groups of proposed duplicates, without labelling</div></div>
<div class="tile"><div class="figure">96%</div><div class="caption">precision on sites held out from tuning</div></div>
<div class="tile"><div class="figure">0.1 s</div><div class="caption">for the whole dataset, on an Apple M2 laptop</div></div>
</div>

[Read the walkthrough](examples/chicago.md)

## Measured, not promised

<div class="viz-root" data-chart="bars"><script type="application/json">{"title": "Deduplicate 100,000 names, semantic", "subtitle": "Wall time with the same model, Apple M2, every core. Lower is better.", "format": "s", "valueName": "Time", "labelWidth": 110, "items": [{"label": "Univo 1.0.1", "value": 4.5, "color": 1, "note": "exact search, 262 MiB"}, {"label": "SemHash 0.5.0", "value": 7.6, "color": "ref", "note": "approximate index, 488 MiB"}]}</script></div>
<div class="viz-root" data-chart="bars"><script type="application/json">{"title": "Peak memory, same run", "subtitle": "Lower is better.", "format": "MiB", "valueName": "Peak memory", "labelWidth": 110, "items": [{"label": "Univo 1.0.1", "value": 262, "color": 1}, {"label": "SemHash 0.5.0", "value": 488, "color": "ref"}]}</script></div>

Synthetic company names, one partition. Univo's search is exhaustive; SemHash's index is approximate. Results depend on your data: measure on a sample of yours. Method and every result: [benchmarks](benchmarks.md).

## Next

- [Guide](guide.md): everything you need, on one page.
- [Chicago walkthrough](examples/chicago.md): a real dataset, end to end.
- [API reference](reference/api.md): every class, method and option.

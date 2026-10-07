"""Build tests/fixtures/critical/cases.jsonl from the case tables below.

The tables are the reviewed source of truth; the JSONL file is generated so
that every case has the same shape and identifiers stay stable. Labels follow
docs/duplicate-definitions.md. All records are synthetic.

Usage:
    python tools/build_critical_corpus.py          # rewrite the fixture
    python tools/build_critical_corpus.py --check  # fail if the fixture is stale
"""

import argparse
import json
import pathlib
import sys
from collections import Counter
from typing import Any

OUTPUT = pathlib.Path(__file__).resolve().parents[1] / "tests/fixtures/critical/cases.jsonl"

ABSENT: Any = object()

DUPLICATE = "duplicate"
DISTINCT = "distinct"
INVALID = "invalid"


def _record(fields: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in fields.items() if value is not ABSENT}


def org(
    name: Any,
    domain: Any = ABSENT,
    description: Any = ABSENT,
    tenant: Any = "tenant-a",
) -> dict[str, Any]:
    return _record(
        {"tenant_id": tenant, "name": name, "domain": domain, "description": description}
    )


def product(name: Any, sku: Any = ABSENT, description: Any = ABSENT) -> dict[str, Any]:
    return _record({"name": name, "sku": sku, "description": description})


def config(
    service: Any,
    environment: Any = "prod",
    params: Any = ABSENT,
    description: Any = ABSENT,
) -> dict[str, Any]:
    return _record(
        {
            "service": service,
            "environment": environment,
            "params": params,
            "description": description,
        }
    )


PROFILE_OF = {"org": "organizations", "product": "products", "config": "configurations"}


class Corpus:
    def __init__(self) -> None:
        self.cases: list[dict[str, Any]] = []
        self._counters: Counter[str] = Counter()

    def add(
        self,
        family: str,
        profile: str,
        label: str,
        left: Any,
        right: Any,
        rationale: str,
        *,
        lang: str = "en",
        constraint: str | None = None,
        expected_kind: str | None = None,
        expected_error: str | None = None,
        assign_ids: bool = True,
    ) -> None:
        self._counters[family] += 1
        case_id = f"{family}-{self._counters[family]:03d}"
        if profile != "strings" and assign_ids:
            left = _with_id(left, f"{case_id}-l")
            right = _with_id(right, f"{case_id}-r")
        self.cases.append(
            {
                "case_id": case_id,
                "profile": profile,
                "family": family,
                "lang": lang,
                "label": label,
                "constraint": constraint,
                "expected_kind": expected_kind,
                "expected_error": expected_error,
                "left": left,
                "right": right,
                "rationale": rationale,
            }
        )


def _with_id(record: Any, identifier: str) -> Any:
    if not isinstance(record, dict) or "id" in record:
        return record
    return {"id": identifier, **record}


def build() -> list[dict[str, Any]]:
    corpus = Corpus()
    _typos(corpus)
    _semantic_competitors(corpus)
    _paraphrases(corpus)
    _homonyms(corpus)
    _negations(corpus)
    _critical_parameters(corpus)
    _versions_and_environments(corpus)
    _missing_values(corpus)
    _unicode(corpus)
    _ordering(corpus)
    _tenants(corpus)
    _dense_series(corpus)
    _short_strings(corpus)
    _adversarial(corpus)
    return corpus.cases


def _typos(c: Corpus) -> None:
    f = "typo"
    for left, right in [
        ("Amazon", "Amiazon"),
        ("Microsoft", "Microsfot"),
        ("Google", "Gogle"),
        ("Salesforce", "Salesfroce"),
        ("Netflix", "Netlfix"),
        ("Spotify", "Spotfy"),
        ("Johnson & Johnson", "Johnson and Johnson"),
    ]:
        c.add(f, "strings", DUPLICATE, left, right, "Spelling variant of the same name.")
    for left, right in [
        ("Galeries Lafayette", "Galeries Lafayete"),
        ("Crédit Agricole", "Crédit Agricol"),
        ("La Poste", "LaPoste"),
    ]:
        c.add(
            f,
            "strings",
            DUPLICATE,
            left,
            right,
            "Spelling variant of the same name.",
            lang="fr",
        )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Amazon", "amazon.com", "Online retail"),
        org("Amiazon", "amazon.com", "Online retailer"),
        "Typo in the name; same domain and equivalent description.",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Shopify", "shopify.com", "E-commerce platform for online stores"),
        org("Shopfiy", "shopify.com", "E-commerce platform for online stores"),
        "Transposed letters; every other field equal.",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Airbnb", "airbnb.com", "Marketplace for short-term home rentals"),
        org("Air bnb", "airbnb.com", "Marketplace for short-term home rentals"),
        "Inserted space in the name.",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Deutsche Bank", "db.com", "Investment and retail bank"),
        org("Deutsche Bnak", "db.com", "Investment and retail bank"),
        "Transposed letters in a two-word name.",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Carrefour", "carrefour.fr", "Chaîne d'hypermarchés et de supermarchés"),
        org("Carefour", "carrefour.fr", "Chaîne d'hypermarchés et de supermarchés"),
        "Missing doubled consonant.",
        lang="fr",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Decathlon", "decathlon.fr", "Distributeur d'articles de sport"),
        org("Decatlon", "decathlon.fr", "Distributeur d'articles de sport"),
        "Missing letter.",
        lang="fr",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Michelin", "michelin.com", "Fabricant de pneumatiques"),
        org("Michellin", "michelin.com", "Fabricant de pneumatiques"),
        "Doubled consonant.",
        lang="fr",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("PayPal", "paypal.com", "Online payments"),
        org("Paypall", "paypal.com", "Online payment services"),
        "Extra letter and case change.",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Dropbox", "dropbox.com", "Cloud file storage"),
        org("Drop box", "dropbox.com", "Cloud file storage and sharing"),
        "Split compound name.",
    )
    c.add(
        f,
        "products",
        DUPLICATE,
        product("USB-C charging cable 1 m", "CBL-USBC-1M"),
        product("USB-C chargng cable 1 m", "CBL-USBC-1M"),
        "Typo in the name; same SKU.",
    )
    c.add(
        f,
        "products",
        DUPLICATE,
        product("Wireless mouse M185", "MOU-M185"),
        product("Wirless mouse M185", "MOU-M185"),
        "Missing letter; same SKU.",
    )
    c.add(
        f,
        "products",
        DUPLICATE,
        product("Stainless steel water bottle 750 ml"),
        product("Stainless steel water botle 750 ml"),
        "Typo; no SKU on either side.",
    )
    c.add(
        f,
        "products",
        DUPLICATE,
        product("Ergonomic office chair", "CHR-ERGO-01"),
        product("Ergonomic ofice chair", "CHR-ERGO-01"),
        "Missing letter; same SKU.",
    )
    c.add(
        f,
        "products",
        DUPLICATE,
        product("Chaise de bureau ergonomique"),
        product("Chaise de bureau ergonomiqe"),
        "Missing letter.",
        lang="fr",
    )


def _semantic_competitors(c: Corpus) -> None:
    f = "semantic_competitor"
    for left, right in [
        ("Amazon", "Microsoft"),
        ("Coca-Cola", "PepsiCo"),
        ("Visa", "Mastercard"),
        ("Nike", "Adidas"),
        ("Uber", "Lyft"),
        ("Airbus", "Boeing"),
        ("Intel", "AMD"),
    ]:
        c.add(f, "strings", DISTINCT, left, right, "Competitors in the same market.")
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("Apple", "apple.com", "Consumer electronics, smartphones and computers"),
        org("Samsung Electronics", "samsung.com", "Consumer electronics, smartphones and TVs"),
        "Near-identical descriptions; different companies.",
    )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("BNP Paribas", "bnpparibas.com", "Banque de détail et d'investissement"),
        org("Société Générale", "societegenerale.com", "Banque de détail et d'investissement"),
        "Identical descriptions; different banks.",
        lang="fr",
    )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("Renault", "renault.fr", "Constructeur automobile français"),
        org("Peugeot", "peugeot.fr", "Constructeur automobile français"),
        "Identical descriptions; different manufacturers.",
        lang="fr",
    )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("Carrefour", "carrefour.fr", "Chaîne d'hypermarchés et de supermarchés"),
        org("Auchan", "auchan.fr", "Chaîne d'hypermarchés et de supermarchés"),
        "Identical descriptions; different retailers.",
        lang="fr",
    )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("Spotify", "spotify.com", "Music streaming service"),
        org("Deezer", "deezer.com", "Music streaming service"),
        "Identical descriptions; different services.",
    )
    c.add(
        f,
        "products",
        DISTINCT,
        product("Slack", description="Team messaging and collaboration app"),
        product("Microsoft Teams", description="Team chat and collaboration app"),
        "Competing products with paraphrased descriptions.",
    )
    c.add(
        f,
        "products",
        DISTINCT,
        product("Zoom Meetings", description="Video conferencing software"),
        product("Google Meet", description="Video conferencing software"),
        "Competing products with identical descriptions.",
    )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("Free", "free.fr", "Fournisseur d'accès à internet et opérateur mobile"),
        org("SFR", "sfr.fr", "Opérateur mobile et fournisseur d'accès à internet"),
        "Reworded identical activity; different operators.",
        lang="fr",
    )


def _paraphrases(c: Corpus) -> None:
    f = "paraphrased_description"
    pairs = [
        (
            "organizations",
            org("Stripe", "stripe.com", "Payment processing for internet businesses"),
            org("Stripe", "stripe.com", "Online payments infrastructure for companies"),
            "en",
        ),
        (
            "organizations",
            org("Doctolib", "doctolib.fr", "Prise de rendez-vous médicaux en ligne"),
            org("Doctolib", "doctolib.fr", "Plateforme de réservation de consultations médicales"),
            "fr",
        ),
        (
            "organizations",
            org("BlaBlaCar", "blablacar.fr", "Covoiturage longue distance"),
            org("BlaBlaCar", "blablacar.fr", "Long-distance carpooling platform"),
            "mixed",
        ),
        (
            "organizations",
            org("Atlassian", "atlassian.com", "Software for team collaboration and tracking"),
            org("Atlassian", "atlassian.com", "Tools that help teams plan and track work"),
            "en",
        ),
        (
            "organizations",
            org("Ubisoft", "ubisoft.com", "Éditeur de jeux vidéo"),
            org("Ubisoft", "ubisoft.com", "Développeur et éditeur de jeux vidéo"),
            "fr",
        ),
        (
            "products",
            product(
                "Noise-cancelling headphones WH-1000XM5",
                "AUD-WH1000XM5",
                "Over-ear wireless headphones with active noise cancellation",
            ),
            product(
                "Noise-cancelling headphones WH-1000XM5",
                "AUD-WH1000XM5",
                "Wireless over-ear headset featuring ANC",
            ),
            "en",
        ),
        (
            "products",
            product("Robot pâtissier 5 L", "KIT-5L", "Robot de cuisine pour pétrir et fouetter"),
            product("Robot pâtissier 5 L", "KIT-5L", "Batteur sur socle pour pâtes et blancs"),
            "fr",
        ),
        (
            "products",
            product("Trail running shoes", "SHO-TR-42", "Lightweight shoes for off-road running"),
            product("Trail running shoes", "SHO-TR-42", "Light footwear designed for trail runs"),
            "en",
        ),
        (
            "products",
            product("Lampe de bureau LED", "LMP-LED-02", "Lampe orientable à faible consommation"),
            product("Lampe de bureau LED", "LMP-LED-02", "Adjustable low-energy desk lamp"),
            "mixed",
        ),
        (
            "products",
            product("4-slot toaster", "TST-4", "Toasts four slices of bread at once"),
            product("4-slot toaster", "TST-4", "Toaster that browns up to four slices together"),
            "en",
        ),
        (
            "configurations",
            config(
                "billing-api",
                params={"retries": 3},
                description="Retries failed payment webhooks",
            ),
            config(
                "billing-api",
                params={"retries": 3},
                description="Re-sends payment webhooks that failed",
            ),
            "en",
        ),
        (
            "configurations",
            config(
                "search",
                params={"timeout_ms": 500},
                description="Abandonne la requête de recherche après 500 ms",
            ),
            config(
                "search",
                params={"timeout_ms": 500},
                description="Coupe les recherches qui dépassent une demi-seconde",
            ),
            "fr",
        ),
        (
            "configurations",
            config(
                "mailer",
                params={"batch_size": 100},
                description="Sends newsletters in batches of one hundred",
            ),
            config(
                "mailer",
                params={"batch_size": 100},
                description="Newsletter delivery grouped by 100 recipients",
            ),
            "en",
        ),
        (
            "configurations",
            config("cdn", params={"ttl_s": 3600}, description="Caches static assets for one hour"),
            config(
                "cdn", params={"ttl_s": 3600}, description="Static files stay cached 60 minutes"
            ),
            "en",
        ),
        (
            "configurations",
            config(
                "auth",
                params={"session_days": 30},
                description="Les sessions expirent au bout d'un mois",
            ),
            config(
                "auth",
                params={"session_days": 30},
                description="User sessions expire after thirty days",
            ),
            "mixed",
        ),
    ]
    for profile, left, right, lang in pairs:
        c.add(
            f,
            profile,
            DUPLICATE,
            left,
            right,
            "Same record; only the description is reworded.",
            lang=lang,
        )


def _homonyms(c: Corpus) -> None:
    f = "homonym"
    pairs = [
        (
            org("Delta", "delta.com", "Airline"),
            org("Delta", "deltafaucet.com", "Kitchen and bathroom faucets"),
            "en",
        ),
        (
            org("Apple", "apple.com", "Consumer electronics"),
            org("Apple", "apple-corps.example", "Record label and music publisher"),
            "en",
        ),
        (
            org("Orange", "orange.fr", "Opérateur de télécommunications"),
            org("Orange", "orange-jus.example", "Marque de jus de fruits"),
            "fr",
        ),
        (
            org("Mercury", "mercury.com", "Banking for startups"),
            org("Mercury", "mercury-insurance.example", "Car insurance"),
            "en",
        ),
        (
            org("Jaguar", "jaguar.com", "Luxury car manufacturer"),
            org("Jaguar", "jaguar-sound.example", "Audio equipment retailer"),
            "en",
        ),
        (
            org("Dove", "dove.com", "Soap and personal care"),
            org("Dove", "dovechocolate.example", "Chocolate confectionery"),
            "en",
        ),
        (
            org("Lincoln", "lincoln.com", "Luxury vehicles"),
            org("Lincoln", "lincolnelectric.example", "Welding equipment"),
            "en",
        ),
        (
            org("Atlas", "atlas-fret.example", "Transport routier de marchandises"),
            org("Atlas", "atlas-editions.example", "Maison d'édition de cartes"),
            "fr",
        ),
        (
            org("Phoenix", "phoenix-labs.example", "Clinical laboratory testing"),
            org("Phoenix", "phoenix-games.example", "Video game studio"),
            "en",
        ),
        (
            org("Total", "totalenergies.example", "Énergie et carburants"),
            org("Total", "total-nutrition.example", "Compléments alimentaires"),
            "fr",
        ),
    ]
    for left, right, lang in pairs:
        c.add(
            f,
            "organizations",
            DISTINCT,
            left,
            right,
            "Same name; different domain and activity.",
            lang=lang,
        )
    c.add(
        f,
        "products",
        DISTINCT,
        product("Galaxy", "PHN-GALAXY", "Android smartphone"),
        product("Galaxy", "CHOC-GALAXY", "Milk chocolate bar"),
        "Same name; different SKU and category.",
        constraint="veto",
    )
    c.add(
        f,
        "products",
        DISTINCT,
        product("Mustang", description="Sports car scale model 1:18"),
        product("Mustang", description="Leather saddle for horse riding"),
        "Same name; unrelated products, no SKU.",
    )
    c.add(
        f,
        "products",
        DISTINCT,
        product("Mistral", description="Ventilateur colonne 3 vitesses"),
        product("Mistral", description="Voilier de croisière 9 mètres"),
        "Same name; unrelated products, no SKU.",
        lang="fr",
    )
    c.add(
        f,
        "strings",
        DISTINCT,
        "Python (programming language)",
        "Python (snake)",
        "Same head word; different meaning.",
    )
    c.add(
        f,
        "strings",
        DISTINCT,
        "Avocat (fruit)",
        "Avocat (profession)",
        "Same head word; different meaning.",
        lang="fr",
    )
    c.add(
        f,
        "strings",
        DISTINCT,
        "Paris, France",
        "Paris, Texas",
        "Same city name; different places.",
    )


def _negations(c: Corpus) -> None:
    f = "negation"
    for left, right, lang in [
        ("Laptop with charger", "Laptop without charger", "en"),
        ("Supports two-factor authentication", "Does not support two-factor authentication", "en"),
        ("Gluten-free bread", "Bread with gluten", "en"),
        ("Waterproof jacket", "Non-waterproof jacket", "en"),
        ("Alcohol-free beer", "Beer", "en"),
        ("Price includes VAT", "Price excludes VAT", "en"),
        ("Yaourt avec sucre", "Yaourt sans sucre", "fr"),
        ("Compte activé", "Compte désactivé", "fr"),
        ("Livraison incluse", "Livraison non incluse", "fr"),
        ("Fumeur autorisé", "Fumeur interdit", "fr"),
        ("Animaux acceptés", "Animaux non acceptés", "fr"),
    ]:
        c.add(f, "strings", DISTINCT, left, right, "Negated or opposite attribute.", lang=lang)
    for left, right in [
        ("Feature flag enabled for checkout", "Feature flag disabled for checkout"),
        ("Allow public read access to the bucket", "Deny public read access to the bucket"),
        ("Encryption at rest is enabled", "Encryption at rest is not enabled"),
    ]:
        c.add(
            f,
            "configurations",
            DISTINCT,
            config("storage", description=left),
            config("storage", description=right),
            "Opposite meaning in an otherwise identical description.",
        )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("cache", description="Cache activé pour les pages produit"),
        config("cache", description="Cache désactivé pour les pages produit"),
        "Opposite meaning in an otherwise identical description.",
        lang="fr",
    )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("checkout", params={"enabled": True}, description="Express checkout"),
        config("checkout", params={"enabled": False}, description="Express checkout"),
        "Typed boolean parameter contradicts.",
        constraint="veto",
    )


def _critical_parameters(c: Corpus) -> None:
    f = "critical_parameter"
    vetoed = [
        ({"retries": 0}, {"retries": 3}, "Different retry count."),
        ({"timeout_ms": 1000}, {"timeout_ms": 100}, "Different timeout."),
        ({"max_connections": 10}, {"max_connections": 100}, "Different pool size."),
        ({"tls": True}, {"tls": False}, "TLS on versus off."),
        ({"region": "eu-west-1"}, {"region": "us-east-1"}, "Different region."),
        ({"port": 5432}, {"port": 5433}, "Different port."),
        ({"log_level": "info"}, {"log_level": "debug"}, "Different log level."),
        ({"retries": 3}, {"retries": "3"}, "Integer versus string: types differ."),
        ({"tls": True}, {"tls": 1}, "Boolean versus integer: types differ."),
        ({"ratio": 0.5}, {"ratio": 0.05}, "Different float value."),
        ({"timeout": "1 s"}, {"timeout": "1000 ms"}, "No unit conversion is declared."),
    ]
    for left, right, rationale in vetoed:
        c.add(
            f,
            "configurations",
            DISTINCT,
            config("worker", params=left, description="Background job worker"),
            config("worker", params=right, description="Background job worker"),
            rationale,
            constraint="veto",
        )
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("worker", params={"retries": 3, "timeout_ms": 1000}, description="Job worker"),
        config("worker", params={"retries": 3, "timeout_ms": 1000}, description="Job worker"),
        "Every field equal.",
        expected_kind="exact",
    )
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("worker", params={"replicas": 3}, description="Runs three worker replicas"),
        config("worker", params={"replicas": 3}, description="Three replicas of the worker"),
        "Equal parameters; reworded description.",
    )
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("worker", params={"ratio": 0.5}, description="Échantillonne la moitié des jobs"),
        config("worker", params={"ratio": 0.5}, description="Échantillonnage de 50 % des jobs"),
        "Equal parameters; reworded description.",
        lang="fr",
    )
    c.add(
        f,
        "products",
        DISTINCT,
        product("SSD 1 TB", "SSD-1TB", "NVMe solid-state drive"),
        product("SSD 2 TB", "SSD-2TB", "NVMe solid-state drive"),
        "Different capacity and SKU.",
        constraint="veto",
    )


def _versions_and_environments(c: Corpus) -> None:
    f = "version_environment"
    for left, right, rationale in [
        ("prod", "staging", "Production versus staging."),
        ("prod", "dev", "Production versus development."),
        ("staging", "dev", "Staging versus development."),
        ("prod", "production", "No environment alias is declared."),
        ("eu-prod", "us-prod", "Different regional environment."),
    ]:
        c.add(
            f,
            "configurations",
            DISTINCT,
            config("payments", left, {"retries": 3}, "Payment gateway settings"),
            config("payments", right, {"retries": 3}, "Payment gateway settings"),
            rationale,
            constraint="veto",
        )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("payments-v1", params={"retries": 3}, description="Payment gateway settings"),
        config("payments-v2", params={"retries": 3}, description="Payment gateway settings"),
        "Different service version.",
        constraint="veto",
    )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("api", params={"version": "1.4.2"}, description="Public API gateway"),
        config("api", params={"version": "1.4.3"}, description="Public API gateway"),
        "Different patch version.",
        constraint="veto",
    )
    for left, right, lang in [
        ("iPhone 14", "iPhone 15", "en"),
        ("Windows 10 Pro", "Windows 11 Pro", "en"),
        ("Python 3.11", "Python 3.12", "en"),
        ("Guide Michelin 2025", "Guide Michelin 2026", "fr"),
        ("Logiciel de paie version 2.0", "Logiciel de paie version 2.1", "fr"),
    ]:
        c.add(
            f,
            "products",
            DISTINCT,
            product(left),
            product(right),
            "Different version or edition.",
            lang=lang,
        )
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("payments", "staging", {"retries": 3}, "Payment gateway settings"),
        config("payments", "staging", {"retries": 3}, "Settings for the payment gateway"),
        "Same environment and parameters.",
    )
    c.add(
        f,
        "products",
        DUPLICATE,
        product("iPhone 15 128 GB", "PHN-IP15-128"),
        product("iPhone 15 (128 GB)", "PHN-IP15-128"),
        "Same version; punctuation differs.",
    )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("facturation", "prod", {"tentatives": 3}, "Relance des factures impayées"),
        config("facturation", "recette", {"tentatives": 3}, "Relance des factures impayées"),
        "Production versus acceptance environment.",
        lang="fr",
        constraint="veto",
    )


def _missing_values(c: Corpus) -> None:
    f = "missing_value"
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Amazon", description="Online retail"),
        org("Amazon", "amazon.com", "Online retail"),
        "Absent domain is unknown, not a contradiction.",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Amazon", None, "Online retail"),
        org("Amazon", "amazon.com", "Online retail"),
        "Null domain is unknown, not a contradiction.",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Amazon", "amazon.com", None),
        org("Amazon", "amazon.com", ""),
        "Null and empty descriptions both carry no evidence.",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Amazon"),
        org("Amazon"),
        "Only the name is present on both sides and it is equal.",
        expected_kind="exact",
    )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("", description=None),
        org("", description=None),
        "No informative field: no score, no match.",
    )
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("worker", params={"retries": 0}, description="Job worker"),
        config("worker", params={}, description="Job worker"),
        "Absent parameter is unknown, not a contradiction.",
    )
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("worker", params={"retries": 0}, description="Job worker"),
        config("worker", params={"retries": None}, description="Job worker"),
        "Null parameter is unknown, not a contradiction.",
    )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("worker", params={"retries": 0}, description="Job worker"),
        config("worker", params={"retries": False}, description="Job worker"),
        "Zero and false are different present values.",
        constraint="veto",
    )
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("worker", params={"enabled": False}, description="Job worker"),
        config("worker", params={}, description="Job worker"),
        "Absent flag is unknown, not false.",
    )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("worker", params={"enabled": False}, description="Job worker"),
        config("worker", params={"enabled": 0}, description="Job worker"),
        "False and zero are different present values.",
        constraint="veto",
    )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("worker", params={"prefix": ""}, description="Job worker"),
        config("worker", params={"prefix": "jobs/"}, description="Job worker"),
        "Empty string is a present value that contradicts.",
        constraint="veto",
    )
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("worker", params={"prefix": ""}, description="Job worker"),
        config("worker", params={"prefix": None}, description="Job worker"),
        "Null is unknown and does not contradict an empty string.",
    )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("worker", "", {"retries": 3}, "Job worker"),
        config("worker", "prod", {"retries": 3}, "Job worker"),
        "Empty environment is a present value that contradicts.",
        constraint="veto",
    )
    c.add(
        f,
        "products",
        DUPLICATE,
        product("Wireless mouse M185", "MOU-M185"),
        product("Wireless mouse M185", None, "Compact wireless mouse"),
        "Null SKU and absent description are unknown.",
    )
    c.add(
        f,
        "products",
        DUPLICATE,
        product("Souris sans fil M185", description="Souris compacte"),
        product("Souris sans fil M185", description=None),
        "Null description is unknown.",
        lang="fr",
    )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("export", params={"actif": False}, description="Export comptable nocturne"),
        config("export", params={"actif": 0}, description="Export comptable nocturne"),
        "False and zero are different present values.",
        lang="fr",
        constraint="veto",
    )


def _unicode(c: Corpus) -> None:
    f = "unicode"
    c.add(
        f,
        "strings",
        DUPLICATE,
        "Café de Flore",
        "Café de Flore",
        "NFC and NFD forms of the same text.",
        lang="fr",
        expected_kind="exact",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Nestlé", "nestle.com", "Food and beverages"),
        org("Nestlé", "nestle.com", "Food and beverages"),
        "NFC and NFD forms in a structured record.",
        expected_kind="exact",
    )
    for left, right, lang in [
        ("Société Générale", "Societe Generale", "fr"),
        ("Müller GmbH", "Mueller GmbH", "de"),
        ("Zürich Insurance", "Zurich Insurance", "en"),
        ("Ålesund Fisk AS", "Alesund Fisk AS", "no"),
        ("Straße der Nationen", "Strasse der Nationen", "de"),
        ("\u0130stanbul Havaliman\u0131", "Istanbul Havalimani", "tr"),
        ("Œuvre d'Orient", "Oeuvre d'Orient", "fr"),
        ("\uff21\uff22\uff23 Corp", "ABC Corp", "en"),
        ("ﬁnance team", "finance team", "en"),
        ("IBM", "ibm", "en"),
        ("Яндекс", "Yandex", "ru"),
        ("東京電力", "Tokyo Electric Power", "ja"),
    ]:
        c.add(
            f,
            "strings",
            DUPLICATE,
            left,
            right,
            "Accent, transliteration, width, ligature, case or script variant.",
            lang=lang,
        )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("PayPal", "paypal.com", "Online payments"),
        org("PayPal", "p\u0430ypal.com", "Online payments"),
        "Cyrillic homoglyph in the domain: a different, possibly malicious, host.",
    )
    c.add(
        f,
        "strings",
        DISTINCT,
        "中国银行",
        "中国建设银行",
        "Bank of China versus China Construction Bank share characters.",
        lang="zh",
    )


def _ordering(c: Corpus) -> None:
    f = "ordering"
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("worker", params={"retries": 3, "timeout_ms": 500}, description="Job worker"),
        config("worker", params={"timeout_ms": 500, "retries": 3}, description="Job worker"),
        "Reordered object keys are the same object.",
        expected_kind="exact",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        {
            "id": "stripe-1",
            "tenant_id": "tenant-a",
            "name": "Stripe",
            "domain": "stripe.com",
        },
        {
            "domain": "stripe.com",
            "name": "Stripe",
            "tenant_id": "tenant-a",
            "id": "stripe-2",
        },
        "Reordered record fields are the same record.",
        expected_kind="exact",
        assign_ids=False,
    )
    c.add(
        f,
        "configurations",
        DISTINCT,
        config("cdn", params={"regions": ["eu", "us"]}, description="Edge regions"),
        config("cdn", params={"regions": ["us", "eu"]}, description="Edge regions"),
        "Lists keep their order by default.",
        constraint="veto",
    )
    c.add(
        f,
        "configurations",
        DUPLICATE,
        config("cdn", params={"regions": ["eu", "us"]}, description="Edge regions"),
        config("cdn", params={"regions": ["eu", "us"]}, description="Edge regions"),
        "Same list in the same order.",
        expected_kind="exact",
    )
    for left, right, label, rationale, lang in [
        ("Smith, John", "John Smith", DUPLICATE, "Name written family name first.", "en"),
        ("Black USB-C cable 1 m", "USB-C cable 1 m, black", DUPLICATE, "Reordered tokens.", "en"),
        ("Dog bites man", "Man bites dog", DISTINCT, "Word order changes the meaning.", "en"),
        ("Flight Paris to Lyon", "Flight Lyon to Paris", DISTINCT, "Opposite direction.", "en"),
        ("Vol Paris → Lyon", "Vol Lyon → Paris", DISTINCT, "Opposite direction.", "fr"),
        ("Pomme de terre", "Terre de pomme", DISTINCT, "Word order changes the meaning.", "fr"),
        ("Dupont, Marie", "Marie Dupont", DUPLICATE, "Name written family name first.", "fr"),
    ]:
        c.add(f, "strings", label, left, right, rationale, lang=lang)


def _tenants(c: Corpus) -> None:
    f = "tenant"
    for name, domain, description in [
        ("Amazon", "amazon.com", "Online retail"),
        ("Stripe", "stripe.com", "Online payments"),
        ("Carrefour", "carrefour.fr", "Hypermarchés"),
        ("Acme", "acme.example", "Industrial supplies"),
        ("Globex", "globex.example", "Energy trading"),
        ("Initech", "initech.example", "Software consulting"),
        ("Umbrella", "umbrella.example", "Pharmaceuticals"),
        ("Hooli", "hooli.example", "Search and cloud"),
    ]:
        c.add(
            f,
            "organizations",
            DISTINCT,
            org(name, domain, description, tenant="tenant-a"),
            org(name, domain, description, tenant="tenant-b"),
            "Identical records in different tenants are never compared.",
            constraint="partition",
        )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("Amazon", "amazon.com", "Online retail", tenant="tenant-a"),
        org("Amiazon", "amazon.com", "Online retailer", tenant="tenant-b"),
        "Near duplicates in different tenants are never compared.",
        constraint="partition",
    )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("Acme", "acme.example", tenant="tenant-a"),
        org("Acme", "acme.example", tenant="Tenant-A"),
        "Partition keys are compared exactly; case differs.",
        constraint="partition",
    )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("Acme", "acme.example", tenant="1"),
        org("Acme", "acme.example", tenant=1),
        "Partition keys are typed; string versus integer.",
        constraint="partition",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Amazon", "amazon.com", "Online retail", tenant="tenant-b"),
        org("Amiazon", "amazon.com", "Online retailer", tenant="tenant-b"),
        "Control: same tenant, near duplicate.",
    )
    c.add(
        f,
        "organizations",
        DUPLICATE,
        org("Acme", "acme.example", "Industrial supplies", tenant="tenant-b"),
        org("Acme", "acme.example", "Industrial supplies", tenant="tenant-b"),
        "Control: same tenant, identical records.",
        expected_kind="exact",
    )
    c.add(
        f,
        "organizations",
        INVALID,
        org("Acme", "acme.example", tenant="tenant-a"),
        org("Acme", "acme.example", tenant=ABSENT),
        "A record without its partition key is rejected.",
        expected_error="InvalidRecordError",
    )
    c.add(
        f,
        "organizations",
        DISTINCT,
        org("Boulangerie Dupont", "dupont.example", "Boulangerie artisanale", tenant="client-1"),
        org("Boulangerie Dupont", "dupont.example", "Boulangerie artisanale", tenant="client-2"),
        "Identical records in different tenants are never compared.",
        lang="fr",
        constraint="partition",
    )


def _dense_series(c: Corpus) -> None:
    f = "dense_series"
    for left, right, lang in [
        ("Acme Store #101", "Acme Store #102", "en"),
        ("Meeting room 204", "Meeting room 205", "en"),
        ("Invoice 2026-0001", "Invoice 2026-0002", "en"),
        ("T-shirt, size M", "T-shirt, size L", "en"),
        ("Sneakers, red", "Sneakers, blue", "en"),
        ("Paris 1er arrondissement", "Paris 2e arrondissement", "fr"),
        ("Lot 17", "Lot 71", "fr"),
        ("Bus line 21", "Bus line 12", "en"),
    ]:
        c.add(
            f,
            "products",
            DISTINCT,
            product(left),
            product(right),
            "Members of a dense series differ by one attribute.",
            lang=lang,
        )
    c.add(
        f,
        "products",
        DISTINCT,
        product("Acme Store #101", "STORE-101"),
        product("Acme Store #101", "STORE-110"),
        "Equal names; different SKUs veto the pair.",
        constraint="veto",
    )
    c.add(
        f,
        "products",
        DUPLICATE,
        product("Acme Store #101", "STORE-101"),
        product("Acme store #101", "STORE-101"),
        "Control: same member of the series.",
    )
    for left, right in [("Appartement T2", "Appartement T3"), ("Chambre 12", "Chambre 21")]:
        c.add(
            f,
            "products",
            DISTINCT,
            product(left),
            product(right),
            "Members of a dense series differ by one attribute.",
            lang="fr",
        )


def _short_strings(c: Corpus) -> None:
    f = "short_string"
    for left, right, label, rationale, lang in [
        ("IBM", "I.B.M.", DUPLICATE, "Dotted acronym.", "en"),
        ("3M", "3 M", DUPLICATE, "Inserted space.", "en"),
        ("BP", "B.P.", DUPLICATE, "Dotted acronym.", "en"),
        ("SNCF", "S.N.C.F.", DUPLICATE, "Dotted acronym.", "fr"),
        ("AT&T", "ATT", DUPLICATE, "Dropped ampersand.", "en"),
        ("BMW", "BWM", DUPLICATE, "Transposed letters.", "en"),
        ("Q", "Q", DUPLICATE, "Identical single character.", "en"),
        ("RATP", "R.A.T.P.", DUPLICATE, "Dotted acronym.", "fr"),
        ("BNP", "BPI", DISTINCT, "Different banks with similar acronyms.", "fr"),
        ("HP", "HPE", DISTINCT, "Different companies one letter apart.", "en"),
        ("AA", "AB", DISTINCT, "Different two-letter codes.", "en"),
        ("GE", "GM", DISTINCT, "Different companies one letter apart.", "en"),
        ("X", "Y", DISTINCT, "Different single characters.", "en"),
        ("EDF", "GDF", DISTINCT, "Different companies one letter apart.", "fr"),
        ("UPS", "USPS", DISTINCT, "Different carriers.", "en"),
        ("R2", "R2-D2", DISTINCT, "Code versus longer name.", "en"),
        ("EU", "US", DISTINCT, "Different region codes.", "en"),
    ]:
        c.add(
            f,
            "strings",
            label,
            left,
            right,
            rationale,
            lang=lang,
            expected_kind="exact" if left == right else None,
        )


def _adversarial(c: Corpus) -> None:
    f = "adversarial"
    c.add(f, "strings", DISTINCT, "", "", "Empty strings carry no evidence.")
    c.add(f, "strings", DISTINCT, "   ", "\t", "Blank strings carry no evidence.")
    c.add(f, "strings", DISTINCT, "🍕", "🍔", "Different emoji-only strings.")
    c.add(f, "strings", DUPLICATE, "Ama​zon", "Amazon", "Zero-width space inside a name.")
    c.add(
        f,
        "organizations",
        INVALID,
        org("Acme", "acme.example"),
        ["Acme", "acme.example"],
        "A list is not an organization record.",
        expected_error="InvalidRecordError",
    )
    c.add(
        f,
        "organizations",
        INVALID,
        org(42, "acme.example"),
        org("42", "acme.example"),
        "Text fields reject numbers instead of coercing them.",
        expected_error="InvalidRecordError",
    )
    c.add(
        f,
        "organizations",
        INVALID,
        org(True, "acme.example"),
        org("True", "acme.example"),
        "Text fields reject booleans instead of coercing them.",
        expected_error="InvalidRecordError",
    )
    c.add(
        f,
        "organizations",
        INVALID,
        {"tenant_id": "tenant-a", "name": "Acme"},
        {"id": "acme-2", "tenant_id": "tenant-a", "name": "Acme"},
        "A record without an identifier is rejected when id_field is set.",
        expected_error="InvalidRecordError",
        assign_ids=False,
    )
    c.add(
        f,
        "organizations",
        INVALID,
        {"id": "same", **org("Acme", "acme.example")},
        {"id": "same", **org("Acme Corp", "acme.example")},
        "Two records with the same identifier in one call.",
        expected_error="DuplicateIdError",
        assign_ids=False,
    )
    nested: Any = "deep"
    for _ in range(17):
        nested = {"level": nested}
    c.add(
        f,
        "configurations",
        INVALID,
        config("worker", params=nested, description="Job worker"),
        config("worker", params={"level": "shallow"}, description="Job worker"),
        "Nesting depth 17 exceeds the default limit of 16.",
        expected_error="ResourceLimitError",
    )


def render(cases: list[dict[str, Any]]) -> str:
    return "".join(json.dumps(case, ensure_ascii=False) + "\n" for case in cases)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if the fixture is stale")
    arguments = parser.parse_args()

    content = render(build())
    if arguments.check:
        if not OUTPUT.exists() or OUTPUT.read_text(encoding="utf-8") != content:
            print(f"{OUTPUT} is stale; run tools/build_critical_corpus.py", file=sys.stderr)
            return 1
        return 0
    OUTPUT.write_text(content, encoding="utf-8")
    print(f"wrote {len(content.splitlines())} cases to {OUTPUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

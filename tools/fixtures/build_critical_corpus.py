"""Build tests/fixtures/critical/cases.jsonl from the case table below.

The table is the reviewed source of truth. Case identifiers are written by
hand and never reused or renumbered: a new case takes the next free number of
its family, and a retired case keeps its number out of circulation. Labels
follow docs/reference/duplicate-definitions.md. All records are synthetic.

Usage:
    python tools/fixtures/build_critical_corpus.py          # rewrite fixture and coverage doc
    python tools/fixtures/build_critical_corpus.py --check  # fail if either is stale
"""

import argparse
import dataclasses
import json
import pathlib
import re
import sys
from collections import Counter
from typing import Any

FIXTURE_DIR = pathlib.Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "critical"
OUTPUT = FIXTURE_DIR / "cases.jsonl"
DOC = FIXTURE_DIR.parents[2] / "docs" / "development" / "critical-corpus.md"
COVERAGE_START = "<!-- coverage:start -->"
COVERAGE_END = "<!-- coverage:end -->"

ABSENT: Any = object()

DUPLICATE = "duplicate"
DISTINCT = "distinct"
COMPATIBLE = "compatible"
INVALID = "invalid"

FAMILIES = (
    "typo",
    "semantic_competitor",
    "paraphrased_description",
    "homonym",
    "negation",
    "critical_parameter",
    "version_environment",
    "missing_value",
    "unicode",
    "ordering",
    "tenant",
    "dense_series",
    "short_string",
    "adversarial",
)
CASE_ID = re.compile(r"(?P<family>[a-z_]+)-\d{3}")


@dataclasses.dataclass(frozen=True)
class Case:
    case_id: str
    profile: str
    label: str
    left: Any
    right: Any
    rationale: str
    lang: str = "en"
    constraint: str | None = None
    expected_kind: str | None = None
    expected_error: str | None = None
    assign_ids: bool = True

    @property
    def family(self) -> str:
        match = CASE_ID.fullmatch(self.case_id)
        if match is None:
            raise ValueError(f"malformed case id {self.case_id!r}")
        return match.group("family")


def _record(fields: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in fields.items() if value is not ABSENT}


def org(
    name: Any = ABSENT,
    domain: Any = ABSENT,
    description: Any = ABSENT,
    tenant: Any = "tenant-a",
) -> dict[str, Any]:
    return _record(
        {"tenant_id": tenant, "name": name, "domain": domain, "description": description}
    )


def product(name: Any = ABSENT, sku: Any = ABSENT, description: Any = ABSENT) -> dict[str, Any]:
    return _record({"name": name, "sku": sku, "description": description})


def config(
    service: Any = ABSENT,
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


def nested(depth: int) -> Any:
    value: Any = "deep"
    for _ in range(depth):
        value = {"level": value}
    return value


CASES = [
    # typo
    Case(
        "typo-001", "strings", DUPLICATE, "Amazon", "Amiazon", "Spelling variant of the same name."
    ),
    Case(
        "typo-002",
        "strings",
        DUPLICATE,
        "Microsoft",
        "Microsfot",
        "Spelling variant of the same name.",
    ),
    Case("typo-003", "strings", DUPLICATE, "Google", "Gogle", "Spelling variant of the same name."),
    Case(
        "typo-004",
        "strings",
        DUPLICATE,
        "Salesforce",
        "Salesfroce",
        "Spelling variant of the same name.",
    ),
    Case(
        "typo-005", "strings", DUPLICATE, "Netflix", "Netlfix", "Spelling variant of the same name."
    ),
    Case(
        "typo-006", "strings", DUPLICATE, "Spotify", "Spotfy", "Spelling variant of the same name."
    ),
    Case(
        "typo-007",
        "strings",
        DUPLICATE,
        "Johnson & Johnson",
        "Johnson and Johnson",
        "Spelling variant of the same name.",
    ),
    Case(
        "typo-008",
        "strings",
        DUPLICATE,
        "Galeries Lafayette",
        "Galeries Lafayete",
        "Spelling variant of the same name.",
        lang="fr",
    ),
    Case(
        "typo-009",
        "strings",
        DUPLICATE,
        "Crédit Agricole",
        "Crédit Agricol",
        "Spelling variant of the same name.",
        lang="fr",
    ),
    Case(
        "typo-010",
        "strings",
        DUPLICATE,
        "La Poste",
        "LaPoste",
        "Spelling variant of the same name.",
        lang="fr",
    ),
    Case(
        "typo-011",
        "organizations",
        DUPLICATE,
        org(name="Amazon", domain="amazon.com", description="Online retail"),
        org(name="Amiazon", domain="amazon.com", description="Online retailer"),
        "Typo in the name; same domain and equivalent description.",
    ),
    Case(
        "typo-012",
        "organizations",
        DUPLICATE,
        org(
            name="Shopify",
            domain="shopify.com",
            description="E-commerce platform for online stores",
        ),
        org(
            name="Shopfiy",
            domain="shopify.com",
            description="E-commerce platform for online stores",
        ),
        "Transposed letters; every other field equal.",
    ),
    Case(
        "typo-013",
        "organizations",
        DUPLICATE,
        org(
            name="Airbnb",
            domain="airbnb.com",
            description="Marketplace for short-term home rentals",
        ),
        org(
            name="Air bnb",
            domain="airbnb.com",
            description="Marketplace for short-term home rentals",
        ),
        "Inserted space in the name.",
    ),
    Case(
        "typo-014",
        "organizations",
        DUPLICATE,
        org(name="Deutsche Bank", domain="db.com", description="Investment and retail bank"),
        org(name="Deutsche Bnak", domain="db.com", description="Investment and retail bank"),
        "Transposed letters in a two-word name.",
    ),
    Case(
        "typo-015",
        "organizations",
        DUPLICATE,
        org(
            name="Carrefour",
            domain="carrefour.fr",
            description="Chaîne d'hypermarchés et de supermarchés",
        ),
        org(
            name="Carefour",
            domain="carrefour.fr",
            description="Chaîne d'hypermarchés et de supermarchés",
        ),
        "Missing doubled consonant.",
        lang="fr",
    ),
    Case(
        "typo-016",
        "organizations",
        DUPLICATE,
        org(
            name="Decathlon", domain="decathlon.fr", description="Distributeur d'articles de sport"
        ),
        org(name="Decatlon", domain="decathlon.fr", description="Distributeur d'articles de sport"),
        "Missing letter.",
        lang="fr",
    ),
    Case(
        "typo-017",
        "organizations",
        DUPLICATE,
        org(name="Michelin", domain="michelin.com", description="Fabricant de pneumatiques"),
        org(name="Michellin", domain="michelin.com", description="Fabricant de pneumatiques"),
        "Doubled consonant.",
        lang="fr",
    ),
    Case(
        "typo-018",
        "organizations",
        DUPLICATE,
        org(name="PayPal", domain="paypal.com", description="Online payments"),
        org(name="Paypall", domain="paypal.com", description="Online payment services"),
        "Extra letter and case change.",
    ),
    Case(
        "typo-019",
        "organizations",
        DUPLICATE,
        org(name="Dropbox", domain="dropbox.com", description="Cloud file storage"),
        org(name="Drop box", domain="dropbox.com", description="Cloud file storage and sharing"),
        "Split compound name.",
    ),
    Case(
        "typo-020",
        "products",
        DUPLICATE,
        product(name="USB-C charging cable 1 m", sku="CBL-USBC-1M"),
        product(name="USB-C chargng cable 1 m", sku="CBL-USBC-1M"),
        "Typo in the name; same SKU.",
    ),
    Case(
        "typo-021",
        "products",
        DUPLICATE,
        product(name="Wireless mouse M185", sku="MOU-M185"),
        product(name="Wirless mouse M185", sku="MOU-M185"),
        "Missing letter; same SKU.",
    ),
    Case(
        "typo-022",
        "products",
        DUPLICATE,
        product(name="Stainless steel water bottle 750 ml"),
        product(name="Stainless steel water botle 750 ml"),
        "Typo; no SKU on either side.",
    ),
    Case(
        "typo-023",
        "products",
        DUPLICATE,
        product(name="Ergonomic office chair", sku="CHR-ERGO-01"),
        product(name="Ergonomic ofice chair", sku="CHR-ERGO-01"),
        "Missing letter; same SKU.",
    ),
    Case(
        "typo-024",
        "products",
        DUPLICATE,
        product(name="Chaise de bureau ergonomique"),
        product(name="Chaise de bureau ergonomiqe"),
        "Missing letter.",
        lang="fr",
    ),
    # semantic_competitor
    Case(
        "semantic_competitor-001",
        "strings",
        DISTINCT,
        "Amazon",
        "Microsoft",
        "Competitors in the same market.",
    ),
    Case(
        "semantic_competitor-002",
        "strings",
        DISTINCT,
        "Coca-Cola",
        "PepsiCo",
        "Competitors in the same market.",
    ),
    Case(
        "semantic_competitor-003",
        "strings",
        DISTINCT,
        "Visa",
        "Mastercard",
        "Competitors in the same market.",
    ),
    Case(
        "semantic_competitor-004",
        "strings",
        DISTINCT,
        "Nike",
        "Adidas",
        "Competitors in the same market.",
    ),
    Case(
        "semantic_competitor-005",
        "strings",
        DISTINCT,
        "Uber",
        "Lyft",
        "Competitors in the same market.",
    ),
    Case(
        "semantic_competitor-006",
        "strings",
        DISTINCT,
        "Airbus",
        "Boeing",
        "Competitors in the same market.",
    ),
    Case(
        "semantic_competitor-007",
        "strings",
        DISTINCT,
        "Intel",
        "AMD",
        "Competitors in the same market.",
    ),
    Case(
        "semantic_competitor-008",
        "organizations",
        DISTINCT,
        org(
            name="Apple",
            domain="apple.com",
            description="Consumer electronics, smartphones and computers",
        ),
        org(
            name="Samsung Electronics",
            domain="samsung.com",
            description="Consumer electronics, smartphones and TVs",
        ),
        "Near-identical descriptions; different companies.",
    ),
    Case(
        "semantic_competitor-009",
        "organizations",
        DISTINCT,
        org(
            name="BNP Paribas",
            domain="bnpparibas.com",
            description="Banque de détail et d'investissement",
        ),
        org(
            name="Société Générale",
            domain="societegenerale.com",
            description="Banque de détail et d'investissement",
        ),
        "Identical descriptions; different banks.",
        lang="fr",
    ),
    Case(
        "semantic_competitor-010",
        "organizations",
        DISTINCT,
        org(name="Renault", domain="renault.fr", description="Constructeur automobile français"),
        org(name="Peugeot", domain="peugeot.fr", description="Constructeur automobile français"),
        "Identical descriptions; different manufacturers.",
        lang="fr",
    ),
    Case(
        "semantic_competitor-011",
        "organizations",
        DISTINCT,
        org(
            name="Carrefour",
            domain="carrefour.fr",
            description="Chaîne d'hypermarchés et de supermarchés",
        ),
        org(
            name="Auchan",
            domain="auchan.fr",
            description="Chaîne d'hypermarchés et de supermarchés",
        ),
        "Identical descriptions; different retailers.",
        lang="fr",
    ),
    Case(
        "semantic_competitor-012",
        "organizations",
        DISTINCT,
        org(name="Spotify", domain="spotify.com", description="Music streaming service"),
        org(name="Deezer", domain="deezer.com", description="Music streaming service"),
        "Identical descriptions; different services.",
    ),
    Case(
        "semantic_competitor-013",
        "products",
        DISTINCT,
        product(name="Slack", description="Team messaging and collaboration app"),
        product(name="Microsoft Teams", description="Team chat and collaboration app"),
        "Competing products with paraphrased descriptions.",
    ),
    Case(
        "semantic_competitor-014",
        "products",
        DISTINCT,
        product(name="Zoom Meetings", description="Video conferencing software"),
        product(name="Google Meet", description="Video conferencing software"),
        "Competing products with identical descriptions.",
    ),
    Case(
        "semantic_competitor-015",
        "organizations",
        DISTINCT,
        org(
            name="Free",
            domain="free.fr",
            description="Fournisseur d'accès à internet et opérateur mobile",
        ),
        org(
            name="SFR",
            domain="sfr.fr",
            description="Opérateur mobile et fournisseur d'accès à internet",
        ),
        "Reworded identical activity; different operators.",
        lang="fr",
    ),
    # paraphrased_description
    Case(
        "paraphrased_description-001",
        "organizations",
        DUPLICATE,
        org(
            name="Stripe",
            domain="stripe.com",
            description="Payment processing for internet businesses",
        ),
        org(
            name="Stripe",
            domain="stripe.com",
            description="Online payments infrastructure for companies",
        ),
        "Same record; only the description is reworded.",
    ),
    Case(
        "paraphrased_description-002",
        "organizations",
        DUPLICATE,
        org(
            name="Doctolib",
            domain="doctolib.fr",
            description="Prise de rendez-vous médicaux en ligne",
        ),
        org(
            name="Doctolib",
            domain="doctolib.fr",
            description="Plateforme de réservation de consultations médicales",
        ),
        "Same record; only the description is reworded.",
        lang="fr",
    ),
    Case(
        "paraphrased_description-003",
        "organizations",
        DUPLICATE,
        org(name="BlaBlaCar", domain="blablacar.fr", description="Covoiturage longue distance"),
        org(
            name="BlaBlaCar", domain="blablacar.fr", description="Long-distance carpooling platform"
        ),
        "Same record; only the description is reworded.",
        lang="mixed",
    ),
    Case(
        "paraphrased_description-004",
        "organizations",
        DUPLICATE,
        org(
            name="Atlassian",
            domain="atlassian.com",
            description="Software for team collaboration and tracking",
        ),
        org(
            name="Atlassian",
            domain="atlassian.com",
            description="Tools that help teams plan and track work",
        ),
        "Same record; only the description is reworded.",
    ),
    Case(
        "paraphrased_description-005",
        "organizations",
        DUPLICATE,
        org(name="Ubisoft", domain="ubisoft.com", description="Éditeur de jeux vidéo"),
        org(
            name="Ubisoft", domain="ubisoft.com", description="Développeur et éditeur de jeux vidéo"
        ),
        "Same record; only the description is reworded.",
        lang="fr",
    ),
    Case(
        "paraphrased_description-006",
        "products",
        DUPLICATE,
        product(
            name="Noise-cancelling headphones WH-1000XM5",
            sku="AUD-WH1000XM5",
            description="Over-ear wireless headphones with active noise cancellation",
        ),
        product(
            name="Noise-cancelling headphones WH-1000XM5",
            sku="AUD-WH1000XM5",
            description="Wireless over-ear headset featuring ANC",
        ),
        "Same record; only the description is reworded.",
    ),
    Case(
        "paraphrased_description-007",
        "products",
        DUPLICATE,
        product(
            name="Robot pâtissier 5 L",
            sku="KIT-5L",
            description="Robot de cuisine pour pétrir et fouetter",
        ),
        product(
            name="Robot pâtissier 5 L",
            sku="KIT-5L",
            description="Batteur sur socle pour pâtes et blancs",
        ),
        "Same record; only the description is reworded.",
        lang="fr",
    ),
    Case(
        "paraphrased_description-008",
        "products",
        DUPLICATE,
        product(
            name="Trail running shoes",
            sku="SHO-TR-42",
            description="Lightweight shoes for off-road running",
        ),
        product(
            name="Trail running shoes",
            sku="SHO-TR-42",
            description="Light footwear designed for trail runs",
        ),
        "Same record; only the description is reworded.",
    ),
    Case(
        "paraphrased_description-009",
        "products",
        DUPLICATE,
        product(
            name="Lampe de bureau LED",
            sku="LMP-LED-02",
            description="Lampe orientable à faible consommation",
        ),
        product(
            name="Lampe de bureau LED",
            sku="LMP-LED-02",
            description="Adjustable low-energy desk lamp",
        ),
        "Same record; only the description is reworded.",
        lang="mixed",
    ),
    Case(
        "paraphrased_description-010",
        "products",
        DUPLICATE,
        product(
            name="4-slot toaster", sku="TST-4", description="Toasts four slices of bread at once"
        ),
        product(
            name="4-slot toaster",
            sku="TST-4",
            description="Toaster that browns up to four slices together",
        ),
        "Same record; only the description is reworded.",
    ),
    Case(
        "paraphrased_description-011",
        "configurations",
        DUPLICATE,
        config(
            service="billing-api",
            params={"retries": 3},
            description="Retries failed payment webhooks",
        ),
        config(
            service="billing-api",
            params={"retries": 3},
            description="Re-sends payment webhooks that failed",
        ),
        "Same record; only the description is reworded.",
    ),
    Case(
        "paraphrased_description-012",
        "configurations",
        DUPLICATE,
        config(
            service="search",
            params={"timeout_ms": 500},
            description="Abandonne la requête de recherche après 500 ms",
        ),
        config(
            service="search",
            params={"timeout_ms": 500},
            description="Coupe les recherches qui dépassent une demi-seconde",
        ),
        "Same record; only the description is reworded.",
        lang="fr",
    ),
    Case(
        "paraphrased_description-013",
        "configurations",
        DUPLICATE,
        config(
            service="mailer",
            params={"batch_size": 100},
            description="Sends newsletters in batches of one hundred",
        ),
        config(
            service="mailer",
            params={"batch_size": 100},
            description="Newsletter delivery grouped by 100 recipients",
        ),
        "Same record; only the description is reworded.",
    ),
    Case(
        "paraphrased_description-014",
        "configurations",
        DUPLICATE,
        config(
            service="cdn", params={"ttl_s": 3600}, description="Caches static assets for one hour"
        ),
        config(
            service="cdn", params={"ttl_s": 3600}, description="Static files stay cached 60 minutes"
        ),
        "Same record; only the description is reworded.",
    ),
    Case(
        "paraphrased_description-015",
        "configurations",
        DUPLICATE,
        config(
            service="auth",
            params={"session_days": 30},
            description="Les sessions expirent au bout d'un mois",
        ),
        config(
            service="auth",
            params={"session_days": 30},
            description="User sessions expire after thirty days",
        ),
        "Same record; only the description is reworded.",
        lang="mixed",
    ),
    Case(
        "paraphrased_description-016",
        "products",
        DUPLICATE,
        product(
            name="Cordless drill 18 V", description="18-volt drill supplied with two batteries"
        ),
        product(
            name="Battery-powered drill driver",
            description="Drill/driver running on 18 V batteries, two packs included",
        ),
        "Same product described with different words; no shared identifier.",
    ),
    Case(
        "paraphrased_description-017",
        "products",
        DISTINCT,
        product(name="Corded drill 750 W", description="Mains-powered drill, 750 watts"),
        product(
            name="Cordless drill 18 V", description="18-volt drill supplied with two batteries"
        ),
        "Near negative: corded versus cordless drill.",
    ),
    Case(
        "paraphrased_description-018",
        "products",
        DUPLICATE,
        product(
            name="Electric kettle 1.7 L", description="Boils water quickly with automatic shut-off"
        ),
        product(
            name="1.7-litre electric water boiler",
            description="Heats water fast and switches itself off",
        ),
        "Same product described with different words; no shared identifier.",
    ),
    Case(
        "paraphrased_description-019",
        "products",
        DISTINCT,
        product(
            name="Electric kettle 1.7 L", description="Boils water quickly with automatic shut-off"
        ),
        product(
            name="Electric kettle 1.0 L",
            description="Compact kettle for one litre with automatic shut-off",
        ),
        "Near negative: different capacity.",
    ),
    Case(
        "paraphrased_description-020",
        "products",
        DUPLICATE,
        product(
            name="Aspirateur balai sans fil",
            description="Aspirateur léger sur batterie pour les sols",
        ),
        product(
            name="Balai aspirant rechargeable",
            description="Aspirateur vertical autonome fonctionnant sur batterie",
        ),
        "Same product described with different words; no shared identifier.",
        lang="fr",
    ),
    Case(
        "paraphrased_description-021",
        "products",
        DISTINCT,
        product(
            name="Aspirateur balai sans fil",
            description="Aspirateur léger sur batterie pour les sols",
        ),
        product(
            name="Aspirateur traîneau filaire", description="Aspirateur à sac branché sur secteur"
        ),
        "Near negative: corded canister versus cordless stick vacuum.",
        lang="fr",
    ),
    Case(
        "paraphrased_description-022",
        "organizations",
        DUPLICATE,
        org(
            name="Société nationale des chemins de fer français",
            description="Opérateur ferroviaire public français",
        ),
        org(name="SNCF", description="Compagnie nationale des chemins de fer en France"),
        "Full name versus acronym; no domain on either side.",
        lang="fr",
    ),
    Case(
        "paraphrased_description-023",
        "organizations",
        DISTINCT,
        org(name="SNCF", description="Compagnie nationale des chemins de fer en France"),
        org(name="RATP", description="Opérateur public des transports parisiens"),
        "Near negative: another public transport operator.",
        lang="fr",
    ),
    Case(
        "paraphrased_description-024",
        "strings",
        DUPLICATE,
        "Wireless noise-cancelling headphones",
        "Bluetooth headset with active noise cancellation",
        "Same product type described with different words.",
    ),
    Case(
        "paraphrased_description-025",
        "strings",
        DISTINCT,
        "Wireless noise-cancelling headphones",
        "Wired headphones without noise cancellation",
        "Near negative: opposite connectivity and feature.",
    ),
    # homonym
    Case(
        "homonym-001",
        "organizations",
        DISTINCT,
        org(name="Delta", domain="delta.com", description="Airline"),
        org(name="Delta", domain="deltafaucet.com", description="Kitchen and bathroom faucets"),
        "Same name; different domain and activity.",
    ),
    Case(
        "homonym-002",
        "organizations",
        DISTINCT,
        org(name="Apple", domain="apple.com", description="Consumer electronics"),
        org(
            name="Apple",
            domain="apple-corps.example",
            description="Record label and music publisher",
        ),
        "Same name; different domain and activity.",
    ),
    Case(
        "homonym-003",
        "organizations",
        DISTINCT,
        org(name="Orange", domain="orange.fr", description="Opérateur de télécommunications"),
        org(name="Orange", domain="orange-jus.example", description="Marque de jus de fruits"),
        "Same name; different domain and activity.",
        lang="fr",
    ),
    Case(
        "homonym-004",
        "organizations",
        DISTINCT,
        org(name="Mercury", domain="mercury.com", description="Banking for startups"),
        org(name="Mercury", domain="mercury-insurance.example", description="Car insurance"),
        "Same name; different domain and activity.",
    ),
    Case(
        "homonym-005",
        "organizations",
        DISTINCT,
        org(name="Jaguar", domain="jaguar.com", description="Luxury car manufacturer"),
        org(name="Jaguar", domain="jaguar-sound.example", description="Audio equipment retailer"),
        "Same name; different domain and activity.",
    ),
    Case(
        "homonym-006",
        "organizations",
        DISTINCT,
        org(name="Dove", domain="dove.com", description="Soap and personal care"),
        org(name="Dove", domain="dovechocolate.example", description="Chocolate confectionery"),
        "Same name; different domain and activity.",
    ),
    Case(
        "homonym-007",
        "organizations",
        DISTINCT,
        org(name="Lincoln", domain="lincoln.com", description="Luxury vehicles"),
        org(name="Lincoln", domain="lincolnelectric.example", description="Welding equipment"),
        "Same name; different domain and activity.",
    ),
    Case(
        "homonym-008",
        "organizations",
        DISTINCT,
        org(
            name="Atlas",
            domain="atlas-fret.example",
            description="Transport routier de marchandises",
        ),
        org(
            name="Atlas", domain="atlas-editions.example", description="Maison d'édition de cartes"
        ),
        "Same name; different domain and activity.",
        lang="fr",
    ),
    Case(
        "homonym-009",
        "organizations",
        DISTINCT,
        org(
            name="Phoenix", domain="phoenix-labs.example", description="Clinical laboratory testing"
        ),
        org(name="Phoenix", domain="phoenix-games.example", description="Video game studio"),
        "Same name; different domain and activity.",
    ),
    Case(
        "homonym-010",
        "organizations",
        DISTINCT,
        org(name="Total", domain="totalenergies.example", description="Énergie et carburants"),
        org(name="Total", domain="total-nutrition.example", description="Compléments alimentaires"),
        "Same name; different domain and activity.",
        lang="fr",
    ),
    Case(
        "homonym-011",
        "products",
        DISTINCT,
        product(name="Galaxy", sku="PHN-GALAXY", description="Android smartphone"),
        product(name="Galaxy", sku="CHOC-GALAXY", description="Milk chocolate bar"),
        "Same name; different SKU and category.",
        constraint="veto",
    ),
    Case(
        "homonym-012",
        "products",
        DISTINCT,
        product(name="Mustang", description="Sports car scale model 1:18"),
        product(name="Mustang", description="Leather saddle for horse riding"),
        "Same name; unrelated products, no SKU.",
    ),
    Case(
        "homonym-013",
        "products",
        DISTINCT,
        product(name="Mistral", description="Ventilateur colonne 3 vitesses"),
        product(name="Mistral", description="Voilier de croisière 9 mètres"),
        "Same name; unrelated products, no SKU.",
        lang="fr",
    ),
    Case(
        "homonym-014",
        "strings",
        DISTINCT,
        "Python (programming language)",
        "Python (snake)",
        "Same head word; different meaning.",
    ),
    Case(
        "homonym-015",
        "strings",
        DISTINCT,
        "Avocat (fruit)",
        "Avocat (profession)",
        "Same head word; different meaning.",
        lang="fr",
    ),
    Case(
        "homonym-016",
        "strings",
        DISTINCT,
        "Paris, France",
        "Paris, Texas",
        "Same city name; different places.",
    ),
    Case(
        "homonym-017",
        "organizations",
        DUPLICATE,
        org(name="Amazon", domain="amazon.com", description="Online retail"),
        org(name="Amazon", domain="amazon.fr", description="Online retail"),
        "Same organization under two country domains; a different domain is evidence, not a veto.",
    ),
    Case(
        "homonym-018",
        "organizations",
        DUPLICATE,
        org(
            name="Decathlon", domain="decathlon.fr", description="Distributeur d'articles de sport"
        ),
        org(
            name="Decathlon", domain="decathlon.com", description="Distributeur d'articles de sport"
        ),
        "Same organization under two domains.",
        lang="fr",
    ),
    Case(
        "homonym-019",
        "organizations",
        DUPLICATE,
        org(name="Google", domain="google.com", description="Search engine and online advertising"),
        org(
            name="Google LLC",
            domain="about.google",
            description="Web search and advertising company",
        ),
        "Same organization; different domain and reworded description.",
    ),
    # negation
    Case(
        "negation-001",
        "strings",
        DISTINCT,
        "Laptop with charger",
        "Laptop without charger",
        "Negated or opposite attribute.",
    ),
    Case(
        "negation-002",
        "strings",
        DISTINCT,
        "Supports two-factor authentication",
        "Does not support two-factor authentication",
        "Negated or opposite attribute.",
    ),
    Case(
        "negation-003",
        "strings",
        DISTINCT,
        "Gluten-free bread",
        "Bread with gluten",
        "Negated or opposite attribute.",
    ),
    Case(
        "negation-004",
        "strings",
        DISTINCT,
        "Waterproof jacket",
        "Non-waterproof jacket",
        "Negated or opposite attribute.",
    ),
    Case(
        "negation-005",
        "strings",
        DISTINCT,
        "Alcohol-free beer",
        "Beer",
        "Negated or opposite attribute.",
    ),
    Case(
        "negation-006",
        "strings",
        DISTINCT,
        "Price includes VAT",
        "Price excludes VAT",
        "Negated or opposite attribute.",
    ),
    Case(
        "negation-007",
        "strings",
        DISTINCT,
        "Yaourt avec sucre",
        "Yaourt sans sucre",
        "Negated or opposite attribute.",
        lang="fr",
    ),
    Case(
        "negation-008",
        "strings",
        DISTINCT,
        "Compte activé",
        "Compte désactivé",
        "Negated or opposite attribute.",
        lang="fr",
    ),
    Case(
        "negation-009",
        "strings",
        DISTINCT,
        "Livraison incluse",
        "Livraison non incluse",
        "Negated or opposite attribute.",
        lang="fr",
    ),
    Case(
        "negation-010",
        "strings",
        DISTINCT,
        "Fumeur autorisé",
        "Fumeur interdit",
        "Negated or opposite attribute.",
        lang="fr",
    ),
    Case(
        "negation-011",
        "strings",
        DISTINCT,
        "Animaux acceptés",
        "Animaux non acceptés",
        "Negated or opposite attribute.",
        lang="fr",
    ),
    Case(
        "negation-012",
        "configurations",
        DISTINCT,
        config(service="storage", description="Feature flag enabled for checkout"),
        config(service="storage", description="Feature flag disabled for checkout"),
        "Opposite meaning in an otherwise identical description.",
    ),
    Case(
        "negation-013",
        "configurations",
        DISTINCT,
        config(service="storage", description="Allow public read access to the bucket"),
        config(service="storage", description="Deny public read access to the bucket"),
        "Opposite meaning in an otherwise identical description.",
    ),
    Case(
        "negation-014",
        "configurations",
        DISTINCT,
        config(service="storage", description="Encryption at rest is enabled"),
        config(service="storage", description="Encryption at rest is not enabled"),
        "Opposite meaning in an otherwise identical description.",
    ),
    Case(
        "negation-015",
        "configurations",
        DISTINCT,
        config(service="cache", description="Cache activé pour les pages produit"),
        config(service="cache", description="Cache désactivé pour les pages produit"),
        "Opposite meaning in an otherwise identical description.",
        lang="fr",
    ),
    Case(
        "negation-016",
        "configurations",
        DISTINCT,
        config(service="checkout", params={"enabled": True}, description="Express checkout"),
        config(service="checkout", params={"enabled": False}, description="Express checkout"),
        "Typed boolean parameter contradicts.",
        constraint="veto",
    ),
    # critical_parameter
    Case(
        "critical_parameter-001",
        "configurations",
        DISTINCT,
        config(service="worker", params={"retries": 0}, description="Background job worker"),
        config(service="worker", params={"retries": 3}, description="Background job worker"),
        "Different retry count.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-002",
        "configurations",
        DISTINCT,
        config(service="worker", params={"timeout_ms": 1000}, description="Background job worker"),
        config(service="worker", params={"timeout_ms": 100}, description="Background job worker"),
        "Different timeout.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-003",
        "configurations",
        DISTINCT,
        config(
            service="worker", params={"max_connections": 10}, description="Background job worker"
        ),
        config(
            service="worker", params={"max_connections": 100}, description="Background job worker"
        ),
        "Different pool size.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-004",
        "configurations",
        DISTINCT,
        config(service="worker", params={"tls": True}, description="Background job worker"),
        config(service="worker", params={"tls": False}, description="Background job worker"),
        "TLS on versus off.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-005",
        "configurations",
        DISTINCT,
        config(
            service="worker", params={"region": "eu-west-1"}, description="Background job worker"
        ),
        config(
            service="worker", params={"region": "us-east-1"}, description="Background job worker"
        ),
        "Different region.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-006",
        "configurations",
        DISTINCT,
        config(service="worker", params={"port": 5432}, description="Background job worker"),
        config(service="worker", params={"port": 5433}, description="Background job worker"),
        "Different port.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-007",
        "configurations",
        DISTINCT,
        config(service="worker", params={"log_level": "info"}, description="Background job worker"),
        config(
            service="worker", params={"log_level": "debug"}, description="Background job worker"
        ),
        "Different log level.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-008",
        "configurations",
        DISTINCT,
        config(service="worker", params={"retries": 3}, description="Background job worker"),
        config(service="worker", params={"retries": "3"}, description="Background job worker"),
        "Integer versus string: types differ.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-009",
        "configurations",
        DISTINCT,
        config(service="worker", params={"tls": True}, description="Background job worker"),
        config(service="worker", params={"tls": 1}, description="Background job worker"),
        "Boolean versus integer: types differ.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-010",
        "configurations",
        DISTINCT,
        config(service="worker", params={"ratio": 0.5}, description="Background job worker"),
        config(service="worker", params={"ratio": 0.05}, description="Background job worker"),
        "Different float value.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-011",
        "configurations",
        DISTINCT,
        config(service="worker", params={"timeout": "1 s"}, description="Background job worker"),
        config(
            service="worker", params={"timeout": "1000 ms"}, description="Background job worker"
        ),
        "No unit conversion is declared.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-012",
        "configurations",
        DUPLICATE,
        config(
            service="worker", params={"retries": 3, "timeout_ms": 1000}, description="Job worker"
        ),
        config(
            service="worker", params={"retries": 3, "timeout_ms": 1000}, description="Job worker"
        ),
        "Every field equal.",
        expected_kind="exact",
    ),
    Case(
        "critical_parameter-013",
        "configurations",
        DUPLICATE,
        config(service="worker", params={"replicas": 3}, description="Runs three worker replicas"),
        config(
            service="worker", params={"replicas": 3}, description="Three replicas of the worker"
        ),
        "Equal parameters; reworded description.",
    ),
    Case(
        "critical_parameter-014",
        "configurations",
        DUPLICATE,
        config(
            service="worker", params={"ratio": 0.5}, description="Échantillonne la moitié des jobs"
        ),
        config(
            service="worker", params={"ratio": 0.5}, description="Échantillonnage de 50 % des jobs"
        ),
        "Equal parameters; reworded description.",
        lang="fr",
    ),
    Case(
        "critical_parameter-015",
        "products",
        DISTINCT,
        product(name="SSD 1 TB", sku="SSD-1TB", description="NVMe solid-state drive"),
        product(name="SSD 2 TB", sku="SSD-2TB", description="NVMe solid-state drive"),
        "Different capacity and SKU.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-016",
        "configurations",
        DISTINCT,
        config(service="worker", params={"log_level": "INFO"}, description="Background job worker"),
        config(service="worker", params={"log_level": "info"}, description="Background job worker"),
        "Parameters are compared exactly; case is not folded.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-017",
        "configurations",
        DISTINCT,
        config(service="Billing", params={"retries": 3}, description="Background job worker"),
        config(service="billing", params={"retries": 3}, description="Background job worker"),
        "Discriminant fields are compared exactly; case is not folded.",
        constraint="veto",
    ),
    Case(
        "critical_parameter-018",
        "configurations",
        DISTINCT,
        config(
            service="worker",
            params={"db": {"host": "db1", "port": 5432}},
            description="Background job worker",
        ),
        config(
            service="worker",
            params={"db": {"host": "db1", "port": 5433}},
            description="Background job worker",
        ),
        "Nested parameters are compared key by key; a different port is a veto.",
        constraint="veto",
    ),
    # version_environment
    Case(
        "version_environment-001",
        "configurations",
        DISTINCT,
        config(service="payments", params={"retries": 3}, description="Payment gateway settings"),
        config(
            service="payments",
            environment="staging",
            params={"retries": 3},
            description="Payment gateway settings",
        ),
        "Production versus staging.",
        constraint="veto",
    ),
    Case(
        "version_environment-002",
        "configurations",
        DISTINCT,
        config(service="payments", params={"retries": 3}, description="Payment gateway settings"),
        config(
            service="payments",
            environment="dev",
            params={"retries": 3},
            description="Payment gateway settings",
        ),
        "Production versus development.",
        constraint="veto",
    ),
    Case(
        "version_environment-003",
        "configurations",
        DISTINCT,
        config(
            service="payments",
            environment="staging",
            params={"retries": 3},
            description="Payment gateway settings",
        ),
        config(
            service="payments",
            environment="dev",
            params={"retries": 3},
            description="Payment gateway settings",
        ),
        "Staging versus development.",
        constraint="veto",
    ),
    Case(
        "version_environment-004",
        "configurations",
        DISTINCT,
        config(service="payments", params={"retries": 3}, description="Payment gateway settings"),
        config(
            service="payments",
            environment="production",
            params={"retries": 3},
            description="Payment gateway settings",
        ),
        "No environment alias is declared.",
        constraint="veto",
    ),
    Case(
        "version_environment-005",
        "configurations",
        DISTINCT,
        config(
            service="payments",
            environment="eu-prod",
            params={"retries": 3},
            description="Payment gateway settings",
        ),
        config(
            service="payments",
            environment="us-prod",
            params={"retries": 3},
            description="Payment gateway settings",
        ),
        "Different regional environment.",
        constraint="veto",
    ),
    Case(
        "version_environment-006",
        "configurations",
        DISTINCT,
        config(
            service="payments-v1", params={"retries": 3}, description="Payment gateway settings"
        ),
        config(
            service="payments-v2", params={"retries": 3}, description="Payment gateway settings"
        ),
        "Different service version.",
        constraint="veto",
    ),
    Case(
        "version_environment-007",
        "configurations",
        DISTINCT,
        config(service="api", params={"version": "1.4.2"}, description="Public API gateway"),
        config(service="api", params={"version": "1.4.3"}, description="Public API gateway"),
        "Different patch version.",
        constraint="veto",
    ),
    Case(
        "version_environment-008",
        "products",
        DISTINCT,
        product(name="iPhone 14"),
        product(name="iPhone 15"),
        "Different version or edition.",
    ),
    Case(
        "version_environment-009",
        "products",
        DISTINCT,
        product(name="Windows 10 Pro"),
        product(name="Windows 11 Pro"),
        "Different version or edition.",
    ),
    Case(
        "version_environment-010",
        "products",
        DISTINCT,
        product(name="Python 3.11"),
        product(name="Python 3.12"),
        "Different version or edition.",
    ),
    Case(
        "version_environment-011",
        "products",
        DISTINCT,
        product(name="Guide Michelin 2025"),
        product(name="Guide Michelin 2026"),
        "Different version or edition.",
        lang="fr",
    ),
    Case(
        "version_environment-012",
        "products",
        DISTINCT,
        product(name="Logiciel de paie version 2.0"),
        product(name="Logiciel de paie version 2.1"),
        "Different version or edition.",
        lang="fr",
    ),
    Case(
        "version_environment-013",
        "configurations",
        DUPLICATE,
        config(
            service="payments",
            environment="staging",
            params={"retries": 3},
            description="Payment gateway settings",
        ),
        config(
            service="payments",
            environment="staging",
            params={"retries": 3},
            description="Settings for the payment gateway",
        ),
        "Same environment and parameters.",
    ),
    Case(
        "version_environment-014",
        "products",
        DUPLICATE,
        product(name="iPhone 15 128 GB", sku="PHN-IP15-128"),
        product(name="iPhone 15 (128 GB)", sku="PHN-IP15-128"),
        "Same version; punctuation differs.",
    ),
    Case(
        "version_environment-015",
        "configurations",
        DISTINCT,
        config(
            service="facturation",
            params={"tentatives": 3},
            description="Relance des factures impayées",
        ),
        config(
            service="facturation",
            environment="recette",
            params={"tentatives": 3},
            description="Relance des factures impayées",
        ),
        "Production versus acceptance environment.",
        lang="fr",
        constraint="veto",
    ),
    # missing_value
    Case(
        "missing_value-001",
        "organizations",
        DUPLICATE,
        org(name="Amazon", description="Online retail"),
        org(name="Amazon", domain="amazon.com", description="Online retail"),
        "Absent domain is unknown, not a contradiction.",
    ),
    Case(
        "missing_value-002",
        "organizations",
        DUPLICATE,
        org(name="Amazon", domain=None, description="Online retail"),
        org(name="Amazon", domain="amazon.com", description="Online retail"),
        "Null domain is unknown, not a contradiction.",
    ),
    Case(
        "missing_value-003",
        "organizations",
        DUPLICATE,
        org(name="Amazon", domain="amazon.com", description=None),
        org(name="Amazon", domain="amazon.com", description=""),
        "Null and empty descriptions both carry no evidence.",
    ),
    Case(
        "missing_value-004",
        "organizations",
        DUPLICATE,
        org(name="Amazon"),
        org(name="Amazon"),
        "Only the name is present on both sides and it is equal.",
        expected_kind="exact",
    ),
    Case(
        "missing_value-005",
        "organizations",
        DISTINCT,
        org(name="", description=None),
        org(name="", description=None),
        "No informative field: no score, no match.",
    ),
    Case(
        "missing_value-006",
        "configurations",
        COMPATIBLE,
        config(service="worker", params={"retries": 0}, description="Job worker"),
        config(service="worker", params={}, description="Job worker"),
        "Compatible, not proven: a missing parameter may stand for another default.",
    ),
    Case(
        "missing_value-007",
        "configurations",
        COMPATIBLE,
        config(service="worker", params={"retries": 0}, description="Job worker"),
        config(service="worker", params={"retries": None}, description="Job worker"),
        "Compatible, not proven: a missing parameter may stand for another default.",
    ),
    Case(
        "missing_value-008",
        "configurations",
        DISTINCT,
        config(service="worker", params={"retries": 0}, description="Job worker"),
        config(service="worker", params={"retries": False}, description="Job worker"),
        "Zero and false are different present values.",
        constraint="veto",
    ),
    Case(
        "missing_value-009",
        "configurations",
        COMPATIBLE,
        config(service="worker", params={"enabled": False}, description="Job worker"),
        config(service="worker", params={}, description="Job worker"),
        "Compatible, not proven: a missing parameter may stand for another default.",
    ),
    Case(
        "missing_value-010",
        "configurations",
        DISTINCT,
        config(service="worker", params={"enabled": False}, description="Job worker"),
        config(service="worker", params={"enabled": 0}, description="Job worker"),
        "False and zero are different present values.",
        constraint="veto",
    ),
    Case(
        "missing_value-011",
        "configurations",
        DISTINCT,
        config(service="worker", params={"prefix": ""}, description="Job worker"),
        config(service="worker", params={"prefix": "jobs/"}, description="Job worker"),
        "Empty string is a present value that contradicts.",
        constraint="veto",
    ),
    Case(
        "missing_value-012",
        "configurations",
        COMPATIBLE,
        config(service="worker", params={"prefix": ""}, description="Job worker"),
        config(service="worker", params={"prefix": None}, description="Job worker"),
        "Compatible, not proven: a missing parameter may stand for another default.",
    ),
    Case(
        "missing_value-013",
        "configurations",
        DISTINCT,
        config(service="worker", environment="", params={"retries": 3}, description="Job worker"),
        config(service="worker", params={"retries": 3}, description="Job worker"),
        "Empty environment is a present value that contradicts.",
        constraint="veto",
    ),
    Case(
        "missing_value-014",
        "products",
        DUPLICATE,
        product(name="Wireless mouse M185", sku="MOU-M185"),
        product(name="Wireless mouse M185", sku=None, description="Compact wireless mouse"),
        "Null SKU and absent description are unknown.",
    ),
    Case(
        "missing_value-015",
        "products",
        DUPLICATE,
        product(name="Souris sans fil M185", description="Souris compacte"),
        product(name="Souris sans fil M185", description=None),
        "Null description is unknown.",
        lang="fr",
    ),
    Case(
        "missing_value-016",
        "configurations",
        DISTINCT,
        config(service="export", params={"actif": False}, description="Export comptable nocturne"),
        config(service="export", params={"actif": 0}, description="Export comptable nocturne"),
        "False and zero are different present values.",
        lang="fr",
        constraint="veto",
    ),
    Case(
        "missing_value-017",
        "configurations",
        COMPATIBLE,
        config(
            service="worker",
            params={"db": {"host": "db1", "port": None}},
            description="Job worker",
        ),
        config(
            service="worker",
            params={"db": {"host": "db1", "port": 5432}},
            description="Job worker",
        ),
        "A nested null is unknown at every depth: compatible, not a contradiction.",
    ),
    # unicode
    Case(
        "unicode-001",
        "strings",
        DUPLICATE,
        "Café de Flore",
        "Cafe\u0301 de Flore",
        "NFC and NFD forms of the same text.",
        lang="fr",
        expected_kind="exact",
    ),
    Case(
        "unicode-002",
        "organizations",
        DUPLICATE,
        org(name="Nestlé", domain="nestle.com", description="Food and beverages"),
        org(name="Nestle\u0301", domain="nestle.com", description="Food and beverages"),
        "NFC and NFD forms in a structured record.",
        expected_kind="exact",
    ),
    Case(
        "unicode-003",
        "strings",
        DUPLICATE,
        "Société Générale",
        "Societe Generale",
        "Accent, transliteration, width, ligature, case or script variant.",
        lang="fr",
    ),
    Case(
        "unicode-004",
        "strings",
        DUPLICATE,
        "Müller GmbH",
        "Mueller GmbH",
        "Accent, transliteration, width, ligature, case or script variant.",
        lang="de",
    ),
    Case(
        "unicode-005",
        "strings",
        DUPLICATE,
        "Zürich Insurance",
        "Zurich Insurance",
        "Accent, transliteration, width, ligature, case or script variant.",
    ),
    Case(
        "unicode-006",
        "strings",
        DUPLICATE,
        "Ålesund Fisk AS",
        "Alesund Fisk AS",
        "Accent, transliteration, width, ligature, case or script variant.",
        lang="no",
    ),
    Case(
        "unicode-007",
        "strings",
        DUPLICATE,
        "Straße der Nationen",
        "Strasse der Nationen",
        "Equal after full case folding, which maps ß to ss.",
        lang="de",
        expected_kind="exact",
    ),
    Case(
        "unicode-008",
        "strings",
        DUPLICATE,
        "\u0130stanbul Havaliman\u0131",
        "Istanbul Havalimani",
        "Accent, transliteration, width, ligature, case or script variant.",
        lang="tr",
    ),
    Case(
        "unicode-009",
        "strings",
        DUPLICATE,
        "Œuvre d'Orient",
        "Oeuvre d'Orient",
        "Accent, transliteration, width, ligature, case or script variant.",
        lang="fr",
    ),
    Case(
        "unicode-010",
        "strings",
        DUPLICATE,
        "\uff21\uff22\uff23 Corp",
        "ABC Corp",
        "Accent, transliteration, width, ligature, case or script variant.",
    ),
    Case(
        "unicode-011",
        "strings",
        DUPLICATE,
        "\ufb01nance team",
        "finance team",
        "Equal after full case folding, which expands the fi ligature.",
        expected_kind="exact",
    ),
    Case(
        "unicode-012",
        "strings",
        DUPLICATE,
        "IBM",
        "ibm",
        "Equal after case folding.",
        expected_kind="exact",
    ),
    Case(
        "unicode-013",
        "strings",
        DUPLICATE,
        "Яндекс",
        "Yandex",
        "Accent, transliteration, width, ligature, case or script variant.",
        lang="ru",
    ),
    Case(
        "unicode-014",
        "strings",
        DUPLICATE,
        "東京電力",
        "Tokyo Electric Power",
        "Accent, transliteration, width, ligature, case or script variant.",
        lang="ja",
    ),
    Case(
        "unicode-015",
        "organizations",
        DISTINCT,
        org(name="PayPal", domain="paypal.com", description="Online payments"),
        org(name="PayPal", domain="p\u0430ypal.com", description="Online payments"),
        "Cyrillic homoglyph in the domain: a different, possibly malicious, host.",
    ),
    Case(
        "unicode-016",
        "strings",
        DISTINCT,
        "中国银行",
        "中国建设银行",
        "Bank of China versus China Construction Bank share characters.",
        lang="zh",
    ),
    Case(
        "unicode-017",
        "organizations",
        DUPLICATE,
        org(name="STRIPE", domain="Stripe.COM", description="Online Payments"),
        org(name="Stripe", domain="stripe.com", description="online payments"),
        "Equal after case folding of text and hostname normalization.",
        expected_kind="exact",
    ),
    Case(
        "unicode-018",
        "organizations",
        DUPLICATE,
        org(name="Stripe", domain="stripe.com."),
        org(name="Stripe", domain="stripe.com"),
        "A trailing dot does not change a hostname.",
        expected_kind="exact",
    ),
    Case(
        "unicode-019",
        "organizations",
        DISTINCT,
        org(name="Weinhaus", domain="fa\u00df.de", description="Weinhandlung"),
        org(name="Weinhaus", domain="fass.de", description="Weinhandlung"),
        "Distinct IDNA2008 domains (xn--fa-hia.de and fass.de): a lookalike host, never exact.",
        lang="de",
    ),
    Case(
        "unicode-020",
        "organizations",
        DUPLICATE,
        org(name="B\u00fccher Online", domain="B\u00dcCHER.de", description="Buchhandel"),
        org(name="B\u00fccher Online", domain="b\u00fccher.de", description="Buchhandel"),
        "Same organization; only ASCII case is folded in hostnames, so not exact.",
        lang="de",
    ),
    # ordering
    Case(
        "ordering-001",
        "configurations",
        DUPLICATE,
        config(
            service="worker", params={"retries": 3, "timeout_ms": 500}, description="Job worker"
        ),
        config(
            service="worker", params={"timeout_ms": 500, "retries": 3}, description="Job worker"
        ),
        "Reordered object keys are the same object.",
        expected_kind="exact",
    ),
    Case(
        "ordering-002",
        "organizations",
        DUPLICATE,
        {"id": "stripe-1", "tenant_id": "tenant-a", "name": "Stripe", "domain": "stripe.com"},
        {"domain": "stripe.com", "name": "Stripe", "tenant_id": "tenant-a", "id": "stripe-2"},
        "Reordered record fields are the same record.",
        expected_kind="exact",
    ),
    Case(
        "ordering-003",
        "configurations",
        DISTINCT,
        config(service="cdn", params={"regions": ["eu", "us"]}, description="Edge regions"),
        config(service="cdn", params={"regions": ["us", "eu"]}, description="Edge regions"),
        "Lists keep their order by default.",
        constraint="veto",
    ),
    Case(
        "ordering-004",
        "configurations",
        DUPLICATE,
        config(service="cdn", params={"regions": ["eu", "us"]}, description="Edge regions"),
        config(service="cdn", params={"regions": ["eu", "us"]}, description="Edge regions"),
        "Same list in the same order.",
        expected_kind="exact",
    ),
    Case(
        "ordering-005",
        "strings",
        DUPLICATE,
        "Smith, John",
        "John Smith",
        "Name written family name first.",
    ),
    Case(
        "ordering-006",
        "strings",
        DUPLICATE,
        "Black USB-C cable 1 m",
        "USB-C cable 1 m, black",
        "Reordered tokens.",
    ),
    Case(
        "ordering-007",
        "strings",
        DISTINCT,
        "Dog bites man",
        "Man bites dog",
        "Word order changes the meaning.",
    ),
    Case(
        "ordering-008",
        "strings",
        DISTINCT,
        "Flight Paris to Lyon",
        "Flight Lyon to Paris",
        "Opposite direction.",
    ),
    Case(
        "ordering-009",
        "strings",
        DISTINCT,
        "Vol Paris → Lyon",
        "Vol Lyon → Paris",
        "Opposite direction.",
        lang="fr",
    ),
    Case(
        "ordering-010",
        "strings",
        DISTINCT,
        "Pomme de terre",
        "Terre de pomme",
        "Word order changes the meaning.",
        lang="fr",
    ),
    Case(
        "ordering-011",
        "strings",
        DUPLICATE,
        "Dupont, Marie",
        "Marie Dupont",
        "Name written family name first.",
        lang="fr",
    ),
    # tenant
    Case(
        "tenant-001",
        "organizations",
        DISTINCT,
        org(name="Amazon", domain="amazon.com", description="Online retail"),
        org(tenant="tenant-b", name="Amazon", domain="amazon.com", description="Online retail"),
        "Identical records in different tenants are never compared.",
        constraint="partition",
    ),
    Case(
        "tenant-002",
        "organizations",
        DISTINCT,
        org(name="Stripe", domain="stripe.com", description="Online payments"),
        org(tenant="tenant-b", name="Stripe", domain="stripe.com", description="Online payments"),
        "Identical records in different tenants are never compared.",
        constraint="partition",
    ),
    Case(
        "tenant-003",
        "organizations",
        DISTINCT,
        org(name="Carrefour", domain="carrefour.fr", description="Hypermarchés"),
        org(tenant="tenant-b", name="Carrefour", domain="carrefour.fr", description="Hypermarchés"),
        "Identical records in different tenants are never compared.",
        constraint="partition",
    ),
    Case(
        "tenant-004",
        "organizations",
        DISTINCT,
        org(name="Acme", domain="acme.example", description="Industrial supplies"),
        org(
            tenant="tenant-b", name="Acme", domain="acme.example", description="Industrial supplies"
        ),
        "Identical records in different tenants are never compared.",
        constraint="partition",
    ),
    Case(
        "tenant-005",
        "organizations",
        DISTINCT,
        org(name="Globex", domain="globex.example", description="Energy trading"),
        org(
            tenant="tenant-b", name="Globex", domain="globex.example", description="Energy trading"
        ),
        "Identical records in different tenants are never compared.",
        constraint="partition",
    ),
    Case(
        "tenant-006",
        "organizations",
        DISTINCT,
        org(name="Initech", domain="initech.example", description="Software consulting"),
        org(
            tenant="tenant-b",
            name="Initech",
            domain="initech.example",
            description="Software consulting",
        ),
        "Identical records in different tenants are never compared.",
        constraint="partition",
    ),
    Case(
        "tenant-007",
        "organizations",
        DISTINCT,
        org(name="Umbrella", domain="umbrella.example", description="Pharmaceuticals"),
        org(
            tenant="tenant-b",
            name="Umbrella",
            domain="umbrella.example",
            description="Pharmaceuticals",
        ),
        "Identical records in different tenants are never compared.",
        constraint="partition",
    ),
    Case(
        "tenant-008",
        "organizations",
        DISTINCT,
        org(name="Hooli", domain="hooli.example", description="Search and cloud"),
        org(
            tenant="tenant-b", name="Hooli", domain="hooli.example", description="Search and cloud"
        ),
        "Identical records in different tenants are never compared.",
        constraint="partition",
    ),
    Case(
        "tenant-009",
        "organizations",
        DISTINCT,
        org(name="Amazon", domain="amazon.com", description="Online retail"),
        org(tenant="tenant-b", name="Amiazon", domain="amazon.com", description="Online retailer"),
        "Near duplicates in different tenants are never compared.",
        constraint="partition",
    ),
    Case(
        "tenant-010",
        "organizations",
        DISTINCT,
        org(name="Acme", domain="acme.example"),
        org(tenant="Tenant-A", name="Acme", domain="acme.example"),
        "Partition keys are compared exactly; case differs.",
        constraint="partition",
    ),
    Case(
        "tenant-011",
        "organizations",
        DISTINCT,
        org(tenant="1", name="Acme", domain="acme.example"),
        org(tenant=1, name="Acme", domain="acme.example"),
        "Partition keys are typed; string versus integer.",
        constraint="partition",
    ),
    Case(
        "tenant-012",
        "organizations",
        DUPLICATE,
        org(tenant="tenant-b", name="Amazon", domain="amazon.com", description="Online retail"),
        org(tenant="tenant-b", name="Amiazon", domain="amazon.com", description="Online retailer"),
        "Control: same tenant, near duplicate.",
    ),
    Case(
        "tenant-013",
        "organizations",
        DUPLICATE,
        org(
            tenant="tenant-b", name="Acme", domain="acme.example", description="Industrial supplies"
        ),
        org(
            tenant="tenant-b", name="Acme", domain="acme.example", description="Industrial supplies"
        ),
        "Control: same tenant, identical records.",
        expected_kind="exact",
    ),
    Case(
        "tenant-014",
        "organizations",
        INVALID,
        org(name="Acme", domain="acme.example"),
        org(name="Acme", domain="acme.example", tenant=ABSENT),
        "A record without its partition key is rejected.",
        expected_error="InvalidRecordError",
    ),
    Case(
        "tenant-015",
        "organizations",
        DISTINCT,
        org(
            tenant="client-1",
            name="Boulangerie Dupont",
            domain="dupont.example",
            description="Boulangerie artisanale",
        ),
        org(
            tenant="client-2",
            name="Boulangerie Dupont",
            domain="dupont.example",
            description="Boulangerie artisanale",
        ),
        "Identical records in different tenants are never compared.",
        lang="fr",
        constraint="partition",
    ),
    # dense_series
    Case(
        "dense_series-001",
        "products",
        DISTINCT,
        product(name="Acme Store #101"),
        product(name="Acme Store #102"),
        "Members of a dense series differ by one attribute.",
    ),
    Case(
        "dense_series-002",
        "products",
        DISTINCT,
        product(name="Meeting room 204"),
        product(name="Meeting room 205"),
        "Members of a dense series differ by one attribute.",
    ),
    Case(
        "dense_series-003",
        "products",
        DISTINCT,
        product(name="Invoice 2026-0001"),
        product(name="Invoice 2026-0002"),
        "Members of a dense series differ by one attribute.",
    ),
    Case(
        "dense_series-004",
        "products",
        DISTINCT,
        product(name="T-shirt, size M"),
        product(name="T-shirt, size L"),
        "Members of a dense series differ by one attribute.",
    ),
    Case(
        "dense_series-005",
        "products",
        DISTINCT,
        product(name="Sneakers, red"),
        product(name="Sneakers, blue"),
        "Members of a dense series differ by one attribute.",
    ),
    Case(
        "dense_series-006",
        "products",
        DISTINCT,
        product(name="Paris 1er arrondissement"),
        product(name="Paris 2e arrondissement"),
        "Members of a dense series differ by one attribute.",
        lang="fr",
    ),
    Case(
        "dense_series-007",
        "products",
        DISTINCT,
        product(name="Lot 17"),
        product(name="Lot 71"),
        "Members of a dense series differ by one attribute.",
        lang="fr",
    ),
    Case(
        "dense_series-008",
        "products",
        DISTINCT,
        product(name="Bus line 21"),
        product(name="Bus line 12"),
        "Members of a dense series differ by one attribute.",
    ),
    Case(
        "dense_series-009",
        "products",
        DISTINCT,
        product(name="Acme Store #101", sku="STORE-101"),
        product(name="Acme Store #101", sku="STORE-110"),
        "Equal names; different SKUs veto the pair.",
        constraint="veto",
    ),
    Case(
        "dense_series-010",
        "products",
        DUPLICATE,
        product(name="Acme Store #101", sku="STORE-101"),
        product(name="Acme store #101", sku="STORE-101"),
        "Control: same member of the series; names equal after case folding.",
        expected_kind="exact",
    ),
    Case(
        "dense_series-011",
        "products",
        DISTINCT,
        product(name="Appartement T2"),
        product(name="Appartement T3"),
        "Members of a dense series differ by one attribute.",
        lang="fr",
    ),
    Case(
        "dense_series-012",
        "products",
        DISTINCT,
        product(name="Chambre 12"),
        product(name="Chambre 21"),
        "Members of a dense series differ by one attribute.",
        lang="fr",
    ),
    # short_string
    Case("short_string-001", "strings", DUPLICATE, "IBM", "I.B.M.", "Dotted acronym."),
    Case("short_string-002", "strings", DUPLICATE, "3M", "3 M", "Inserted space."),
    Case("short_string-003", "strings", DUPLICATE, "BP", "B.P.", "Dotted acronym."),
    Case(
        "short_string-004", "strings", DUPLICATE, "SNCF", "S.N.C.F.", "Dotted acronym.", lang="fr"
    ),
    Case("short_string-005", "strings", DUPLICATE, "AT&T", "ATT", "Dropped ampersand."),
    Case("short_string-006", "strings", DUPLICATE, "BMW", "BWM", "Transposed letters."),
    Case(
        "short_string-007",
        "strings",
        DUPLICATE,
        "Q",
        "Q",
        "Identical single character.",
        expected_kind="exact",
    ),
    Case(
        "short_string-008", "strings", DUPLICATE, "RATP", "R.A.T.P.", "Dotted acronym.", lang="fr"
    ),
    Case(
        "short_string-009",
        "strings",
        DISTINCT,
        "BNP",
        "BPI",
        "Different banks with similar acronyms.",
        lang="fr",
    ),
    Case(
        "short_string-010",
        "strings",
        DISTINCT,
        "HP",
        "HPE",
        "Different companies one letter apart.",
    ),
    Case("short_string-011", "strings", DISTINCT, "AA", "AB", "Different two-letter codes."),
    Case(
        "short_string-012", "strings", DISTINCT, "GE", "GM", "Different companies one letter apart."
    ),
    Case("short_string-013", "strings", DISTINCT, "X", "Y", "Different single characters."),
    Case(
        "short_string-014",
        "strings",
        DISTINCT,
        "EDF",
        "GDF",
        "Different companies one letter apart.",
        lang="fr",
    ),
    Case("short_string-015", "strings", DISTINCT, "UPS", "USPS", "Different carriers."),
    Case("short_string-016", "strings", DISTINCT, "R2", "R2-D2", "Code versus longer name."),
    Case("short_string-017", "strings", DISTINCT, "EU", "US", "Different region codes."),
    # adversarial
    Case("adversarial-001", "strings", DISTINCT, "", "", "Empty strings carry no evidence."),
    Case("adversarial-002", "strings", DISTINCT, "   ", "\t", "Blank strings carry no evidence."),
    Case("adversarial-003", "strings", DISTINCT, "🍕", "🍔", "Different emoji-only strings."),
    Case(
        "adversarial-004",
        "strings",
        DUPLICATE,
        "Ama\u200bzon",
        "Amazon",
        "Zero-width space inside a name.",
    ),
    Case(
        "adversarial-005",
        "organizations",
        INVALID,
        org(name="Acme", domain="acme.example"),
        ["Acme", "acme.example"],
        "A list is not an organization record.",
        expected_error="InvalidRecordError",
    ),
    Case(
        "adversarial-006",
        "organizations",
        INVALID,
        org(name=42, domain="acme.example"),
        org(name="42", domain="acme.example"),
        "Text fields reject numbers instead of coercing them.",
        expected_error="InvalidRecordError",
    ),
    Case(
        "adversarial-007",
        "organizations",
        INVALID,
        org(name=True, domain="acme.example"),
        org(name="True", domain="acme.example"),
        "Text fields reject booleans instead of coercing them.",
        expected_error="InvalidRecordError",
    ),
    Case(
        "adversarial-008",
        "organizations",
        INVALID,
        {"tenant_id": "tenant-a", "name": "Acme"},
        {"id": "acme-2", "tenant_id": "tenant-a", "name": "Acme"},
        "A record without an identifier is rejected when id_field is set.",
        expected_error="InvalidRecordError",
        assign_ids=False,
    ),
    Case(
        "adversarial-009",
        "organizations",
        INVALID,
        {"id": "same", "tenant_id": "tenant-a", "name": "Acme", "domain": "acme.example"},
        {"id": "same", "tenant_id": "tenant-a", "name": "Acme Corp", "domain": "acme.example"},
        "Two records with the same identifier in one call.",
        expected_error="DuplicateIdError",
    ),
    Case(
        "adversarial-010",
        "configurations",
        INVALID,
        config(service="worker", params=nested(17), description="Job worker"),
        config(service="worker", params={"level": "shallow"}, description="Job worker"),
        "Nesting depth 17 exceeds the default limit of 16.",
        expected_error="ResourceLimitError",
    ),
]


def _with_id(record: Any, identifier: str) -> Any:
    if not isinstance(record, dict) or "id" in record:
        return record
    return {"id": identifier, **record}


def to_json(case: Case) -> dict[str, Any]:
    left, right = case.left, case.right
    if case.profile != "strings" and case.assign_ids:
        left = _with_id(left, f"{case.case_id}-l")
        right = _with_id(right, f"{case.case_id}-r")
    return {
        "case_id": case.case_id,
        "profile": case.profile,
        "family": case.family,
        "lang": case.lang,
        "label": case.label,
        "constraint": case.constraint,
        "expected_kind": case.expected_kind,
        "expected_error": case.expected_error,
        "left": left,
        "right": right,
        "rationale": case.rationale,
    }


def validate(cases: list[Case]) -> None:
    identifiers = Counter(case.case_id for case in cases)
    duplicates = sorted(identifier for identifier, count in identifiers.items() if count > 1)
    if duplicates:
        raise ValueError(f"duplicate case ids: {duplicates}")
    unknown = sorted({case.family for case in cases} - set(FAMILIES))
    if unknown:
        raise ValueError(f"unknown families: {unknown}")


def render(cases: list[Case]) -> str:
    return "".join(json.dumps(to_json(case), ensure_ascii=False) + "\n" for case in cases)


def coverage(cases: list[Case]) -> str:
    families = Counter(case.family for case in cases)
    labels = Counter(case.label for case in cases)
    constraints = Counter(case.constraint for case in cases if case.constraint)
    languages = Counter(case.lang for case in cases)
    rows = "\n".join(f"| {family} | {families[family]} |" for family in FAMILIES)
    return (
        f"{COVERAGE_START}\n"
        f"{len(cases)} cases.\n\n"
        "| Family | Cases |\n| --- | --- |\n"
        f"{rows}\n\n"
        "| Label | Cases |\n| --- | --- |\n"
        + "".join(f"| {label} | {labels[label]} |\n" for label in sorted(labels))
        + "\nConstraints: "
        + ", ".join(f"{count} {name}" for name, count in sorted(constraints.items()))
        + ". Languages: "
        + ", ".join(f"{count} {name}" for name, count in languages.most_common())
        + f".\n{COVERAGE_END}"
    )


def with_coverage(readme: str, cases: list[Case]) -> str:
    start, end = readme.index(COVERAGE_START), readme.index(COVERAGE_END) + len(COVERAGE_END)
    return readme[:start] + coverage(cases) + readme[end:]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if outputs are stale")
    arguments = parser.parse_args()

    validate(CASES)
    outputs = {
        OUTPUT: render(CASES),
        DOC: with_coverage(DOC.read_text(encoding="utf-8"), CASES),
    }
    if arguments.check:
        stale = [path for path, content in outputs.items() if path.read_text("utf-8") != content]
        for path in stale:
            print(f"{path} is stale; run tools/fixtures/build_critical_corpus.py", file=sys.stderr)
        return 1 if stale else 0
    for path, content in outputs.items():
        path.write_text(content, encoding="utf-8")
    print(f"wrote {len(CASES)} cases to {OUTPUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

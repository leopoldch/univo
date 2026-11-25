"""Splink on the record datasets, with the settings of its own examples.

Comparisons, blocking rules and training steps are those of the example
notebooks of Splink for `febrl3`, `febrl4` (its detailed model) and
`historical_50k` (`docs/demos/examples/duckdb/` at commit f319070e), on the
DuckDB backend. Splink learns its parameters from the records it scores,
without labels: each half is trained and scored on its own.

Writes every compared pair with a match weight of at least `--floor`, and
the time from the records in memory to the pairs in memory.

    python benchmarks/records/record_pairs_splink.py --dataset febrl3 --cache /tmp/records-data \
        --output /tmp/records-pairs/febrl3-splink.json
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from importlib import metadata
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from record_datasets import HALVES, Record, load


def febrl3(tables: list[Any], api: Any) -> Any:
    import splink.comparison_library as cl
    from splink import Linker, SettingsCreator, block_on

    settings = SettingsCreator(
        unique_id_column_name="id",
        link_type="dedupe_only",
        blocking_rules_to_generate_predictions=[
            block_on("soc_sec_id"),
            block_on("given_name"),
            block_on("surname"),
            block_on("date_of_birth"),
            block_on("postcode"),
        ],
        comparisons=[
            cl.NameComparison("given_name"),
            cl.NameComparison("surname"),
            cl.DateOfBirthComparison(
                "date_of_birth", input_is_string=True, datetime_format="%Y%m%d"
            ),
            cl.DamerauLevenshteinAtThresholds("soc_sec_id", [2]),
            cl.ExactMatch("street_number").configure(term_frequency_adjustments=True),
            cl.ExactMatch("postcode").configure(term_frequency_adjustments=True),
        ],
        retain_intermediate_calculation_columns=True,
    )
    linker = Linker(tables[0], settings)
    linker.training.estimate_probability_two_random_records_match(
        [
            block_on("soc_sec_id"),
            block_on("given_name", "surname", "date_of_birth"),
            "l.given_name = r.surname and l.surname = r.given_name "
            "and l.date_of_birth = r.date_of_birth",
        ],
        recall=0.9,
    )
    linker.training.estimate_u_using_random_sampling(max_pairs=1e6, seed=2)
    for rule in ("date_of_birth", "postcode"):
        linker.training.estimate_parameters_using_expectation_maximisation(block_on(rule))
    return linker


def febrl4(tables: list[Any], api: Any) -> Any:
    import splink.comparison_library as cl
    from splink import Linker, SettingsCreator, block_on

    settings = SettingsCreator(
        unique_id_column_name="id",
        link_type="link_only",
        blocking_rules_to_generate_predictions=[
            block_on("given_name", "surname"),
            "l.given_name = r.surname and l.surname = r.given_name",
            block_on("date_of_birth"),
            block_on("soc_sec_id"),
            block_on("state", "address_1"),
            block_on("street_number", "address_1"),
            block_on("postcode"),
        ],
        comparisons=[
            cl.NameComparison("given_name").configure(term_frequency_adjustments=True),
            cl.NameComparison("surname").configure(term_frequency_adjustments=True),
            cl.DateOfBirthComparison(
                "date_of_birth",
                input_is_string=True,
                datetime_format="%Y%m%d",
                invalid_dates_as_null=True,
            ),
            cl.DamerauLevenshteinAtThresholds("soc_sec_id", [1, 2]),
            cl.ExactMatch("street_number").configure(term_frequency_adjustments=True),
            cl.DamerauLevenshteinAtThresholds("postcode", [1, 2]).configure(
                term_frequency_adjustments=True
            ),
        ],
        retain_intermediate_calculation_columns=True,
    )
    linker = Linker(tables, settings)
    linker.training.estimate_probability_two_random_records_match(
        [block_on("soc_sec_id"), block_on("given_name", "surname", "date_of_birth")],
        recall=0.8,
    )
    linker.training.estimate_u_using_random_sampling(max_pairs=1e6)
    for rule in ("date_of_birth", "postcode"):
        linker.training.estimate_parameters_using_expectation_maximisation(
            block_on(rule), estimate_without_term_frequencies=True
        )
    return linker


def historical_50k(tables: list[Any], api: Any) -> Any:
    import splink.comparison_library as cl
    from splink import Linker, SettingsCreator, block_on

    settings = SettingsCreator(
        unique_id_column_name="id",
        link_type="dedupe_only",
        blocking_rules_to_generate_predictions=[
            block_on("substr(first_name,1,3)", "substr(surname,1,4)"),
            block_on("surname", "dob"),
            block_on("first_name", "dob"),
            block_on("postcode_fake", "first_name"),
            block_on("postcode_fake", "surname"),
            block_on("dob", "birth_place"),
            block_on("substr(postcode_fake,1,3)", "dob"),
            block_on("substr(postcode_fake,1,3)", "first_name"),
            block_on("substr(postcode_fake,1,3)", "surname"),
            block_on("substr(first_name,1,2)", "substr(surname,1,2)", "substr(dob,1,4)"),
        ],
        comparisons=[
            cl.ForenameSurnameComparison(
                "first_name",
                "surname",
                forename_surname_concat_col_name="first_name_surname_concat",
            ),
            cl.DateOfBirthComparison("dob", input_is_string=True),
            cl.PostcodeComparison("postcode_fake"),
            cl.ExactMatch("birth_place").configure(term_frequency_adjustments=True),
            cl.ExactMatch("occupation").configure(term_frequency_adjustments=True),
        ],
        retain_intermediate_calculation_columns=True,
    )
    relation = api.duckdb_con.from_arrow(tables[0].as_duckdbpyrelation().arrow().read_all())
    table = api.register(
        relation.project("*, first_name || ' ' || surname as first_name_surname_concat")
    )
    linker = Linker(table, settings)
    linker.training.estimate_probability_two_random_records_match(
        [
            block_on("first_name", "surname", "dob"),
            block_on("substr(first_name,1,2)", "surname", "substr(postcode_fake,1,2)"),
            block_on("dob", "postcode_fake"),
        ],
        recall=0.6,
    )
    linker.training.estimate_u_using_random_sampling(max_pairs=1e7)
    for rule in (("first_name", "surname"), ("dob",)):
        linker.training.estimate_parameters_using_expectation_maximisation(
            block_on(*rule), estimate_without_term_frequencies=True
        )
    return linker


MODELS = {"febrl3": febrl3, "febrl4": febrl4, "historical_50k": historical_50k}


def scored(
    model: Any, fields: tuple[str, ...], records: list[Record], floor: float
) -> list[list[object]]:
    """Pairs `[left, right, match weight]` of one set of records."""
    import pyarrow as pa
    from splink import DuckDBAPI

    api = DuckDBAPI()
    sides = sorted({str(record["source"]) for record in records})
    tables = [
        api.register(
            pa.table(
                {
                    column: [record[column] for record in records if record["source"] == side]
                    for column in ("id", *fields)
                },
                schema=pa.schema([(column, pa.string()) for column in ("id", *fields)]),
            )
        )
        for side in sides
    ]
    linker = model(tables, api)
    predictions = linker.inference.predict()
    rows = (
        predictions.as_duckdbpyrelation()
        .filter(f"match_weight >= {floor}")
        .project("id_l, id_r, match_weight")
        .fetchall()
    )
    return [[left, right, round(weight, 4)] for left, right, weight in rows]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dataset", choices=sorted(MODELS), required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--floor", type=float, default=-10.0, help="lowest match weight kept")
    arguments = parser.parse_args()

    dataset = load(arguments.dataset, arguments.cache)
    runs = []
    for half in HALVES:
        records = dataset.half(half)
        started = time.perf_counter()
        pairs = scored(MODELS[dataset.name], dataset.fields, records, arguments.floor)
        seconds = time.perf_counter() - started
        runs.append({"variant": "example", "half": half, "seconds": seconds, "pairs": pairs})
        print(f"{dataset.name} {half}: {len(pairs)} pairs in {seconds:.1f} s")
    result = {
        "tool": "splink",
        "version": metadata.version("splink"),
        "dataset": dataset.name,
        "runs": runs,
    }
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(result) + "\n")


if __name__ == "__main__":
    main()

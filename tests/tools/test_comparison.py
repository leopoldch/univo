"""The comparison benchmark's decision rule, which must match SemHash's."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "benchmarks" / "comparison"))

from dedupe import greedy, metrics


def test_exact_copies_share_the_representative_of_their_first_occurrence() -> None:
    # [X, A, Y, A]: SemHash groups both A first, so the second A follows the
    # first to X, even though Y, kept in between, is closer to it.
    texts = ["X", "A", "Y", "A"]
    pairs = [(0, 1, 0.9), (0, 3, 0.9), (1, 2, 0.95), (2, 3, 0.95), (0, 2, 0.1), (1, 3, 1.0)]
    assert sorted(greedy(texts, pairs, 0.8)) == [(1, 0), (3, 0)]


def test_a_selected_group_keeps_its_first_copy() -> None:
    texts = ["A", "B", "A", "A"]
    pairs = [(0, 2, 1.0), (0, 3, 1.0), (2, 3, 1.0), (0, 1, 0.1)]
    assert sorted(greedy(texts, pairs, 0.8)) == [(2, 0), (3, 0)]


def test_ties_choose_the_earliest_kept_group() -> None:
    texts = ["X", "Y", "Z"]
    pairs = [(0, 2, 0.9), (1, 2, 0.9), (0, 1, 0.1)]
    assert greedy(texts, pairs, 0.8) == [(2, 0)]


def test_metrics_count_correct_removals_and_possible_ones() -> None:
    records = [(0, "e1", "X"), (1, "e1", "X2"), (2, "e2", "Y"), (3, "e1", "X3")]
    result = metrics(records, [(1, 0), (2, 0)])
    assert (result["removed"], result["correct"], result["possible"]) == (2, 1, 2)

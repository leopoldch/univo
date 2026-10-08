//! Lexical primitives against naive reference implementations.

// Symmetry and reflexivity must hold exactly, not within a tolerance.
#![allow(clippy::float_cmp)]

use std::collections::BTreeSet;

use proptest::prelude::*;
use univo_core::{MAX_EDIT_CHARS, edit_similarity, jaccard, trigrams};

/// Textbook Levenshtein distance over characters, with a full matrix.
fn reference_levenshtein(a: &[char], b: &[char]) -> usize {
    let mut matrix = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in matrix[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let substitution = matrix[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]);
            matrix[i][j] = substitution
                .min(matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1);
        }
    }
    matrix[a.len()][b.len()]
}

/// Padded trigrams as strings, in a set.
fn reference_trigrams(text: &str) -> BTreeSet<String> {
    let padded: Vec<char> = format!("  {text}  ").chars().collect();
    padded
        .windows(3)
        .map(|window| window.iter().collect())
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn reference_jaccard(a: &str, b: &str) -> f64 {
    let (a, b) = (reference_trigrams(a), reference_trigrams(b));
    let union = a.union(&b).count();
    if union == 0 {
        0.0
    } else {
        a.intersection(&b).count() as f64 / union as f64
    }
}

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            prop::char::range('a', 'f'),
            Just(' '),
            Just('é'),
            Just('ß'),
            any::<char>(),
        ],
        0..24,
    )
    .prop_map(|characters| characters.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn edit_similarity_matches_the_reference(a in text(), b in text()) {
        let (left, right): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        let longest = left.len().max(right.len());
        let expected = if longest == 0 {
            1.0
        } else {
            1.0 - reference_levenshtein(&left, &right) as f64 / longest as f64
        };
        let actual = f64::from(edit_similarity(&a, &b).unwrap());
        prop_assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    #[test]
    fn jaccard_of_trigrams_matches_the_reference(a in text(), b in text()) {
        let actual = f64::from(jaccard(&trigrams(&a), &trigrams(&b)));
        prop_assert!((actual - reference_jaccard(&a, &b)).abs() < 1e-6);
    }

    #[test]
    fn similarities_are_symmetric_bounded_and_reflexive(a in text(), b in text()) {
        let edit = edit_similarity(&a, &b).unwrap();
        prop_assert_eq!(edit, edit_similarity(&b, &a).unwrap());
        prop_assert!((0.0..=1.0).contains(&edit));
        prop_assert_eq!(edit_similarity(&a, &a), Some(1.0));
        let overlap = jaccard(&trigrams(&a), &trigrams(&b));
        prop_assert_eq!(overlap, jaccard(&trigrams(&b), &trigrams(&a)));
        prop_assert!((0.0..=1.0).contains(&overlap));
        prop_assert_eq!(jaccard(&trigrams(&a), &trigrams(&a)), 1.0);
    }
}

#[test]
fn edit_distance_stops_at_the_length_bound() {
    let at_bound = "x".repeat(MAX_EDIT_CHARS);
    let above = "x".repeat(MAX_EDIT_CHARS + 1);
    assert_eq!(edit_similarity(&at_bound, &at_bound), Some(1.0));
    assert_eq!(edit_similarity(&above, "x"), None);
    assert_eq!(edit_similarity("x", &above), None);
}

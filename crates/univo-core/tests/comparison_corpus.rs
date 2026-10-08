//! Pairwise comparison against the critical corpus: hard invariants on every
//! case, and the ranking quality of the score as a regression guard. The
//! corpus is a regression set; nothing here tunes a weight or a threshold.

#[path = "../../../tests/rust/corpus.rs"]
#[allow(dead_code)]
mod corpus;

use serde_json::Value as Json;
use univo_core::{Comparison, Schema, compare};

use corpus::{cases, prepare_pair, schema};

/// The score's AUC on the corpus was 0.763 when this guard was set (U-008);
/// the floor leaves 0.02 for deliberate changes, and a lower value is a
/// regression to explain. Semantic fields do not contribute yet (U-009).
const SCORE_AUC_FLOOR: f64 = 0.74;

fn comparison(case: &Json, schema: &Schema) -> Option<Comparison> {
    let (left, right) = prepare_pair(case, schema);
    compare(schema, &left.unwrap(), &right.unwrap()).unwrap()
}

fn valid_cases() -> Vec<(Json, Schema)> {
    cases()
        .into_iter()
        .filter(|case| case["label"] != "invalid")
        .map(|case| {
            let schema = schema(case["profile"].as_str().unwrap());
            (case, schema)
        })
        .collect()
}

#[test]
fn hard_invariants_hold_on_every_case() {
    let mut failures = Vec::new();
    for (case, schema) in valid_cases() {
        let id = case["case_id"].as_str().unwrap();
        let result = comparison(&case, &schema);
        if case["constraint"] == "partition" {
            if result.is_some() {
                failures.push(format!(
                    "{id}: records of different partitions were compared"
                ));
            }
            continue;
        }
        let Some(result) = result else {
            failures.push(format!("{id}: records of one partition were not compared"));
            continue;
        };
        let expected_veto = case["constraint"] == "veto";
        if result.vetoed != expected_veto {
            failures.push(format!(
                "{id}: vetoed is {}, expected {expected_veto}",
                result.vetoed
            ));
        }
        if case["label"] == "compatible"
            && (result.vetoed || !result.fields.iter().any(|field| field.incomplete))
        {
            failures.push(format!(
                "{id}: compatible pair must be incomplete and not vetoed"
            ));
        }
        if case["expected_kind"] == "exact" && result.score != Some(1.0) {
            failures.push(format!("{id}: exact pair scored {:?}", result.score));
        }
        if !(0.0..=1.0).contains(&result.coverage)
            || result
                .score
                .is_some_and(|score| !(0.0..=1.0).contains(&score))
        {
            failures.push(format!(
                "{id}: score or coverage outside [0, 1]: {result:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn pairs_without_evidence_have_no_score() {
    for id in ["adversarial-001", "adversarial-002", "missing_value-005"] {
        let (case, schema) = valid_cases()
            .into_iter()
            .find(|(case, _)| case["case_id"] == id)
            .unwrap();
        let result = comparison(&case, &schema).unwrap();
        assert_eq!(result.score, None, "{id}");
    }
}

#[test]
fn comparison_is_symmetric_and_deterministic() {
    for (case, schema) in valid_cases() {
        let (left, right) = prepare_pair(&case, &schema);
        let (left, right) = (left.unwrap(), right.unwrap());
        let forward = compare(&schema, &left, &right).unwrap();
        let backward = compare(&schema, &right, &left).unwrap();
        assert_eq!(forward, compare(&schema, &left, &right).unwrap());
        assert_eq!(
            forward.as_ref().map(|c| (c.score, c.coverage, c.vetoed)),
            backward.as_ref().map(|c| (c.score, c.coverage, c.vetoed)),
            "{}",
            case["case_id"]
        );
    }
}

/// Mann-Whitney AUC with ties counted as one half.
#[allow(clippy::cast_precision_loss)]
fn auc(positives: &[f32], negatives: &[f32]) -> f64 {
    let mut wins = 0.0;
    for p in positives {
        for n in negatives {
            wins += match p.total_cmp(n) {
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => 0.5,
                std::cmp::Ordering::Less => 0.0,
            };
        }
    }
    wins / (positives.len() as f64 * negatives.len() as f64)
}

#[test]
fn score_ranks_duplicates_above_distinct_pairs() {
    let (mut positives, mut negatives) = (Vec::new(), Vec::new());
    for (case, schema) in valid_cases() {
        if !case["constraint"].is_null() {
            continue;
        }
        let Some(score) = comparison(&case, &schema).and_then(|result| result.score) else {
            continue;
        };
        match case["label"].as_str().unwrap() {
            "duplicate" => positives.push(score),
            "distinct" => negatives.push(score),
            _ => {}
        }
    }
    let value = auc(&positives, &negatives);
    eprintln!(
        "score AUC {value:.3} on {} duplicates and {} distinct pairs",
        positives.len(),
        negatives.len()
    );
    assert!(
        value >= SCORE_AUC_FLOOR,
        "score AUC {value:.3} below {SCORE_AUC_FLOOR}"
    );
}

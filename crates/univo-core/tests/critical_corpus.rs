//! Differential test against the critical corpus: the Rust normalizers and
//! the exact rule must reproduce every label of tests/fixtures/critical,
//! whose consistency with docs/duplicate-definitions.md is checked by an
//! independent Python oracle in tests/corpus.

#[path = "../../../tests/rust/corpus.rs"]
mod corpus;

use corpus::{BATCH_LEVEL_CASES, cases, error_name, path, prepare_pair, schema};
use univo_core::{Map, Schema, Text, Transforms, Value, exact_match, prepare};

#[test]
fn reproduces_every_label_of_the_critical_corpus() {
    let cases = cases();
    assert!(cases.len() >= 200);
    let mut failures = Vec::new();
    for case in &cases {
        let id = case["case_id"].as_str().unwrap();
        let schema = schema(case["profile"].as_str().unwrap());
        let (left, right) = prepare_pair(case, &schema);

        if case["label"] == "invalid" {
            if BATCH_LEVEL_CASES.contains(&id) {
                continue;
            }
            let expected = case["expected_error"].as_str().unwrap();
            let actual: Vec<_> = [&left, &right]
                .into_iter()
                .filter_map(|result| result.as_ref().err().map(error_name))
                .collect();
            if actual != [expected.to_owned()] {
                failures.push(format!("{id}: expected one {expected}, got {actual:?}"));
            }
            continue;
        }

        let (left, right) = match (left, right) {
            (Ok(left), Ok(right)) => (left, right),
            (left, right) => {
                failures.push(format!("{id}: valid records rejected: {left:?} {right:?}"));
                continue;
            }
        };
        let expected_exact = case["expected_kind"] == "exact";
        let exact = exact_match(&schema, &left, &right).unwrap();
        if exact.is_some() != expected_exact {
            failures.push(format!(
                "{id}: exact is {}, expected {expected_exact}",
                exact.is_some()
            ));
        }
        if case["constraint"] == "partition" && left.partition() == right.partition() {
            failures.push(format!("{id}: partitions should differ"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn exact_matches_report_what_normalization_ignored() {
    let cases = cases();
    let expectations = [
        ("unicode-001", Transforms::UNICODE_FORM),
        ("unicode-012", Transforms::CASE),
        ("unicode-018", Transforms::TRAILING_DOT),
        ("ordering-001", Transforms::KEY_ORDER),
        ("short_string-007", Transforms::NONE),
    ];
    for (id, expected) in expectations {
        let case = cases.iter().find(|case| case["case_id"] == id).unwrap();
        let schema = schema(case["profile"].as_str().unwrap());
        let (left, right) = prepare_pair(case, &schema);
        let exact = exact_match(&schema, &left.unwrap(), &right.unwrap())
            .unwrap()
            .unwrap();
        let ignored = exact
            .normalized_fields
            .iter()
            .fold(Transforms::NONE, |all, (_, transforms)| all | *transforms);
        assert_eq!(ignored, expected, "{id}");
    }
}

#[test]
fn identical_originals_report_no_normalization() {
    let schema = Schema::strings();
    let left = prepare(&schema, &Value::Str("IBM".into())).unwrap();
    let right = prepare(&schema, &Value::Str("IBM".into())).unwrap();
    let exact = exact_match(&schema, &left, &right).unwrap().unwrap();
    assert_eq!(exact.normalized_fields, []);
}

#[test]
fn case_folding_can_be_disabled_per_field() {
    let schema = Schema::builder()
        .text(
            path("unit"),
            Text {
                case_fold: false,
                ..Text::default()
            },
        )
        .build()
        .unwrap();
    let record =
        |unit: &str| Value::Map(Map::new(vec![("unit".into(), Value::Str(unit.into()))]).unwrap());
    let milliampere = prepare(&schema, &record("mA")).unwrap();
    let megaampere = prepare(&schema, &record("MA")).unwrap();
    assert!(
        exact_match(&schema, &milliampere, &megaampere)
            .unwrap()
            .is_none()
    );
}

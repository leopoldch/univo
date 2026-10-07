//! Differential test against the critical corpus: the Rust normalizers and
//! the exact rule must reproduce every label of tests/fixtures/critical,
//! whose consistency with docs/duplicate-definitions.md is checked by an
//! independent Python oracle in tests/corpus.

use std::path::Path;

use serde_json::Value as Json;
use univo_core::{
    Conflict, Error, Exact, ExactNormalizer, FieldPath, Map, PreparedRecord, Schema, Text,
    TextMode, Transforms, Value, exact_match, prepare,
};

/// Corpus cases whose expected error needs a batch, not a single record.
const BATCH_LEVEL_CASES: &[&str] = &["adversarial-009"];

fn path(dotted: &str) -> FieldPath {
    FieldPath::parse(dotted).unwrap()
}

fn semantic() -> Text {
    Text {
        mode: TextMode::Semantic,
        ..Text::default()
    }
}

fn discriminant() -> Exact {
    Exact {
        conflict: Conflict::Reject,
        ..Exact::default()
    }
}

fn schema(profile: &str) -> Schema {
    let builder = Schema::builder().id_field(path("id"));
    match profile {
        "strings" => return Schema::strings(),
        "organizations" => builder
            .partition_by(path("tenant_id"))
            .text(path("name"), Text::default())
            .exact(
                path("domain"),
                Exact {
                    normalizer: ExactNormalizer::Hostname,
                    ..Exact::default()
                },
            )
            .text(path("description"), semantic()),
        "products" => builder
            .text(path("name"), Text::default())
            .exact(path("sku"), discriminant())
            .text(path("description"), semantic()),
        "configurations" => builder
            .exact(path("service"), discriminant())
            .exact(path("environment"), discriminant())
            .exact(path("params"), discriminant())
            .text(path("description"), semantic()),
        other => panic!("unknown profile {other}"),
    }
    .build()
    .unwrap()
}

fn to_value(json: &Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(number) => number.as_i64().map_or_else(
            || Value::float(number.as_f64().unwrap()).unwrap(),
            Value::Int,
        ),
        Json::String(text) => Value::Str(text.clone()),
        Json::Array(items) => Value::List(items.iter().map(to_value).collect()),
        Json::Object(object) => Value::Map(
            Map::new(
                object
                    .iter()
                    .map(|(key, item)| (key.clone(), to_value(item)))
                    .collect(),
            )
            .unwrap(),
        ),
    }
}

fn error_name(error: &Error) -> &'static str {
    match error {
        Error::InvalidRecord { .. } => "InvalidRecordError",
        Error::ResourceLimit { .. } => "ResourceLimitError",
        Error::InvalidSchema { .. } => "InvalidSchemaError",
        _ => "other",
    }
}

fn cases() -> Vec<Json> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/critical/cases.jsonl");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn prepare_pair(
    case: &Json,
    schema: &Schema,
) -> (Result<PreparedRecord, Error>, Result<PreparedRecord, Error>) {
    (
        prepare(schema, &to_value(&case["left"])),
        prepare(schema, &to_value(&case["right"])),
    )
}

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
            if actual != [expected] {
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

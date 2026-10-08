//! Key-by-key comparison of objects: null is unknown at every depth, and
//! keys present on one side only lower the coverage.

// Coverage and similarity are exact ratios of small counts.
#![allow(clippy::float_cmp)]

use univo_core::{Conflict, Exact, FieldPath, Map, Schema, Value, compare, prepare};

fn object(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(
        Map::new(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        )
        .unwrap(),
    )
}

fn params_schema() -> Schema {
    Schema::builder()
        .exact(
            FieldPath::parse("params").unwrap(),
            Exact {
                conflict: Conflict::Reject,
                ..Exact::default()
            },
        )
        .build()
        .unwrap()
}

fn compare_params(left: Value, right: Value) -> univo_core::Comparison {
    let schema = params_schema();
    let left = prepare(&schema, &object(vec![("params", left)])).unwrap();
    let right = prepare(&schema, &object(vec![("params", right)])).unwrap();
    compare(&schema, &left, &right).unwrap().unwrap()
}

#[test]
fn nested_null_is_unknown_not_a_contradiction() {
    let result = compare_params(
        object(vec![("db", object(vec![("port", Value::Null)]))]),
        object(vec![("db", object(vec![("port", Value::Int(5432))]))]),
    );
    assert!(!result.vetoed, "{result:?}");
    assert!(result.fields[0].incomplete, "{result:?}");
    assert!(!result.fields[0].contradiction, "{result:?}");
}

#[test]
fn nested_differences_still_veto() {
    let result = compare_params(
        object(vec![(
            "db",
            object(vec![
                ("port", Value::Int(5432)),
                ("host", Value::Str("a".into())),
            ]),
        )]),
        object(vec![(
            "db",
            object(vec![
                ("port", Value::Int(5433)),
                ("host", Value::Str("a".into())),
            ]),
        )]),
    );
    assert!(result.vetoed, "{result:?}");
    assert_eq!(result.fields[0].similarity, Some(0.5));
}

#[test]
fn one_sided_keys_lower_the_coverage() {
    let result = compare_params(
        object(vec![("port", Value::Int(5432))]),
        object(vec![("port", Value::Int(5432)), ("tls", Value::Bool(true))]),
    );
    assert_eq!(result.score, Some(1.0));
    assert!(result.fields[0].incomplete);
    // One shared key, one key on one side only.
    assert_eq!(result.coverage, 0.5, "coverage hides a one-sided key");
}

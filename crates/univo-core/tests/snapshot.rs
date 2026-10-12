//! The versioned codec preserves decisions and refuses hostile lengths.
use proptest::prelude::*;
use univo_core::{Exact, FieldPath, Limits, Map, Schema, Text, Value, prepare, snapshot};

fn object(values: Vec<(&str, Value)>) -> Value {
    Value::Map(Map::new(values.into_iter().map(|(k, v)| (k.into(), v)).collect()).unwrap())
}

#[test]
fn round_trip_types_canonical_values_and_explanation_stages() {
    let schema = Schema::builder()
        .id_field(FieldPath::parse("id").unwrap())
        .partition_by(FieldPath::parse("tenant").unwrap())
        .text(FieldPath::parse("text").unwrap(), Text::default())
        .exact(FieldPath::parse("data").unwrap(), Exact::default())
        .build()
        .unwrap();
    let values = vec![
        Value::Null,
        Value::Bool(false),
        Value::Int(0),
        Value::float(0.0).unwrap(),
        Value::Str("e\u{301}".into()),
        Value::List(vec![Value::Int(1), Value::Int(2)]),
        object(vec![("b", Value::Int(2)), ("a", Value::Int(1))]),
    ];
    for id in [Value::Int(1), Value::Str("1".into())] {
        for value in &values {
            let prepared = prepare(
                &schema,
                &object(vec![
                    ("id", id.clone()),
                    ("tenant", Value::Bool(false)),
                    ("text", Value::Str("CAFE\u{301} STRAßE".into())),
                    ("data", value.clone()),
                ]),
            )
            .unwrap();
            let mut bytes = Vec::new();
            snapshot::write_record(&mut bytes, &prepared).unwrap();
            let restored =
                snapshot::read_record(&mut bytes.as_slice(), &schema, bytes.len()).unwrap();
            assert_eq!(restored, prepared);
            for end in 0..bytes.len() {
                assert!(snapshot::read_record(&mut &bytes[..end], &schema, end).is_err());
            }
        }
    }
}

#[test]
fn deepest_supported_value_round_trips() {
    let schema = Schema::builder()
        .limits(Limits {
            max_depth: 128,
            ..Default::default()
        })
        .exact(FieldPath::parse("data").unwrap(), Exact::default())
        .build()
        .unwrap();
    let mut value = Value::Bool(true);
    for _ in 0..128 {
        value = Value::List(vec![value]);
    }
    let prepared = prepare(&schema, &object(vec![("data", value)])).unwrap();
    let mut bytes = Vec::new();
    snapshot::write_record(&mut bytes, &prepared).unwrap();
    assert_eq!(
        snapshot::read_record(&mut bytes.as_slice(), &schema, bytes.len()).unwrap(),
        prepared
    );
}

#[test]
fn collection_overhead_is_checked_before_decoding_children() {
    let schema = Schema::builder()
        .limits(Limits {
            max_selected_bytes: 16,
            ..Default::default()
        })
        .exact(FieldPath::parse("data").unwrap(), Exact::default())
        .build()
        .unwrap();
    let mut bytes = vec![0, 3, 5]; // no ID, exact field, list
    bytes.extend_from_slice(&20u64.to_le_bytes());
    bytes.extend_from_slice(&[0; 20]); // null children
    bytes.extend_from_slice(&[0; 25]); // transforms and stage fingerprints
    assert!(snapshot::read_record(&mut bytes.as_slice(), &schema, bytes.len()).is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]
    #[test]
    fn bounded_random_inputs_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let schema = Schema::builder().limits(Limits { max_selected_bytes: 4096, ..Default::default() })
            .exact(FieldPath::parse("data").unwrap(), Exact::default()).build().unwrap();
        let _ = snapshot::read_record(&mut bytes.as_slice(), &schema, bytes.len());
    }
}

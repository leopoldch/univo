use univo_core::{
    ErrorCode, Exact, FieldPath, Map, PreparedRecord, Schema, Text, Transforms, Value, exact_match,
    prepare,
};
use univo_engine::ExactIndex;

fn path(name: &str) -> FieldPath {
    FieldPath::parse(name).unwrap()
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Map(
        Map::new(
            fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
        .unwrap(),
    )
}

fn text(value: &str) -> Value {
    Value::Str(value.to_owned())
}

fn structured_schema() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant"))
        .text(path("name"), Text::default())
        .exact(path("value"), Exact::default())
        .build()
        .unwrap()
}

fn record(schema: &Schema, id: Value, tenant: Value, name: &str) -> PreparedRecord {
    prepare(
        schema,
        &object(vec![("id", id), ("tenant", tenant), ("name", text(name))]),
    )
    .unwrap()
}

#[test]
fn indexed_search_agrees_with_exhaustive_pairwise_oracle() {
    let schema = structured_schema();
    let mut records = Vec::new();
    for index in 0..96 {
        let tenant = match index % 4 {
            0 => text("1"),
            1 => Value::Int(1),
            2 => Value::Bool(true),
            _ => text("other"),
        };
        let name = match (index / 4) % 4 {
            0 => "Café",
            1 => "CAFE\u{301}",
            2 => "Straße",
            _ => "STRASSE",
        };
        let mut fields = vec![
            ("id", Value::Int(index)),
            ("tenant", tenant),
            ("name", text(name)),
        ];
        match (index / 16) % 3 {
            0 => {}
            1 => fields.push(("value", Value::Null)),
            _ => fields.push(("value", Value::Int(0))),
        }
        records.push(prepare(&schema, &object(fields)).unwrap());
    }
    let mut index = ExactIndex::new(schema.clone(), records.len());
    assert_eq!(index.add(records.clone()).unwrap(), 0..records.len());
    assert_eq!(index.schema(), &schema);
    for query in &records {
        let mut expected: Vec<_> = records
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.id() != query.id())
            .filter_map(|(slot, candidate)| {
                exact_match(&schema, query, candidate)
                    .unwrap()
                    .map(|evidence| (slot, evidence))
            })
            .collect();
        expected.sort_by(|(left, _), (right, _)| records[*left].id().cmp(&records[*right].id()));
        let result = index.search(query, usize::MAX).unwrap();
        let actual: Vec<_> = result
            .matches
            .iter()
            .map(|hit| (hit.slot, hit.evidence.clone()))
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(result.total_matches, expected.len());
        assert!(!result.truncated());
    }
}

#[test]
fn ids_are_typed_ordered_and_excluded_without_normalization() {
    let schema = structured_schema();
    let ids = [
        Value::Int(3),
        text("b"),
        Value::Int(1),
        text("1"),
        text("A"),
    ];
    let records = ids
        .into_iter()
        .map(|id| record(&schema, id, text("tenant"), "same"))
        .collect();
    let mut index = ExactIndex::new(schema.clone(), 5);
    index.add(records).unwrap();
    let query = record(&schema, text("1"), text("tenant"), "same");
    let result = index.search(&query, 10).unwrap();
    let slots: Vec<_> = result.matches.iter().map(|hit| hit.slot).collect();
    assert_eq!(slots, [4, 1, 2, 0]);
    let other_case = record(&schema, text("a"), text("tenant"), "same");
    assert_eq!(index.search(&other_case, 10).unwrap().total_matches, 5);
    assert!(index.record(5).is_none());
    assert_eq!(
        index.record(4).unwrap().id(),
        Some(&univo_core::RecordId::Str("A".into()))
    );
}

#[test]
fn missing_states_and_blank_evidence_do_not_create_false_matches() {
    let schema = Schema::builder()
        .text(path("name"), Text::default())
        .exact(path("value"), Exact::default())
        .build()
        .unwrap();
    let values = [
        object(vec![]),
        object(vec![("name", Value::Null)]),
        object(vec![("name", text(""))]),
        object(vec![("name", text(" "))]),
        object(vec![("name", text("same"))]),
        object(vec![("name", text("same")), ("value", Value::Null)]),
        object(vec![("name", text("same")), ("value", Value::Int(0))]),
        object(vec![("name", text("same")), ("value", Value::Bool(false))]),
        object(vec![("name", text("same")), ("value", text("0"))]),
    ];
    let records: Vec<_> = values
        .iter()
        .map(|value| prepare(&schema, value).unwrap())
        .collect();
    let mut index = ExactIndex::new(schema, records.len());
    index.add(records.clone()).unwrap();
    for (slot, query) in records.iter().enumerate() {
        let result = index.search(query, 10).unwrap();
        let expected = if slot < 4 { vec![] } else { vec![slot] };
        assert_eq!(
            result
                .matches
                .iter()
                .map(|hit| hit.slot)
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn equal_signed_zero_is_retrieved_and_nested_types_stay_distinct() {
    let schema = Schema::builder()
        .exact(path("value"), Exact::default())
        .build()
        .unwrap();
    let values = [
        Value::float(-0.0).unwrap(),
        Value::float(0.0).unwrap(),
        Value::Int(0),
        Value::Bool(false),
        Value::List(vec![Value::Int(0), text("x")]),
        Value::List(vec![text("0"), text("x")]),
    ];
    let records: Vec<_> = values
        .into_iter()
        .map(|value| prepare(&schema, &object(vec![("value", value)])).unwrap())
        .collect();
    let mut index = ExactIndex::new(schema, records.len());
    index.add(records.clone()).unwrap();
    assert_eq!(index.search(&records[0], 10).unwrap().total_matches, 2);
    for query in &records[2..] {
        assert_eq!(index.search(query, 10).unwrap().total_matches, 1);
    }
}

#[test]
fn duplicate_ids_reject_whole_batches_even_across_partitions() {
    let schema = structured_schema();
    let first = record(&schema, text("id"), text("one"), "first");
    let duplicate = record(&schema, text("id"), text("two"), "second");
    let fresh = record(&schema, text("fresh"), text("one"), "fresh");
    let mut index = ExactIndex::new(schema, 10);
    let error = index
        .add(vec![first.clone(), duplicate.clone()])
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::DuplicateId);
    assert_eq!(error.record().unwrap().index, 1);
    assert_eq!(error.record().unwrap().id.as_ref(), duplicate.id());
    assert!(index.is_empty());
    index.add(vec![first.clone()]).unwrap();
    let error = index.add(vec![fresh.clone(), duplicate]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DuplicateId);
    assert_eq!(error.record().unwrap().index, 1);
    assert_eq!(index.len(), 1);
    assert_eq!(index.record(0), Some(&first));
    assert_eq!(index.add(vec![fresh]).unwrap(), 1..2);
}

#[test]
fn capacity_and_schema_failures_leave_index_unchanged() {
    let schema = Schema::strings();
    let first = prepare(&schema, &text("first")).unwrap();
    let second = prepare(&schema, &text("second")).unwrap();
    let mut index = ExactIndex::new(schema.clone(), 1);
    index.add(vec![first.clone()]).unwrap();
    let error = index.add(vec![second]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::IndexCapacity);
    assert_eq!(index.len(), 1);
    assert_eq!(index.record(0), Some(&first));
    assert_eq!(index.add(vec![]).unwrap(), 1..1);
    let mut empty = ExactIndex::new(schema, 0);
    assert_eq!(empty.add(vec![]).unwrap(), 0..0);
    assert_eq!(
        empty.add(vec![first.clone()]).unwrap_err().code(),
        ErrorCode::IndexCapacity
    );
    let other_schema = Schema::builder()
        .text(path("name"), Text::default())
        .build()
        .unwrap();
    let incompatible = prepare(&other_schema, &object(vec![("name", text("first"))])).unwrap();
    let mut roomy = ExactIndex::new(Schema::strings(), 3);
    assert_eq!(
        roomy
            .add(vec![first, incompatible.clone()])
            .unwrap_err()
            .code(),
        ErrorCode::SchemaMismatch
    );
    assert!(roomy.is_empty());
    assert_eq!(
        roomy.search(&incompatible, 10).unwrap_err().code(),
        ErrorCode::SchemaMismatch
    );
}

#[test]
fn output_limits_preserve_counts_and_normalization_evidence() {
    let schema = Schema::strings();
    let records = ["CAFÉ", "CAFE\u{301}", "café", "other"]
        .map(|value| prepare(&schema, &text(value)).unwrap());
    let mut index = ExactIndex::new(schema, records.len());
    index.add(records.to_vec()).unwrap();
    let none = index.search(&records[2], 0).unwrap();
    assert_eq!(none.matches, []);
    assert_eq!(none.total_matches, 3);
    assert!(none.truncated());
    let one = index.search(&records[2], 1).unwrap();
    assert_eq!(one.matches[0].slot, 0);
    assert_eq!(
        one.matches[0].evidence.normalized_fields,
        [(0, Transforms::CASE)]
    );
    assert_eq!(one.total_matches, 3);
    assert!(one.truncated());
    assert_eq!(one.candidates_examined, none.candidates_examined);
    let all = index.search(&records[2], 3).unwrap();
    assert!(!all.truncated());
    assert_eq!(
        all.matches.iter().map(|hit| hit.slot).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(all.matches[2].evidence.normalized_fields, []);
}

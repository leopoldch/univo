use std::path::Path;

use serde_json::Value as Json;
use univo_core::{
    Conflict, Error, Exact, ExactNormalizer, FieldPath, Map, PreparedRecord, Schema, Text,
    TextMode, Value, prepare,
};

/// Corpus cases whose expected error needs a batch, not a single record.
pub(crate) const BATCH_LEVEL_CASES: &[&str] = &["adversarial-009"];

pub(crate) fn path(dotted: &str) -> FieldPath {
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

pub(crate) fn schema(profile: &str) -> Schema {
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

pub(crate) fn to_value(json: &Json) -> Value {
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

/// Corpus labels name the exception a binding raises for the error kind.
pub(crate) fn error_name(error: &Error) -> String {
    format!("{}Error", error.kind().name())
}

pub(crate) fn cases() -> Vec<Json> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/critical/cases.jsonl");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

pub(crate) fn prepare_pair(
    case: &Json,
    schema: &Schema,
) -> (Result<PreparedRecord, Error>, Result<PreparedRecord, Error>) {
    (
        prepare(schema, &to_value(&case["left"])),
        prepare(schema, &to_value(&case["right"])),
    )
}

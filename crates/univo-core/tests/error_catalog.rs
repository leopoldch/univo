//! Every error code, with a representative example.
//!
//! The examples check that each code is reachable, that messages name paths
//! and types but never field values, and they generate the catalog in
//! `docs/errors.md`, which bindings for every language share. Regenerate it
//! with `UNIVO_UPDATE_DOCS=1 cargo test -p univo-core --test error_catalog`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use univo_core::{
    Error, ErrorCode, Exact, ExactNormalizer, FieldPath, Limits, Map, Schema, Text, Value,
    exact_match, prepare,
};

/// Every string value in the examples contains this marker, so a message
/// that leaks a field value is detected.
const SECRET: &str = "s3cr3t";

const START: &str = "<!-- catalog:start -->";
const END: &str = "<!-- catalog:end -->";

fn path(dotted: &str) -> FieldPath {
    FieldPath::parse(dotted).unwrap()
}

fn secret(suffix: &str) -> Value {
    Value::Str(format!("{SECRET}-{suffix}"))
}

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

fn schema_error(builder: univo_core::SchemaBuilder) -> Error {
    builder.build().unwrap_err()
}

fn record_error(schema: &Schema, record: &Value) -> Error {
    prepare(schema, record).unwrap_err()
}

fn organizations() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant"))
        .text(path("name"), Text::default())
        .exact(
            path("domain"),
            Exact {
                normalizer: ExactNormalizer::Hostname,
                ..Exact::default()
            },
        )
        .exact(path("params"), Exact::default())
        .text(
            path("description"),
            Text {
                required: true,
                ..Text::default()
            },
        )
        .build()
        .unwrap()
}

/// A valid organization with `overrides` applied.
fn organization(overrides: Vec<(&str, Option<Value>)>) -> Value {
    let mut entries = vec![
        ("id", Some(secret("id"))),
        ("tenant", Some(secret("tenant"))),
        ("name", Some(secret("name"))),
        ("domain", Some(Value::Str("example.com".into()))),
        ("params", Some(object(vec![("retries", Value::Int(3))]))),
        ("description", Some(secret("description"))),
    ];
    for (key, value) in overrides {
        entries.retain(|(existing, _)| *existing != key);
        entries.push((key, value));
    }
    object(
        entries
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect(),
    )
}

fn mismatched_records() -> Error {
    let unfolded = Schema::builder()
        .text(
            path("name"),
            Text {
                case_fold: false,
                ..Text::default()
            },
        )
        .build()
        .unwrap();
    let strings = Schema::strings();
    let left = prepare(&strings, &secret("left")).unwrap();
    let right = prepare(&unfolded, &object(vec![("name", secret("right"))])).unwrap();
    exact_match(&strings, &left, &right).unwrap_err()
}

fn examples() -> Vec<Error> {
    let name = || path("name");
    let organizations = organizations();
    let mut nested = Value::Int(1);
    for _ in 0..20 {
        nested = Value::List(vec![nested]);
    }
    let small = Schema::builder()
        .text(name(), Text::default())
        .limits(Limits {
            max_selected_bytes: 64,
            ..Limits::default()
        })
        .build()
        .unwrap();

    vec![
        schema_error(Schema::builder()),
        schema_error({
            let mut builder = Schema::builder();
            for index in 0..65 {
                builder = builder.text(path(&format!("field_{index}")), Text::default());
            }
            builder
        }),
        schema_error(Schema::builder().text(
            name(),
            Text {
                weight: 0.0,
                ..Text::default()
            },
        )),
        schema_error(
            Schema::builder()
                .text(name(), Text::default())
                .limits(Limits {
                    max_depth: 500,
                    ..Limits::default()
                }),
        ),
        FieldPath::parse("params..retries").unwrap_err(),
        schema_error(
            Schema::builder()
                .exact(path("params"), Exact::default())
                .exact(path("params.retries"), Exact::default()),
        ),
        schema_error(Schema::builder().text(FieldPath::root(), Text::default())),
        record_error(&organizations, &secret("record")),
        record_error(
            &organizations,
            &organization(vec![("name", Some(Value::Int(42)))]),
        ),
        Error::unsupported_type(&path("created_at"), "datetime.datetime"),
        Error::non_finite_number(&path("score")),
        Error::integer_out_of_range(&path("population")),
        record_error(&organizations, &organization(vec![("id", None)])),
        record_error(
            &organizations,
            &organization(vec![("tenant", Some(Value::Null))]),
        ),
        record_error(&organizations, &organization(vec![("description", None)])),
        Error::duplicate_key(&path("params"), "retries"),
        record_error(
            &organizations,
            &organization(vec![(
                "params",
                Some(object(vec![
                    ("cafe\u{301}", secret("a")),
                    ("caf\u{e9}", secret("b")),
                ])),
            )]),
        ),
        record_error(
            &organizations,
            &organization(vec![("domain", Some(Value::Str("example.com..".into())))]),
        ),
        record_error(
            &organizations,
            &organization(vec![("params", Some(nested))]),
        ),
        record_error(
            &small,
            &object(vec![("name", Value::Str(SECRET.repeat(20)))]),
        ),
        mismatched_records(),
        Error::duplicate_id().with_record(
            1,
            Some(univo_core::RecordId::Str(format!("{SECRET}-id").into())),
        ),
        Error::index_capacity(3, 2),
    ]
    .into_iter()
    .chain(model_examples())
    .collect()
}

/// Model errors are raised by univo-embed and the bindings, which depend on
/// this crate; their tests trigger them for real.
fn model_examples() -> Vec<Error> {
    vec![
        Error::invalid_model_spec(
            "potion",
            "expected \"default\", \"org/name[@revision]\" or a local path",
        ),
        Error::model_not_available(
            "minishlab/potion-base-8M",
            "offline mode is on and the model is not in the cache",
        ),
        Error::unsupported_model_format(
            "sentence-transformers/all-MiniLM-L6-v2",
            "the repository has no model.safetensors",
        ),
        Error::model_download_failed(
            "minishlab/potion-base-8M",
            "model.safetensors",
            "connection timed out",
        ),
        Error::model_cache_locked(
            "minishlab/potion-base-8M",
            "/cache/models/minishlab/potion-base-8M/bf8b0566.lock",
        ),
        Error::model_too_large(
            "minishlab/potion-multilingual-128M",
            3_000_000_000,
            2_147_483_648,
        ),
        Error::model_integrity(
            "minishlab/potion-base-8M",
            "model.safetensors",
            "size is 1024 bytes, expected 30236760",
        ),
        Error::corrupt_model("/opt/models/m", "a vector is not finite"),
    ]
}

#[test]
fn every_code_is_triggered_by_an_example() {
    let triggered: BTreeSet<_> = examples()
        .iter()
        .map(|error| error.code().as_str())
        .collect();
    let all: BTreeSet<_> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
    assert_eq!(triggered, all);
}

#[test]
fn messages_never_contain_field_values() {
    for error in examples() {
        let text = error.to_string();
        assert!(!text.contains(SECRET), "{text}");
        for (_, detail) in error.details() {
            assert!(!detail.to_string().contains(SECRET), "{text}");
        }
    }
}

#[test]
fn record_identifiers_never_appear_in_text_or_debug_output() {
    let id = univo_core::RecordId::Str(format!("{SECRET}-id").into());
    for error in examples() {
        let located = error.with_record(3, Some(id.clone()));
        assert_eq!(
            located.record().and_then(|record| record.id.as_ref()),
            Some(&id)
        );
        for text in [
            located.to_string(),
            format!("{located:?}"),
            format!("{located:#?}"),
        ] {
            assert!(!text.contains(SECRET), "{text}");
        }
    }
}

#[test]
fn nested_key_collisions_name_their_location() {
    let ambiguous = object(vec![
        ("cafe\u{301}", secret("a")),
        ("caf\u{e9}", secret("b")),
    ]);
    let record = organization(vec![(
        "params",
        Some(object(vec![(
            "nested",
            Value::List(vec![Value::Null, ambiguous]),
        )])),
    )]);
    let error = record_error(&organizations(), &record);
    assert_eq!(error.code(), ErrorCode::KeyCollision);
    assert_eq!(
        error.message(),
        "field `params` contains, at `nested[1]`, an object with two keys that both become \"caf\u{e9}\" after Unicode normalization"
    );
}

#[test]
fn field_errors_name_their_path() {
    for error in examples() {
        if let Some(path) = error.path().filter(|path| !path.is_root()) {
            assert!(error.message().contains(&format!("`{path}`")), "{error}");
        }
    }
}

fn catalog() -> String {
    let examples = examples();
    let mut out = String::new();
    for code in ErrorCode::ALL {
        let example = examples
            .iter()
            .find(|error| error.code() == *code)
            .expect("every code has an example");
        write!(
            out,
            "\n### {}\n\n{}.\n\n- Kind: `{}` (Python: `univo.{}Error`)\n- Fix: {}\n\nExample:\n\n```text\n{}\n```\n",
            code.as_str(),
            code.title(),
            code.kind().name(),
            code.kind().name(),
            code.hint(),
            example,
        )
        .unwrap();
    }
    out
}

#[test]
fn documentation_matches_the_catalog() {
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/errors.md");
    let current = std::fs::read_to_string(&docs).unwrap();
    let start = current.find(START).unwrap() + START.len();
    let end = current.find(END).unwrap();
    let expected = format!("{}{}\n{}", &current[..start], catalog(), &current[end..]);
    if std::env::var_os("UNIVO_UPDATE_DOCS").is_some() {
        std::fs::write(&docs, &expected).unwrap();
        return;
    }
    assert!(
        current == expected,
        "docs/errors.md is stale; run UNIVO_UPDATE_DOCS=1 cargo test -p univo-core --test error_catalog"
    );
}

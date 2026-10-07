//! Resource limits must be enforced before the work they bound: no deep
//! recursion on untrusted nesting and no copy of oversized input.

use std::time::{Duration, Instant};

use univo_core::{Detail, ErrorCode, Exact, FieldPath, Map, Schema, Text, Value, prepare};

const SMALL_STACK: usize = 256 * 1024;

fn path(dotted: &str) -> FieldPath {
    FieldPath::parse(dotted).unwrap()
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

/// Builds a list nested `depth` times without recursion.
fn deeply_nested(depth: usize) -> Value {
    let mut value = Value::Null;
    for _ in 0..depth {
        value = Value::List(vec![value]);
    }
    value
}

/// Runs `test` on a thread with a small stack, so that unbounded recursion
/// aborts the test instead of passing by luck on a large main stack.
fn on_small_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_nesting_is_rejected_without_deep_recursion() {
    on_small_stack(|| {
        let schema = Schema::builder()
            .exact(path("params"), Exact::default())
            .build()
            .unwrap();
        let record = object(vec![("params", deeply_nested(100_000))]);
        let error = prepare(&schema, &record).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NestingTooDeep);
        // Dropping the value would recurse as deeply as it is nested; the
        // engine never owns such values, so the test leaks it instead.
        std::mem::forget(record);
    });
}

#[test]
fn deep_nesting_inside_objects_is_rejected_without_deep_recursion() {
    on_small_stack(|| {
        let mut nested = Value::Null;
        for _ in 0..100_000 {
            nested = object(vec![("level", nested)]);
        }
        let schema = Schema::builder()
            .exact(path("params"), Exact::default())
            .build()
            .unwrap();
        let record = object(vec![("params", nested)]);
        assert_eq!(
            prepare(&schema, &record)
                .map_err(|error| error.code())
                .err(),
            Some(ErrorCode::NestingTooDeep)
        );
        std::mem::forget(record);
    });
}

fn count(error: &univo_core::Error, key: &str) -> usize {
    error
        .details()
        .iter()
        .find_map(|(name, detail)| match detail {
            Detail::Count(value) if *name == key => usize::try_from(*value).ok(),
            _ => None,
        })
        .unwrap()
}

/// Limit, size reached when preparation stopped, and path of a size
/// rejection.
fn rejected_size(schema: &Schema, record: &Value) -> (usize, usize, String) {
    let error = prepare(schema, record).unwrap_err();
    assert_eq!(error.code(), ErrorCode::RecordTooLarge, "{error}");
    let path = error.path().unwrap().to_string();
    (count(&error, "limit"), count(&error, "reached"), path)
}

/// A single string is charged from its length, without being read: even in
/// a debug build on a slow runner, rejecting 64 MiB takes microseconds,
/// while copying and normalizing it takes tens of milliseconds.
const STRING_REJECTION_BOUND: Duration = Duration::from_millis(10);

#[test]
fn oversized_text_is_rejected_before_being_copied() {
    let huge = "A".repeat(64 << 20);
    let schema = Schema::builder()
        .text(path("name"), Text::default())
        .build()
        .unwrap();
    for (schema, record) in [
        (Schema::strings(), Value::Str(huge.clone())),
        (schema, object(vec![("name", Value::Str(huge))])),
    ] {
        let start = Instant::now();
        let (limit, actual, _) = rejected_size(&schema, &record);
        let elapsed = start.elapsed();
        assert!(actual > limit);
        assert!(
            elapsed < STRING_REJECTION_BOUND,
            "rejection took {elapsed:?}"
        );
    }
}

#[test]
fn oversized_collections_stop_at_the_limit() {
    let schema = Schema::builder()
        .exact(path("params"), Exact::default())
        .build()
        .unwrap();
    let wide = object(vec![("params", Value::List(vec![Value::Int(1); 8 << 20]))]);
    // The full list weighs 64 MiB; the traversal must stop at the first
    // element above the 1 MiB limit, each element being charged 8 bytes.
    let (limit, actual, path) = rejected_size(&schema, &wide);
    assert!(actual > limit && actual <= limit + 8, "stopped at {actual}");
    assert_eq!(path, "params");
}

#[test]
fn identifiers_and_partition_keys_count_towards_the_budget() {
    let schema = Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant"))
        .text(path("name"), Text::default())
        .build()
        .unwrap();
    let huge = "x".repeat(2 << 20);
    for (id, tenant, rejected_at) in [(huge.as_str(), "t", "id"), ("1", huge.as_str(), "tenant")] {
        let record = object(vec![
            ("id", Value::Str(id.into())),
            ("tenant", Value::Str(tenant.into())),
            ("name", Value::Str("Acme".into())),
        ]);
        let (_, _, path) = rejected_size(&schema, &record);
        assert_eq!(path, rejected_at);
    }
}

#[test]
fn records_from_another_schema_cannot_be_compared() {
    let folded = Schema::strings();
    let exact = Schema::builder()
        .text(
            path("name"),
            Text {
                case_fold: false,
                ..Text::default()
            },
        )
        .build()
        .unwrap();
    let left = prepare(&folded, &Value::Str("Acme".into())).unwrap();
    let right = prepare(&exact, &object(vec![("name", Value::Str("Acme".into()))])).unwrap();
    let error = univo_core::exact_match(&folded, &left, &right).unwrap_err();
    assert_eq!(error.code(), ErrorCode::SchemaMismatch);
}

#[test]
fn records_prepared_without_a_requirement_cannot_bypass_it() {
    let schema = |required| {
        Schema::builder()
            .text(path("name"), Text::default())
            .exact(
                path("sku"),
                Exact {
                    required,
                    ..Exact::default()
                },
            )
            .build()
            .unwrap()
    };
    let lenient = schema(false);
    let strict = schema(true);
    let record = object(vec![("name", Value::Str("Acme".into()))]);
    assert!(prepare(&strict, &record).is_err());

    let left = prepare(&lenient, &record).unwrap();
    let right = prepare(&lenient, &record).unwrap();
    assert!(univo_core::exact_match(&strict, &left, &right).is_err());
}

/// The deepest value accepted under the highest depth limit must be
/// prepared, compared and dropped on a small stack, in debug as in release.
#[test]
fn the_deepest_accepted_values_need_little_stack() {
    on_small_stack(|| {
        let limits = univo_core::Limits {
            max_depth: univo_core::MAX_NESTING_DEPTH,
            ..univo_core::Limits::default()
        };
        let schema = Schema::builder()
            .exact(path("params"), Exact::default())
            .limits(limits)
            .build()
            .unwrap();
        // Unsorted keys and a decomposed leaf make normalization change the
        // value, so that the explanation fingerprints are computed too.
        for list in [false, true] {
            let mut nested = Value::Str("Cafe\u{301}".into());
            for level in 0..univo_core::MAX_NESTING_DEPTH {
                nested = if list {
                    Value::List(vec![nested, Value::Int(i64::try_from(level).unwrap())])
                } else {
                    object(vec![("level", nested), ("Zz", Value::Int(1))])
                };
            }
            let record = object(vec![("params", nested)]);
            let left = prepare(&schema, &record).unwrap();
            let right = prepare(&schema, &record).unwrap();
            assert!(
                univo_core::exact_match(&schema, &left, &right)
                    .unwrap()
                    .is_some()
            );
            drop((left, right, record));
        }
    });
}

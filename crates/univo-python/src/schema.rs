//! Python schemas and the conversion of Python records to prepared records.

use std::hash::{Hash, Hasher};

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString};
use univo_core::{
    Conflict, Error, Exact, ExactNormalizer, FieldKind, FieldPath, Limits, Map, PreparedRecord,
    RecordId, Schema, Text, TextMode, Value, prepare,
};

use crate::errors::to_py_err;
use crate::models::Model;

type FieldSpec = (String, String, String, f32, bool, bool, String);

#[derive(Debug, Clone)]
#[pyclass(frozen, skip_from_py_object, module = "univo._univo", name = "Schema")]
pub(crate) struct PySchema {
    pub(crate) schema: Schema,
}

#[pymethods]
impl PySchema {
    #[new]
    #[pyo3(signature = (fields=None, id_field=None, partition_by=Vec::new(), limits=(64,16,1_048_576)))]
    fn new(
        py: Python<'_>,
        fields: Option<Vec<FieldSpec>>,
        id_field: Option<String>,
        partition_by: Vec<String>,
        limits: (usize, usize, usize),
    ) -> PyResult<Self> {
        let result = (|| {
            let Some(fields) = fields else {
                return Ok(Schema::strings());
            };
            let mut builder = Schema::builder().limits(Limits {
                max_fields: limits.0,
                max_depth: limits.1,
                max_selected_bytes: limits.2,
            });
            if let Some(id) = id_field {
                builder = builder.id_field(FieldPath::parse(&id)?);
            }
            for path in partition_by {
                builder = builder.partition_by(FieldPath::parse(&path)?);
            }
            for (path, kind, mode, weight, required, case_fold, conflict) in fields {
                let path = FieldPath::parse(&path)?;
                builder = match kind.as_str() {
                    "text" => builder.text(
                        path,
                        Text {
                            mode: match mode.as_str() {
                                "hybrid" => TextMode::Hybrid,
                                "lexical" => TextMode::Lexical,
                                "semantic" => TextMode::Semantic,
                                _ => {
                                    return Err(Error::invalid_option(
                                        "mode",
                                        "lexical, semantic or hybrid",
                                    ));
                                }
                            },
                            weight,
                            required,
                            case_fold,
                        },
                    ),
                    "exact" => builder.exact(
                        path,
                        Exact {
                            normalizer: match mode.as_str() {
                                "identity" => ExactNormalizer::Identity,
                                "hostname" => ExactNormalizer::Hostname,
                                _ => {
                                    return Err(Error::invalid_option(
                                        "normalizer",
                                        "identity or hostname",
                                    ));
                                }
                            },
                            weight,
                            required,
                            conflict: match conflict.as_str() {
                                "evidence" => Conflict::Evidence,
                                "reject" => Conflict::Reject,
                                _ => {
                                    return Err(Error::invalid_option(
                                        "conflict",
                                        "evidence or reject",
                                    ));
                                }
                            },
                        },
                    ),
                    _ => return Err(Error::invalid_option("fields", "Text or Exact fields")),
                };
            }
            builder.build()
        })();
        result
            .map(|schema| Self { schema })
            .map_err(|e| to_py_err(py, &e))
    }
    #[getter]
    fn needs_model(&self) -> bool {
        has_semantic(&self.schema)
    }
    fn check_mutation(&self, py: Python<'_>, operation: &str) -> PyResult<()> {
        if self.schema.id_field().is_none() {
            return Err(to_py_err(
                py,
                &Error::invalid_option(operation, "a schema with id_field"),
            ));
        }
        Ok(())
    }

    fn missing_ids(
        &self,
        py: Python<'_>,
        inputs: &Bound<'_, PyAny>,
        max: usize,
    ) -> PyResult<Vec<Py<PyAny>>> {
        ids(py, &self.schema, inputs, max)?
            .iter()
            .map(|value| external_id(py, value))
            .collect()
    }

    #[pyo3(signature = (model_ref, model=None, ann=None))]
    fn configuration_fingerprint(
        &self,
        model_ref: &str,
        model: Option<&Model>,
        ann: Option<(usize, usize)>,
    ) -> String {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}:{}", self.schema, univo_core::SCORING_VERSION).hash(&mut hash);
        univo_engine::SEARCH_VERSION.hash(&mut hash);
        model_ref.hash(&mut hash);
        if let Some(options) = ann {
            univo_engine::ann::BACKEND.hash(&mut hash);
            options.hash(&mut hash);
        }
        if let Some(model) = model {
            use univo_embed::Embedder;
            format!("{:?}", model.embedder.descriptor()).hash(&mut hash);
        }
        format!("{:016x}", hash.finish())
    }
}

fn child_path(path: &FieldPath, key: &str) -> univo_core::Result<FieldPath> {
    if key.is_empty() {
        return Ok(path.clone());
    }
    FieldPath::from_keys(
        path.keys()
            .iter()
            .map(AsRef::as_ref)
            .chain(std::iter::once(key)),
    )
}

fn convert(
    value: &Bound<'_, PyAny>,
    path: &FieldPath,
    depth: usize,
    limits: Limits,
    used: &mut usize,
) -> univo_core::Result<Value> {
    *used = used.saturating_add(8);
    if *used > limits.max_selected_bytes {
        return Err(Error::record_too_large(
            path,
            limits.max_selected_bytes,
            *used,
        ));
    }
    if value.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(v) = value.cast::<PyBool>() {
        return Ok(Value::Bool(v.is_true()));
    }
    if let Ok(v) = value.cast::<PyInt>() {
        return v
            .extract::<i64>()
            .map(Value::Int)
            .map_err(|_| Error::integer_out_of_range(path));
    }
    if let Ok(v) = value.cast::<PyFloat>() {
        return Value::float(v.value()).ok_or_else(|| Error::non_finite_number(path));
    }
    if let Ok(v) = value.cast::<PyString>() {
        let text = v
            .to_str()
            .map_err(|_| Error::unsupported_type(path, "non-UTF-8 string"))?;
        *used = used.saturating_add(text.len());
        if *used > limits.max_selected_bytes {
            return Err(Error::record_too_large(
                path,
                limits.max_selected_bytes,
                *used,
            ));
        }
        return Ok(Value::Str(text.to_owned()));
    }
    if depth >= limits.max_depth {
        return Err(Error::nesting_too_deep(path, limits.max_depth));
    }
    if let Ok(v) = value.cast::<PyList>() {
        let items = v
            .iter()
            .map(|item| convert(&item, path, depth + 1, limits, used))
            .collect::<univo_core::Result<_>>()?;
        return Ok(Value::List(items));
    }
    if let Ok(v) = value.cast::<PyDict>() {
        let mut entries = Vec::new();
        for (key, item) in v.iter() {
            let key = key
                .cast::<PyString>()
                .map_err(|_| Error::unsupported_type(path, "non-string object key"))?
                .to_str()
                .map_err(|_| Error::unsupported_type(path, "non-UTF-8 key"))?;
            *used = used.saturating_add(key.len());
            if *used > limits.max_selected_bytes {
                return Err(Error::record_too_large(
                    path,
                    limits.max_selected_bytes,
                    *used,
                ));
            }
            let child = child_path(path, key)?;
            entries.push((
                key.to_owned(),
                convert(&item, &child, depth + 1, limits, used)?,
            ));
        }
        return Map::new(entries)
            .map(Value::Map)
            .map_err(|e| Error::duplicate_key(path, &e.0));
    }
    Err(Error::unsupported_type(path, "unsupported Python object"))
}

fn project(
    input: &Bound<'_, PyAny>,
    paths: &[&FieldPath],
    depth: usize,
    limits: Limits,
    used: &mut usize,
) -> univo_core::Result<Value> {
    let parent = paths[0].prefix(depth);
    let dict = input
        .cast::<PyDict>()
        .map_err(|_| Error::wrong_shape(&parent, "an object", "non-object"))?;
    let mut groups: Vec<(&str, Vec<&FieldPath>)> = Vec::new();
    for path in paths {
        let key = path.keys()[depth].as_ref();
        if let Some((_, group)) = groups.iter_mut().find(|(k, _)| *k == key) {
            group.push(path);
        } else {
            groups.push((key, vec![path]));
        }
    }
    let mut entries = Vec::new();
    for (key, group) in groups {
        if let Some(value) = dict
            .get_item(key)
            .map_err(|_| Error::unsupported_type(&parent, "object lookup"))?
        {
            let path = group[0];
            let value = if path.keys().len() == depth + 1 {
                convert(&value, path, 0, limits, used)?
            } else {
                project(&value, &group, depth + 1, limits, used)?
            };
            entries.push((key.to_owned(), value));
        }
    }
    Map::new(entries)
        .map(Value::Map)
        .map_err(|e| Error::duplicate_key(&parent, &e.0))
}

pub(crate) fn record(
    schema: &Schema,
    input: &Bound<'_, PyAny>,
) -> univo_core::Result<PreparedRecord> {
    let mut used = 0;
    if schema.compares_whole_record() {
        return prepare(
            schema,
            &convert(input, &FieldPath::root(), 0, schema.limits(), &mut used)?,
        );
    }
    let paths: Vec<_> = schema
        .id_field()
        .into_iter()
        .chain(schema.partition_by())
        .chain(schema.fields().iter().map(|f| &f.path))
        .collect();
    for path in &paths {
        if path.keys().len() > univo_core::MAX_NESTING_DEPTH {
            return Err(Error::nesting_too_deep(path, univo_core::MAX_NESTING_DEPTH));
        }
    }
    prepare(
        schema,
        &project(input, &paths, 0, schema.limits(), &mut used)?,
    )
}

pub(crate) fn batch(
    py: Python<'_>,
    schema: &Schema,
    inputs: &Bound<'_, PyAny>,
    max: usize,
) -> PyResult<Vec<PreparedRecord>> {
    let mut records = Vec::new();
    for (i, input) in inputs.try_iter()?.enumerate() {
        if i >= max {
            return Err(to_py_err(
                py,
                &Error::index_capacity(i.saturating_add(1), max),
            ));
        }
        records.push(record(schema, &input?).map_err(|e| to_py_err(py, &e.with_record(i, None)))?);
    }
    Ok(records)
}

pub(crate) fn ids(
    py: Python<'_>,
    schema: &Schema,
    inputs: &Bound<'_, PyAny>,
    max: usize,
) -> PyResult<Vec<RecordId>> {
    let path = schema.id_field().ok_or_else(|| {
        to_py_err(
            py,
            &Error::invalid_option("remove", "a schema with id_field"),
        )
    })?;
    let mut output = Vec::new();
    for (position, input) in inputs.try_iter()?.enumerate() {
        if position >= max {
            return Err(to_py_err(
                py,
                &Error::index_capacity(position.saturating_add(1), max),
            ));
        }
        let converted = convert(&input?, path, 0, schema.limits(), &mut 0)
            .and_then(|value| match value {
                Value::Str(text) => Ok(RecordId::Str(text.into_boxed_str())),
                Value::Int(value) => Ok(RecordId::Int(value)),
                other => Err(Error::wrong_shape(
                    path,
                    "a string or an integer",
                    other.type_name(),
                )),
            })
            .map_err(|error| to_py_err(py, &error.with_record(position, None)))?;
        output.push(converted);
    }
    Ok(output)
}

pub(crate) fn external_id(py: Python<'_>, id: &RecordId) -> PyResult<Py<PyAny>> {
    match id {
        RecordId::Str(text) => Ok(text.as_ref().into_pyobject(py)?.into_any().unbind()),
        RecordId::Int(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
    }
}

pub(crate) fn has_semantic(schema: &Schema) -> bool {
    schema
        .fields()
        .iter()
        .any(|f| matches!(f.kind, FieldKind::Text(t) if t.mode == TextMode::Semantic))
}

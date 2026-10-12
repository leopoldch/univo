//! Schema and record conversion; comparison and retrieval are delegated to Rust.
use crate::{errors::to_py_err, models::Model};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use univo_core::{
    Comparison, Conflict, Error, Exact, ExactNormalizer, FieldKind, FieldPath, Limits, Map,
    PreparedRecord, RecordId, Schema, Text, TextMode, Value, exact_match, prepare,
};
use univo_engine::{SearchIndex, SearchOptions};

type FieldSpec = (String, String, String, f32, bool, bool, String);

#[derive(Debug, Clone)]
#[pyclass(frozen, skip_from_py_object, module = "univo._univo", name = "Schema")]
pub(crate) struct PySchema {
    schema: Schema,
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
        self.schema
            .fields()
            .iter()
            .any(|f| matches!(f.kind, FieldKind::Text(t) if t.mode == TextMode::Semantic))
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

fn record(schema: &Schema, input: &Bound<'_, PyAny>) -> univo_core::Result<PreparedRecord> {
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

fn batch(
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

fn ids(
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

fn external_id(py: Python<'_>, id: &RecordId) -> PyResult<Py<PyAny>> {
    match id {
        RecordId::Str(text) => Ok(text.as_ref().into_pyobject(py)?.into_any().unbind()),
        RecordId::Int(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
    }
}

#[derive(Debug)]
#[pyclass(module = "univo._univo", name = "Index")]
pub(crate) struct NativeIndex {
    index: SearchIndex,
    max_records: usize,
    generation: u64,
    pool: Option<Arc<rayon::ThreadPool>>,
}

fn id(py: Python<'_>, record: &PreparedRecord, slot: usize) -> PyResult<Py<PyAny>> {
    match record.id() {
        Some(RecordId::Str(id)) => Ok(id.as_ref().into_pyobject(py)?.into_any().unbind()),
        Some(RecordId::Int(id)) => Ok(id.into_pyobject(py)?.into_any().unbind()),
        None => Ok(slot.into_pyobject(py)?.into_any().unbind()),
    }
}

fn match_dict<'py>(
    py: Python<'py>,
    index: &SearchIndex,
    left: &PreparedRecord,
    left_slot: usize,
    right_slot: usize,
    comparison: &Comparison,
) -> PyResult<Bound<'py, PyDict>> {
    let right = &index.store().records()[right_slot];
    let exact = exact_match(index.schema(), left, right).map_err(|e| to_py_err(py, &e))?;
    let out = PyDict::new(py);
    out.set_item("left_id", id(py, left, left_slot)?)?;
    out.set_item("right_id", id(py, right, right_slot)?)?;
    out.set_item("score", comparison.score)?;
    out.set_item("coverage", comparison.coverage)?;
    out.set_item(
        "kind",
        if exact.is_some() {
            "exact"
        } else {
            "candidate"
        },
    )?;
    let fields = PyDict::new(py);
    let mut reasons = Vec::new();
    let mut conflicts = Vec::new();
    for (field, score) in index.schema().fields().iter().zip(comparison.fields.iter()) {
        let path = field.path.to_string();
        fields.set_item(&path, score.similarity)?;
        reasons.push(format!("{path}: {:?}", score.signal));
        if score.contradiction {
            conflicts.push(path);
        }
    }
    if let Some(exact) = exact {
        for (field, steps) in exact.normalized_fields {
            reasons.push(format!(
                "{}: normalized ({})",
                index.schema().fields()[field].path,
                steps.names().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    out.set_item("field_scores", fields)?;
    out.set_item("reasons", reasons)?;
    out.set_item("conflicts", conflicts)?;
    Ok(out)
}

#[pymethods]
impl NativeIndex {
    #[new]
    #[pyo3(signature = (schema, max_records, model=None, threads=1, cache=None, ann=None))]
    fn new(
        py: Python<'_>,
        schema: &PySchema,
        max_records: usize,
        model: Option<&Model>,
        threads: usize,
        cache: Option<&PyEmbeddingCache>,
        ann: Option<(usize, usize)>,
    ) -> PyResult<Self> {
        let pool = if schema.needs_model() {
            Some(Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .map_err(|_| {
                        to_py_err(
                            py,
                            &Error::invalid_option("threads", "an available worker pool"),
                        )
                    })?,
            ))
        } else {
            None
        };
        let embedder = model.map(|m| Arc::clone(&m.embedder) as Arc<dyn univo_embed::Embedder>);
        py.detach(|| {
            let index = SearchIndex::new(schema.schema.clone(), max_records, embedder)?;
            let index = match cache {
                Some(cache) => index.with_embedding_cache(Arc::clone(&cache.cache))?,
                None => index,
            };
            match ann {
                Some((ef_search, max_candidates)) => index.with_ann(univo_engine::AnnOptions {
                    ef_search,
                    max_candidates,
                }),
                None => Ok(index),
            }
        })
        .map(|index| Self {
            index,
            max_records,
            generation: 0,
            pool,
        })
        .map_err(|e| to_py_err(py, &e))
    }
    #[pyo3(signature = (inputs, semantic_budget=None))]
    fn add(
        &mut self,
        py: Python<'_>,
        inputs: &Bound<'_, PyAny>,
        semantic_budget: Option<usize>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let records = batch(
            py,
            self.index.schema(),
            inputs,
            self.max_records - self.index.len(),
        )?;
        let next_generation = self.next_generation(py, !records.is_empty())?;
        let slots = py
            .detach(|| {
                if let Some(budget) = semantic_budget {
                    self.index.check_find_batch(&records, budget)?;
                }
                run(self.pool.as_deref(), || self.index.add(records))
            })
            .map_err(|e| to_py_err(py, &e))?;
        self.generation = next_generation;
        slots
            .map(|slot| id(py, &self.index.store().records()[slot], slot))
            .collect()
    }
    fn upsert(&mut self, py: Python<'_>, inputs: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
        let records = batch(py, self.index.schema(), inputs, self.max_records)?;
        let generation = self.next_generation(py, !records.is_empty())?;
        let slots = py
            .detach(|| run(self.pool.as_deref(), || self.index.upsert(records)))
            .map_err(|e| to_py_err(py, &e))?;
        self.generation = generation;
        slots
            .into_iter()
            .map(|slot| id(py, &self.index.store().records()[slot], slot))
            .collect()
    }

    fn remove(&mut self, py: Python<'_>, inputs: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
        let ids = ids(py, self.index.schema(), inputs, self.max_records)?;
        let changed = ids.iter().any(|id| self.index.store().contains_id(id));
        let generation = self.next_generation(py, changed)?;
        let missing = py
            .detach(|| self.index.remove(&ids))
            .map_err(|e| to_py_err(py, &e))?;
        self.generation = generation;
        missing.iter().map(|value| external_id(py, value)).collect()
    }

    fn search<'py>(
        &self,
        py: Python<'py>,
        input: &Bound<'_, PyAny>,
        min_score: f32,
        limit: usize,
        max_entries: usize,
        exhaustive: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query = record(self.index.schema(), input).map_err(|e| to_py_err(py, &e))?;
        let options = SearchOptions {
            min_score,
            limit,
            max_entries,
        };
        let result = py
            .detach(|| {
                run(self.pool.as_deref(), || {
                    if exhaustive {
                        self.index.search_exhaustive(&query, options)
                    } else {
                        self.index.search(&query, options)
                    }
                })
            })
            .map_err(|e| to_py_err(py, &e))?;
        let matches = PyList::empty(py);
        for found in &result.matches {
            matches.append(match_dict(
                py,
                &self.index,
                &query,
                0,
                found.slot,
                &found.comparison,
            )?)?;
        }
        self.report(
            py,
            &matches,
            result.total_matches,
            result.candidates,
            (result.truncated, result.approximate),
            1,
        )
    }
    fn find<'py>(
        &self,
        py: Python<'py>,
        min_score: f32,
        limit: usize,
        max_entries: usize,
        exhaustive: bool,
        budgets: (usize, usize),
    ) -> PyResult<Bound<'py, PyDict>> {
        let (max_pairs, max_semantic_comparisons) = budgets;
        let result = py
            .detach(|| {
                run(self.pool.as_deref(), || {
                    self.index.find(
                        SearchOptions {
                            min_score,
                            limit,
                            max_entries,
                        },
                        exhaustive,
                        max_pairs,
                        max_semantic_comparisons,
                    )
                })
            })
            .map_err(|e| to_py_err(py, &e))?;
        let matches = PyList::empty(py);
        for found in &result.matches {
            matches.append(match_dict(
                py,
                &self.index,
                &self.index.store().records()[found.left],
                found.left,
                found.right,
                &found.comparison,
            )?)?;
        }
        self.report(
            py,
            &matches,
            result.total_matches,
            result.candidates,
            (result.candidate_truncated, result.approximate),
            self.index.len(),
        )
    }
    #[allow(clippy::needless_pass_by_value)] // PyO3 extracts an owned path.
    fn save(
        &self,
        py: Python<'_>,
        path: std::path::PathBuf,
        configuration: &str,
        max_bytes: u64,
    ) -> PyResult<()> {
        if configuration.len() > 1 << 20 {
            return Err(to_py_err(
                py,
                &Error::snapshot_invalid("metadata is too large"),
            ));
        }
        let config = serde_json::from_str(configuration)
            .map_err(|_| to_py_err(py, &Error::snapshot_invalid("configuration structure")))?;
        py.detach(|| self.index.save(&path, self.generation, config, max_bytes))
            .map_err(|e| to_py_err(py, &e))
    }

    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        out.set_item("records", self.index.len())?;
        out.set_item("retained_slots", self.index.store().len())?;
        out.set_item("tombstones", self.index.store().len() - self.index.len())?;
        out.set_item("generation", self.generation)?;
        out.set_item("index_bytes", self.index.heap_bytes())?;
        out.set_item("ann_bytes", self.index.ann_bytes())?;
        out.set_item("ann_nodes", self.index.ann_nodes())?;
        Ok(out)
    }
}
impl NativeIndex {
    fn next_generation(&self, py: Python<'_>, changed: bool) -> PyResult<u64> {
        if !changed {
            return Ok(self.generation);
        }
        self.generation.checked_add(1).ok_or_else(|| {
            to_py_err(
                py,
                &Error::snapshot_invalid("index generation exhausted; rebuild the index"),
            )
        })
    }

    fn report<'py>(
        &self,
        py: Python<'py>,
        matches: &Bound<'py, PyList>,
        total: usize,
        candidates: usize,
        (truncated, approximate): (bool, bool),
        processed: usize,
    ) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        out.set_item("matches", matches)?;
        out.set_item("total_matches", total)?;
        out.set_item("candidates", candidates)?;
        out.set_item("candidate_truncated", truncated)?;
        out.set_item("approximate", approximate)?;
        out.set_item("output_truncated", total > matches.len())?;
        out.set_item("processed", processed)?;
        out.set_item("generation", self.generation)?;
        Ok(out)
    }
}

#[pyfunction]
pub(crate) fn validate_model(py: Python<'_>, model: &str) -> PyResult<()> {
    univo_embed::ModelSpec::parse(model)
        .map(drop)
        .map_err(|e| to_py_err(py, &e))
}
#[pyfunction]
pub(crate) fn invalid_option(py: Python<'_>, name: &str, expected: &str) -> PyErr {
    to_py_err(py, &Error::invalid_option(name, expected))
}

fn run<T: Send>(pool: Option<&rayon::ThreadPool>, work: impl FnOnce() -> T + Send) -> T {
    match pool {
        Some(pool) => pool.install(work),
        None => work(),
    }
}

/// Validate before any model resolution, using the same f32 range as search.
#[pyfunction]
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn validate_threshold(py: Python<'_>, min_score: f64) -> PyResult<f32> {
    let score = min_score as f32;
    if !(min_score.is_finite() && min_score > 0.0 && min_score <= 1.0 && score > 0.0) {
        return Err(to_py_err(py, &Error::invalid_threshold(score)));
    }
    Ok(score)
}

#[pyclass(name = "EmbeddingCache", module = "univo._univo")]
pub(crate) struct PyEmbeddingCache {
    cache: Arc<univo_engine::EmbeddingCache>,
}

#[pymethods]
impl PyEmbeddingCache {
    #[new]
    fn new(model: &Model, max_bytes: usize) -> Self {
        use univo_embed::Embedder;
        Self {
            cache: Arc::new(univo_engine::EmbeddingCache::new(
                model.embedder.descriptor().clone(),
                max_bytes,
            )),
        }
    }

    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let stats = self.cache.stats().map_err(|e| to_py_err(py, &e))?;
        let out = PyDict::new(py);
        for (key, value) in [
            ("embedding_cache_hits", stats.hits),
            ("embedding_cache_misses", stats.misses),
            ("embedding_cache_evictions", stats.evictions),
            ("embedding_cache_bytes", stats.bytes),
            ("embedding_cache_entries", stats.entries),
        ] {
            out.set_item(key, value)?;
        }
        Ok(out)
    }
}

/// Keeps an opened generation stable between configuration and data loading.
#[derive(Debug)]
#[pyclass(name = "Snapshot", module = "univo._univo")]
pub(crate) struct PySnapshot {
    snapshot: Option<univo_engine::snapshot::Snapshot>,
}

#[pymethods]
impl PySnapshot {
    #[new]
    #[allow(clippy::needless_pass_by_value)] // PyO3 extracts an owned path.
    fn new(py: Python<'_>, path: std::path::PathBuf, max_bytes: u64) -> PyResult<Self> {
        py.detach(|| univo_engine::snapshot::Snapshot::open(&path, max_bytes))
            .map(|snapshot| Self {
                snapshot: Some(snapshot),
            })
            .map_err(|e| to_py_err(py, &e))
    }
    #[getter]
    fn configuration(&self, py: Python<'_>) -> PyResult<String> {
        self.snapshot
            .as_ref()
            .map(|s| s.configuration().to_string())
            .ok_or_else(|| to_py_err(py, &Error::snapshot_invalid("snapshot already consumed")))
    }
    fn check_capacity(&self, py: Python<'_>, max_records: usize) -> PyResult<()> {
        self.snapshot
            .as_ref()
            .ok_or_else(|| to_py_err(py, &Error::snapshot_invalid("snapshot already consumed")))?
            .check_capacity(max_records)
            .map_err(|e| to_py_err(py, &e))
    }
    #[pyo3(signature = (schema, max_records, model=None, threads=1, cache=None, ann=None))]
    #[allow(clippy::too_many_arguments)] // PyO3 mirrors index construction.
    fn restore(
        &mut self,
        py: Python<'_>,
        schema: &PySchema,
        max_records: usize,
        model: Option<&Model>,
        threads: usize,
        cache: Option<&PyEmbeddingCache>,
        ann: Option<(usize, usize)>,
    ) -> PyResult<NativeIndex> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| to_py_err(py, &Error::snapshot_invalid("snapshot already consumed")))?;
        let generation = snapshot.generation();
        let mut index = NativeIndex::new(py, schema, max_records, model, threads, cache, ann)?;
        index.index = py
            .detach(|| snapshot.restore(index.index))
            .map_err(|e| to_py_err(py, &e))?;
        index.generation = generation;
        Ok(index)
    }
}

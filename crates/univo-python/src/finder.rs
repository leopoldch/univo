//! Schema and record conversion; comparison and retrieval are delegated to Rust.
use crate::{errors::to_py_err, models::Model};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use univo_core::{
    Comparison, Conflict, Error, Exact, ExactNormalizer, FieldKind, FieldPath, Limits, Map,
    PreparedRecord, RecordId, Schema, Text, TextMode, Value, exact_match, prepare,
};
use univo_engine::{Decision, GroupOptions, QueryEncoder, SearchIndex, SearchOptions};

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

/// The mutable state of an index: records, lanes and logical generation.
#[derive(Debug)]
struct IndexState {
    index: SearchIndex,
    generation: u64,
}

impl IndexState {
    fn next_generation(&self, changed: bool) -> univo_core::Result<u64> {
        if !changed {
            return Ok(self.generation);
        }
        self.generation
            .checked_add(1)
            .ok_or_else(|| Error::snapshot_invalid("index generation exhausted; rebuild the index"))
    }
}

/// An index shared by Python threads. Searches hold a shared lock and run
/// in parallel; changes hold the exclusive lock. Locks are only taken with
/// the interpreter released, and nothing holding one touches Python, so a
/// lock never waits on the interpreter.
#[derive(Debug)]
#[pyclass(frozen, module = "univo._univo", name = "Index")]
pub(crate) struct NativeIndex {
    /// The index's schema, to convert inputs without taking a lock.
    schema: Schema,
    state: RwLock<IndexState>,
    max_records: usize,
    pool: Option<Arc<LazyPool>>,
    /// The schema has semantic fields to encode.
    encodes: bool,
}

/// A record identifier as Python receives it.
#[derive(Debug, Clone)]
enum OutputId {
    Id(RecordId),
    Slot(usize),
}

impl OutputId {
    fn of(record: &PreparedRecord, slot: usize) -> Self {
        record.id().cloned().map_or(Self::Slot(slot), Self::Id)
    }

    fn to_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self {
            Self::Id(id) => external_id(py, id),
            Self::Slot(slot) => Ok(slot.into_pyobject(py)?.into_any().unbind()),
        }
    }
}

/// A match, built under the index lock and converted after it.
#[derive(Debug)]
struct MatchData {
    left: OutputId,
    right: OutputId,
    score: Option<f32>,
    coverage: f32,
    exact: bool,
    field_scores: Vec<(String, Option<f32>)>,
    reasons: Vec<String>,
    conflicts: Vec<String>,
}

#[derive(Debug, Default)]
struct ReportData {
    matches: Vec<MatchData>,
    total: usize,
    candidates: usize,
    truncated: bool,
    approximate: bool,
    processed: usize,
    generation: u64,
}

fn match_data(
    index: &SearchIndex,
    left: &PreparedRecord,
    left_slot: usize,
    right_slot: usize,
    comparison: &Comparison,
) -> univo_core::Result<MatchData> {
    let right = &index.store().records()[right_slot];
    let exact = exact_match(index.schema(), left, right)?;
    let mut field_scores = Vec::new();
    let mut reasons = Vec::new();
    let mut conflicts = Vec::new();
    for (field, score) in index.schema().fields().iter().zip(comparison.fields.iter()) {
        let path = field.path.to_string();
        reasons.push(format!("{path}: {:?}", score.signal));
        if score.contradiction {
            conflicts.push(path.clone());
        }
        field_scores.push((path, score.similarity));
    }
    if let Some(exact) = &exact {
        for (field, steps) in &exact.normalized_fields {
            reasons.push(format!(
                "{}: normalized ({})",
                index.schema().fields()[*field].path,
                steps.names().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    Ok(MatchData {
        left: OutputId::of(left, left_slot),
        right: OutputId::of(right, right_slot),
        score: comparison.score,
        coverage: comparison.coverage,
        exact: exact.is_some(),
        field_scores,
        reasons,
        conflicts,
    })
}

fn match_dict<'py>(py: Python<'py>, found: &MatchData) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("left_id", found.left.to_py(py)?)?;
    out.set_item("right_id", found.right.to_py(py)?)?;
    out.set_item("score", found.score)?;
    out.set_item("coverage", found.coverage)?;
    out.set_item("kind", if found.exact { "exact" } else { "candidate" })?;
    let fields = PyDict::new(py);
    for (path, similarity) in &found.field_scores {
        fields.set_item(path, similarity)?;
    }
    out.set_item("field_scores", fields)?;
    out.set_item("reasons", &found.reasons)?;
    out.set_item("conflicts", &found.conflicts)?;
    Ok(out)
}

fn report_dict<'py>(py: Python<'py>, report: &ReportData) -> PyResult<Bound<'py, PyDict>> {
    let matches = PyList::empty(py);
    for found in &report.matches {
        matches.append(match_dict(py, found)?)?;
    }
    let out = PyDict::new(py);
    out.set_item("matches", &matches)?;
    out.set_item("total_matches", report.total)?;
    out.set_item("candidates", report.candidates)?;
    out.set_item("candidate_truncated", report.truncated)?;
    out.set_item("approximate", report.approximate)?;
    out.set_item("output_truncated", report.total > report.matches.len())?;
    out.set_item("processed", report.processed)?;
    out.set_item("generation", report.generation)?;
    Ok(out)
}

/// The report of a search for `query`, built under the lock.
fn search_report(
    state: &IndexState,
    query: &PreparedRecord,
    result: &univo_engine::SearchResult,
) -> univo_core::Result<ReportData> {
    let matches = result
        .matches
        .iter()
        .map(|found| match_data(&state.index, query, 0, found.slot, &found.comparison))
        .collect::<univo_core::Result<_>>()?;
    Ok(ReportData {
        matches,
        total: result.total_matches,
        candidates: result.candidates,
        truncated: result.truncated,
        approximate: result.approximate,
        processed: 1,
        generation: state.generation,
    })
}

impl NativeIndex {
    fn read(&self) -> univo_core::Result<RwLockReadGuard<'_, IndexState>> {
        self.state.read().map_err(|_| Error::index_poisoned())
    }

    fn write(&self) -> univo_core::Result<RwLockWriteGuard<'_, IndexState>> {
        self.state.write().map_err(|_| Error::index_poisoned())
    }

    fn build(
        schema: &Schema,
        max_records: usize,
        model: Option<&Model>,
        pool: Option<&PyWorkerPool>,
        cache: Option<&PyEmbeddingCache>,
        ann: Option<(usize, usize)>,
    ) -> univo_core::Result<(SearchIndex, Option<Arc<LazyPool>>)> {
        // Encoding and all-pairs reports run on the Finder's pool, so
        // `threads` bounds them; single searches run on the calling thread.
        // Without a pool, reports run on the caller. Encoding always needs
        // one, or the tokenizer would use every core.
        let pool = match pool {
            Some(pool) => Some(Arc::clone(&pool.pool)),
            None if has_semantic(schema) => Some(Arc::new(LazyPool::new(1)?)),
            None => None,
        };
        let embedder = model.map(|m| Arc::clone(&m.embedder) as Arc<dyn univo_embed::Embedder>);
        let index = SearchIndex::new(schema.clone(), max_records, embedder)?;
        let index = match cache {
            Some(cache) => index.with_embedding_cache(Arc::clone(&cache.cache))?,
            None => index,
        };
        let index = match ann {
            Some((ef_search, max_candidates)) => index.with_ann(univo_engine::AnnOptions {
                ef_search,
                max_candidates,
            })?,
            None => index,
        };
        Ok((index, pool))
    }

    fn assemble(
        index: SearchIndex,
        generation: u64,
        max_records: usize,
        pool: Option<Arc<LazyPool>>,
    ) -> Self {
        Self {
            schema: index.schema().clone(),
            encodes: has_semantic(index.schema()),
            state: RwLock::new(IndexState { index, generation }),
            max_records,
            pool,
        }
    }

    /// The pool that encodes, started on first use, or `None` without
    /// semantic fields: nothing to encode, so lexical calls never start it.
    fn encode_pool(&self) -> univo_core::Result<Option<Arc<rayon::ThreadPool>>> {
        match &self.pool {
            Some(pool) if self.encodes => pool.get().map(Some),
            _ => Ok(None),
        }
    }

    /// The pool of an all-pairs report over `records`, started on first
    /// use, or `None` to run the report on the calling thread: one thread,
    /// or a report too small to pay for waking the workers.
    fn report_pool(&self, records: usize) -> univo_core::Result<Option<Arc<rayon::ThreadPool>>> {
        match &self.pool {
            Some(pool) if pool.threads > 1 && records >= PARALLEL_MIN_RECORDS => {
                pool.get().map(Some)
            }
            _ => Ok(None),
        }
    }

    /// The query encoder, taken under a brief shared lock: encoding then
    /// runs with no lock held.
    fn encoder(&self) -> univo_core::Result<QueryEncoder> {
        Ok(self.read()?.index.encoder())
    }
}

#[pymethods]
impl NativeIndex {
    #[new]
    #[pyo3(signature = (schema, max_records, model=None, pool=None, cache=None, ann=None))]
    fn new(
        py: Python<'_>,
        schema: &PySchema,
        max_records: usize,
        model: Option<&Model>,
        pool: Option<&PyWorkerPool>,
        cache: Option<&PyEmbeddingCache>,
        ann: Option<(usize, usize)>,
    ) -> PyResult<Self> {
        py.detach(|| Self::build(&schema.schema, max_records, model, pool, cache, ann))
            .map(|(index, pool)| Self::assemble(index, 0, max_records, pool))
            .map_err(|e| to_py_err(py, &e))
    }

    #[pyo3(signature = (inputs, semantic_budget=None))]
    fn add(
        &self,
        py: Python<'_>,
        inputs: &Bound<'_, PyAny>,
        semantic_budget: Option<usize>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let records = batch(py, &self.schema, inputs, self.max_records)?;
        let ids = py
            .detach(|| {
                // The budget is checked before any encoding work, then again
                // under the exclusive lock, in case records arrived between.
                if let Some(budget) = semantic_budget {
                    self.read()?.index.check_find_batch(&records, budget)?;
                }
                let encoder = self.encoder()?;
                let vectors = run(self.encode_pool()?.as_deref(), || {
                    encoder.encode_batch(&records)
                })?;
                let mut state = self.write()?;
                if let Some(budget) = semantic_budget {
                    state.index.check_find_batch(&records, budget)?;
                }
                let generation = state.next_generation(!records.is_empty())?;
                let slots = state.index.add_encoded(records, vectors)?;
                state.generation = generation;
                let store = state.index.store();
                Ok(slots
                    .map(|slot| OutputId::of(&store.records()[slot], slot))
                    .collect::<Vec<_>>())
            })
            .map_err(|e: Error| to_py_err(py, &e))?;
        ids.iter().map(|id| id.to_py(py)).collect()
    }

    fn upsert(&self, py: Python<'_>, inputs: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
        let records = batch(py, &self.schema, inputs, self.max_records)?;
        let ids = py
            .detach(|| {
                let mut guard = self.write()?;
                let state = &mut *guard;
                let generation = state.next_generation(!records.is_empty())?;
                let slots = run(self.encode_pool()?.as_deref(), || {
                    state.index.upsert(records)
                })?;
                state.generation = generation;
                let store = state.index.store();
                Ok(slots
                    .into_iter()
                    .map(|slot| OutputId::of(&store.records()[slot], slot))
                    .collect::<Vec<_>>())
            })
            .map_err(|e: Error| to_py_err(py, &e))?;
        ids.iter().map(|id| id.to_py(py)).collect()
    }

    fn remove(&self, py: Python<'_>, inputs: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
        let ids = ids(py, &self.schema, inputs, self.max_records)?;
        let missing = py
            .detach(|| {
                let mut state = self.write()?;
                let changed = ids.iter().any(|id| state.index.store().contains_id(id));
                let generation = state.next_generation(changed)?;
                let missing = state.index.remove(&ids)?;
                state.generation = generation;
                Ok(missing)
            })
            .map_err(|e: Error| to_py_err(py, &e))?;
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
        let query = record(&self.schema, input).map_err(|e| to_py_err(py, &e))?;
        let options = SearchOptions {
            min_score,
            limit,
            max_entries,
        };
        let report = py
            .detach(|| {
                let encoder = self.encoder()?;
                let vectors = run(self.encode_pool()?.as_deref(), || encoder.encode(&query))?;
                let state = self.read()?;
                let result =
                    state
                        .index
                        .search_encoded(&query, vectors.as_ref(), options, exhaustive)?;
                search_report(&state, &query, &result)
            })
            .map_err(|e| to_py_err(py, &e))?;
        report_dict(py, &report)
    }

    /// Searches for `input` and adds it if nothing reaches `min_score`, under
    /// one exclusive lock; the record is encoded before taking it.
    fn check_or_add<'py>(
        &self,
        py: Python<'py>,
        input: &Bound<'_, PyAny>,
        min_score: f32,
        limit: usize,
        max_entries: usize,
        exhaustive: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query = record(&self.schema, input).map_err(|e| to_py_err(py, &e))?;
        let options = SearchOptions {
            min_score,
            limit,
            max_entries,
        };
        let (decision, inserted, report) = py
            .detach(|| {
                let encoder = self.encoder()?;
                let vectors = run(self.encode_pool()?.as_deref(), || encoder.encode(&query))?;
                let mut state = self.write()?;
                let generation = state.next_generation(true)?;
                let outcome =
                    state
                        .index
                        .check_or_add(query.clone(), vectors, options, exhaustive)?;
                if outcome.slot.is_some() {
                    state.generation = generation;
                }
                let inserted = outcome
                    .slot
                    .map(|slot| OutputId::of(&state.index.store().records()[slot], slot));
                let report = search_report(&state, &query, &outcome.result)?;
                Ok((outcome.decision, inserted, report))
            })
            .map_err(|e: Error| to_py_err(py, &e))?;
        let out = PyDict::new(py);
        out.set_item(
            "decision",
            match decision {
                Decision::Inserted => "inserted",
                Decision::Matched => "matched",
                Decision::Review => "review",
            },
        )?;
        out.set_item("id", inserted.map(|id| id.to_py(py)).transpose()?)?;
        out.set_item("report", report_dict(py, &report)?)?;
        Ok(out)
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
        let report = py
            .detach(|| {
                let state = self.read()?;
                let result = run(self.report_pool(state.index.len())?.as_deref(), || {
                    state.index.find(
                        SearchOptions {
                            min_score,
                            limit,
                            max_entries,
                        },
                        exhaustive,
                        max_pairs,
                        max_semantic_comparisons,
                    )
                })?;
                let records = state.index.store().records();
                let matches = result
                    .matches
                    .iter()
                    .map(|found| {
                        match_data(
                            &state.index,
                            &records[found.left],
                            found.left,
                            found.right,
                            &found.comparison,
                        )
                    })
                    .collect::<univo_core::Result<_>>()?;
                Ok(ReportData {
                    matches,
                    total: result.total_matches,
                    candidates: result.candidates,
                    truncated: result.candidate_truncated,
                    approximate: result.approximate,
                    processed: state.index.len(),
                    generation: state.generation,
                })
            })
            .map_err(|e: Error| to_py_err(py, &e))?;
        report_dict(py, &report)
    }

    /// Exact and conservative groups of the indexed records.
    #[allow(clippy::too_many_arguments)] // PyO3 mirrors the Python options.
    fn groups<'py>(
        &self,
        py: Python<'py>,
        min_score: f32,
        max_entries: usize,
        exhaustive: bool,
        max_group_size: usize,
        max_pairs: usize,
        budgets: (usize, usize),
    ) -> PyResult<Bound<'py, PyDict>> {
        let options = GroupOptions {
            min_score,
            max_entries,
            exhaustive,
            max_group_size,
            max_pairs,
            max_exhaustive_pairs: budgets.0,
            max_semantic_comparisons: budgets.1,
        };
        let (report, ids, processed) = py
            .detach(|| {
                let state = self.read()?;
                let report = run(self.report_pool(state.index.len())?.as_deref(), || {
                    state.index.groups(options)
                })?;
                let store = state.index.store();
                let ids = |slots: &[usize]| -> Vec<OutputId> {
                    slots
                        .iter()
                        .map(|&slot| OutputId::of(&store.records()[slot], slot))
                        .collect()
                };
                let exact: Vec<Vec<OutputId>> =
                    report.exact.iter().map(|group| ids(group)).collect();
                let groups: Vec<(Vec<OutputId>, f32)> = report
                    .groups
                    .iter()
                    .map(|group| (ids(&group.members), group.min_score))
                    .collect();
                Ok((report, (exact, groups), state.index.len()))
            })
            .map_err(|e: Error| to_py_err(py, &e))?;
        let to_list = |members: &[OutputId]| -> PyResult<Vec<Py<PyAny>>> {
            members.iter().map(|id| id.to_py(py)).collect()
        };
        let exact = ids
            .0
            .iter()
            .map(|group| to_list(group))
            .collect::<PyResult<Vec<_>>>()?;
        let groups = PyList::empty(py);
        for (members, min_score) in &ids.1 {
            groups.append((to_list(members)?, *min_score))?;
        }
        let out = PyDict::new(py);
        out.set_item("exact", exact)?;
        out.set_item("groups", groups)?;
        out.set_item("processed", processed)?;
        out.set_item("qualifying_pairs", report.qualifying_pairs)?;
        out.set_item("split_pairs", report.split_pairs)?;
        out.set_item("candidates", report.candidates)?;
        out.set_item("candidate_truncated", report.truncated)?;
        out.set_item("approximate", report.approximate)?;
        Ok(out)
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
        py.detach(|| {
            let state = self.read()?;
            state.index.save(&path, state.generation, config, max_bytes)
        })
        .map_err(|e| to_py_err(py, &e))
    }

    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let counts = py
            .detach(|| {
                let state = self.read()?;
                let index = &state.index;
                Ok([
                    ("records", index.len() as u64),
                    ("retained_slots", index.store().len() as u64),
                    ("tombstones", (index.store().len() - index.len()) as u64),
                    ("generation", state.generation),
                    ("index_bytes", index.heap_bytes() as u64),
                    ("ann_bytes", index.ann_bytes() as u64),
                    ("ann_nodes", index.ann_nodes() as u64),
                ])
            })
            .map_err(|e: Error| to_py_err(py, &e))?;
        let out = PyDict::new(py);
        for (key, value) in counts {
            out.set_item(key, value)?;
        }
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

fn has_semantic(schema: &Schema) -> bool {
    schema
        .fields()
        .iter()
        .any(|f| matches!(f.kind, FieldKind::Text(t) if t.mode == TextMode::Semantic))
}

/// Records from which `find` and `group` run on the worker pool.
const PARALLEL_MIN_RECORDS: usize = 512;

/// Worker threads started on first use: a Finder that never encodes nor
/// runs a large report never starts any.
#[derive(Debug)]
pub(crate) struct LazyPool {
    threads: usize,
    pool: Mutex<Option<Arc<rayon::ThreadPool>>>,
}

impl LazyPool {
    fn new(threads: usize) -> univo_core::Result<Self> {
        if threads == 0 {
            return Err(Error::invalid_option("threads", "a positive integer"));
        }
        Ok(Self {
            threads,
            pool: Mutex::new(None),
        })
    }

    /// The workers, started by the first caller.
    fn get(&self) -> univo_core::Result<Arc<rayon::ThreadPool>> {
        // Building cannot leave the slot half-written: recover a poisoned lock.
        let mut slot = self.pool.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pool) = slot.as_ref() {
            return Ok(Arc::clone(pool));
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.threads)
            .build()
            .map(Arc::new)
            .map_err(|_| Error::invalid_option("threads", "an available worker pool"))?;
        *slot = Some(Arc::clone(&pool));
        Ok(pool)
    }

    fn started(&self) -> bool {
        self.pool
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

/// The worker threads of one Finder, shared by all its indexes: the
/// persistent one and the temporary ones of `find` and `group`. Its threads
/// start on first use and are created once, so small calls pay no thread
/// creation.
#[derive(Debug)]
#[pyclass(frozen, module = "univo._univo", name = "WorkerPool")]
pub(crate) struct PyWorkerPool {
    pool: Arc<LazyPool>,
}

#[pymethods]
impl PyWorkerPool {
    #[new]
    fn new(py: Python<'_>, threads: usize) -> PyResult<Self> {
        LazyPool::new(threads)
            .map(|pool| Self {
                pool: Arc::new(pool),
            })
            .map_err(|e| to_py_err(py, &e))
    }

    #[getter]
    fn threads(&self) -> usize {
        self.pool.threads
    }

    /// Whether the worker threads have started.
    #[getter]
    fn started(&self) -> bool {
        self.pool.started()
    }
}

/// Threads this process may run at once: the cores it is allowed, within
/// CPU affinity and, on Linux, container quotas. One when unknown.
#[pyfunction]
pub(crate) fn available_threads() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
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
    #[pyo3(signature = (schema, max_records, model=None, pool=None, cache=None, ann=None))]
    #[allow(clippy::too_many_arguments)] // PyO3 mirrors index construction.
    fn restore(
        &mut self,
        py: Python<'_>,
        schema: &PySchema,
        max_records: usize,
        model: Option<&Model>,
        pool: Option<&PyWorkerPool>,
        cache: Option<&PyEmbeddingCache>,
        ann: Option<(usize, usize)>,
    ) -> PyResult<NativeIndex> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| to_py_err(py, &Error::snapshot_invalid("snapshot already consumed")))?;
        let generation = snapshot.generation();
        py.detach(|| {
            let (index, pool) =
                NativeIndex::build(&schema.schema, max_records, model, pool, cache, ann)?;
            let index = snapshot.restore(index)?;
            Ok(NativeIndex::assemble(index, generation, max_records, pool))
        })
        .map_err(|e: Error| to_py_err(py, &e))
    }
}

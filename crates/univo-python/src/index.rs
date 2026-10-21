//! The native index behind a Finder: locking, generations and reports.
//! Comparison and retrieval are delegated to the Rust crates.

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use univo_core::{Comparison, Error, PreparedRecord, RecordId, Schema, exact_match};
use univo_engine::{Decision, GroupOptions, QueryEncoder, SearchIndex, SearchOptions};

use crate::cache::PyEmbeddingCache;
use crate::errors::to_py_err;
use crate::models::Model;
use crate::pool::{LazyPool, PARALLEL_MIN_RECORDS, PyWorkerPool, run};
use crate::schema::{PySchema, batch, external_id, has_semantic, ids, record};

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
        reasons.push(format!("{path}: {}", score.signal.name()));
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

    pub(crate) fn build(
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

    pub(crate) fn assemble(
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

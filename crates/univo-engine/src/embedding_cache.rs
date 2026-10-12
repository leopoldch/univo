//! Finder-scoped cache for normalized semantic texts. FIFO eviction, no disk
//! state and no Python references. Model identity is verified in full.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use univo_core::{Error, Result};
use univo_embed::{Embedder, ModelDescriptor};

type Row = Option<Arc<[f32]>>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub entries: usize,
    /// Conservative retained allocation estimate, not process RSS.
    pub bytes: usize,
    pub max_bytes: usize,
}

#[derive(Debug)]
struct Entry {
    row: Row,
    bytes: usize,
}

#[derive(Debug, Default)]
struct State {
    entries: HashMap<Arc<str>, Entry>,
    order: VecDeque<Arc<str>>,
    payload_bytes: usize,
    table_capacity: usize,
    hits: usize,
    misses: usize,
    evictions: usize,
}

impl State {
    fn bytes(&self) -> usize {
        // Track allocation high-water capacity: removals create tombstones
        // and can reduce HashMap::capacity() without freeing its allocation.
        // Two slots per allocation capacity conservatively cover hash-table
        // load factor, control bytes and alignment. Arc headers/rounding
        // are included in each entry's payload charge.
        self.payload_bytes
            + self.table_capacity * 2 * (size_of::<(Arc<str>, Entry)>() + 1)
            + self.order.capacity() * size_of::<Arc<str>>()
    }

    fn evict(&mut self) {
        if let Some(key) = self.order.pop_front()
            && let Some(entry) = self.entries.remove(&key)
        {
            self.payload_bytes -= entry.bytes;
            self.evictions = self.evictions.saturating_add(1);
        }
    }

    fn insert(&mut self, text: &str, row: Row, max_bytes: usize) {
        let bytes = text
            .len()
            .saturating_add(row.as_ref().map_or(0, |v| v.len() * 4))
            .saturating_add(64);
        if bytes > max_bytes || self.entries.contains_key(text) {
            return;
        }
        self.entries.reserve(1);
        self.table_capacity = self.table_capacity.max(self.entries.capacity());
        self.order.reserve(1);
        while !self.entries.is_empty() && self.bytes().saturating_add(bytes) > max_bytes {
            self.evict();
        }
        if self.bytes().saturating_add(bytes) > max_bytes {
            self.entries.shrink_to_fit();
            self.order.shrink_to_fit();
            self.entries.reserve(1);
            self.table_capacity = self.entries.capacity();
            self.order.reserve(1);
        }
        if self.bytes().saturating_add(bytes) > max_bytes {
            // Empty containers may still own allocations after an oversized
            // admission attempt; release those as well.
            if self.entries.is_empty() {
                self.entries = HashMap::new();
                self.table_capacity = 0;
                self.order = VecDeque::new();
            }
            return;
        }
        let key: Arc<str> = text.into();
        self.order.push_back(Arc::clone(&key));
        self.entries.insert(key, Entry { row, bytes });
        self.payload_bytes += bytes;
    }
}

/// Cache identity is the complete model descriptor (including tokenizer and
/// encoding configuration digests). Its only callers encode semantic fields
/// after the current normalizer. Weights, IDs and field paths do not affect
/// inference and deliberately do not partition this cache.
#[derive(Debug)]
pub struct EmbeddingCache {
    model: ModelDescriptor,
    max_bytes: usize,
    state: Mutex<State>,
}

impl EmbeddingCache {
    pub fn new(model: ModelDescriptor, max_bytes: usize) -> Self {
        Self {
            model,
            max_bytes,
            state: Mutex::new(State::default()),
        }
    }

    pub fn model(&self) -> &ModelDescriptor {
        &self.model
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| {
            Error::corrupt_model(&self.model.name(), "the embedding cache lock is poisoned")
        })
    }

    /// # Errors
    /// Returns a typed error if the cache lock is poisoned.
    pub fn stats(&self) -> Result<CacheStats> {
        let state = self.state()?;
        Ok(CacheStats {
            hits: state.hits,
            misses: state.misses,
            evictions: state.evictions,
            entries: state.entries.len(),
            bytes: state.bytes(),
            max_bytes: self.max_bytes,
        })
    }

    pub(crate) fn check_model(&self, model: &ModelDescriptor) -> Result<()> {
        if model != &self.model {
            return Err(Error::model_mismatch(&self.model.name(), &model.name()));
        }
        Ok(())
    }

    /// Distinct, normalized semantic texts of one field. Missing model tokens
    /// are cached too. Inference runs outside the lock: concurrent cold
    /// requests may duplicate work, but cache publication is synchronized.
    pub(crate) fn encode(&self, texts: &[&str], embedder: &dyn Embedder) -> Result<Vec<Row>> {
        self.check_model(embedder.descriptor())?;
        let mut rows = vec![None; texts.len()];
        let mut missing = Vec::new();
        let mut positions = Vec::new();
        {
            let mut state = self.state()?;
            for (position, text) in texts.iter().enumerate() {
                if let Some(entry) = state.entries.get(*text) {
                    rows[position].clone_from(&entry.row);
                    state.hits = state.hits.saturating_add(1);
                } else {
                    missing.push(*text);
                    positions.push(position);
                    state.misses = state.misses.saturating_add(1);
                }
            }
        }
        if missing.is_empty() {
            return Ok(rows);
        }
        let encoded = embedder.encode_batch(&missing)?;
        self.check_model(embedder.descriptor())?;
        if encoded.len() != missing.len() || encoded.dimension() != self.model.dimension {
            return Err(Error::corrupt_model(
                &self.model.name(),
                "the encoder returned an incompatible cache batch",
            ));
        }
        let mut state = self.state()?;
        for (index, (text, position)) in missing.iter().zip(positions).enumerate() {
            let row = if encoded.absent().binary_search(&index).is_ok() {
                None
            } else {
                Some(Arc::from(encoded.row(index)))
            };
            state.insert(text, row.clone(), self.max_bytes);
            rows[position] = row;
        }
        Ok(rows)
    }
}

//! Local exact indexing with verified equality and bounded search output.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use univo_core::{
    Error, ExactMatch, PreparedRecord, RecordId, Result, Schema, exact_fingerprint, exact_match,
};

/// An append-only exact index. Slots identify records without external IDs.
#[derive(Debug)]
pub struct ExactIndex {
    schema: Schema,
    max_records: usize,
    records: Vec<PreparedRecord>,
    ids: HashSet<RecordId>,
    buckets: HashMap<u64, Vec<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactHit {
    pub slot: usize,
    pub evidence: ExactMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactSearch {
    pub matches: Vec<ExactHit>,
    pub total_matches: usize,
    /// Bucket entries visited, including a query's excluded ID.
    pub candidates_examined: usize,
}

impl ExactSearch {
    pub fn truncated(&self) -> bool {
        self.total_matches > self.matches.len()
    }
}

impl ExactIndex {
    pub fn new(schema: Schema, max_records: usize) -> Self {
        Self {
            schema,
            max_records,
            records: Vec::new(),
            ids: HashSet::new(),
            buckets: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub fn record(&self, slot: usize) -> Option<&PreparedRecord> {
        self.records.get(slot)
    }

    /// Validates the whole batch before inserting it; returns its slots.
    ///
    /// # Errors
    /// Rejects incompatible schemas, repeated IDs and batches exceeding capacity.
    pub fn add(&mut self, records: Vec<PreparedRecord>) -> Result<Range<usize>> {
        let start = self.len();
        let end = start.saturating_add(records.len());
        if records.len() > self.max_records.saturating_sub(start) {
            return Err(Error::index_capacity(end, self.max_records));
        }
        let mut ids = HashSet::new();
        let mut fingerprints = Vec::with_capacity(records.len());
        for (index, record) in records.iter().enumerate() {
            let locate = |error: Error| error.with_record(index, record.id().cloned());
            fingerprints.push(exact_fingerprint(&self.schema, record).map_err(locate)?);
            if let Some(id) = record.id() {
                if self.ids.contains(id) || !ids.insert(id) {
                    return Err(locate(Error::duplicate_id()));
                }
            }
        }
        let touched: HashSet<_> = fingerprints.iter().copied().collect();
        for (record, fingerprint) in records.into_iter().zip(fingerprints) {
            let slot = self.len();
            if let Some(id) = record.id() {
                self.ids.insert(id.clone());
            }
            self.buckets.entry(fingerprint).or_default().push(slot);
            self.records.push(record);
        }
        for fingerprint in touched {
            if let Some(bucket) = self.buckets.get_mut(&fingerprint) {
                bucket.sort_unstable_by_key(|&slot| (self.records[slot].id(), slot));
            }
        }
        Ok(start..end)
    }

    /// Verifies every record in the fingerprint bucket, including collisions.
    /// Output is ordered by typed ID, or slot when IDs are absent.
    ///
    /// # Errors
    /// Rejects a query prepared with an incompatible schema.
    pub fn search(&self, query: &PreparedRecord, limit: usize) -> Result<ExactSearch> {
        let fingerprint = exact_fingerprint(&self.schema, query)?;
        let mut matches = Vec::new();
        let mut total_matches = 0;
        let candidates = self
            .buckets
            .get(&fingerprint)
            .map_or(&[][..], Vec::as_slice);
        for &slot in candidates {
            let record = &self.records[slot];
            if query.id().is_some() && query.id() == record.id() {
                continue;
            }
            let Some(evidence) = exact_match(&self.schema, query, record)? else {
                continue;
            };
            total_matches += 1;
            if matches.len() < limit {
                matches.push(ExactHit { slot, evidence });
            }
        }
        Ok(ExactSearch {
            matches,
            total_matches,
            candidates_examined: candidates.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use univo_core::{FieldPath, Map, Text, Value, prepare};

    fn object(tenant: &str, name: &str) -> Value {
        Value::Map(
            Map::new(vec![
                ("tenant".into(), Value::Str(tenant.into())),
                ("name".into(), Value::Str(name.into())),
            ])
            .unwrap(),
        )
    }

    #[test]
    fn collisions_never_bypass_content_or_partitions() {
        let schema = Schema::builder()
            .partition_by(FieldPath::parse("tenant").unwrap())
            .text(FieldPath::parse("name").unwrap(), Text::default())
            .build()
            .unwrap();
        let query = prepare(&schema, &object("a", "same")).unwrap();
        let records = [("a", "same"), ("a", "different"), ("b", "same")]
            .iter()
            .map(|(tenant, name)| prepare(&schema, &object(tenant, name)).unwrap())
            .collect();
        let fingerprint = exact_fingerprint(&schema, &query).unwrap();
        let mut index = ExactIndex::new(schema, 3);
        index.add(records).unwrap();
        index.buckets.clear();
        index.buckets.insert(fingerprint, vec![0, 1, 2]);
        let result = index.search(&query, 3).unwrap();
        assert_eq!(result.total_matches, 1);
        assert_eq!(result.candidates_examined, 3);
        assert_eq!(result.matches[0].slot, 0);
    }
}

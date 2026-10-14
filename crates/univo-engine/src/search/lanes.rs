//! The candidate lanes of a search, one per field, and their union.

use univo_core::{ACRONYM_SIMILARITY, Error, Result};

use super::{Query, SearchIndex, SearchOptions, SearchResult, check_options, lane_threshold};
use crate::dense::extend_within;

impl SearchIndex {
    /// The distinct slots the lanes propose for `query`, recording their
    /// counts, truncation and approximation in `result`.
    pub(super) fn candidates(
        &self,
        query: &Query<'_>,
        options: SearchOptions,
        result: &mut SearchResult,
    ) -> Result<Vec<usize>> {
        check_options(options)?;
        if !query.record.is_prepared_with(self.schema()) {
            return Err(Error::schema_mismatch());
        }
        let threshold = lane_threshold(options.min_score);
        let mut candidates: Vec<usize> = Vec::new();

        let exact = self.exact.matching_slots(query.record)?;
        result.lanes.exact = exact.len();
        candidates.extend(exact);

        for index in &self.lexical {
            let found = index.search_from(
                self.store(),
                query.record,
                threshold,
                options.max_entries,
                query.min_slot,
            )?;
            result.truncated |= found.truncated;
            result.lanes.lexical += found.matches.len();
            candidates.extend(found.matches.iter().map(|found| found.slot));
        }

        if let Some(vectors) = &self.vectors {
            let partition = self.partition(query.record);
            for (position, &field) in self.semantic.iter().enumerate() {
                let (Some(field_vectors), Some(query_vector)) = (
                    vectors.field(field),
                    query.vectors.get(field).copied().flatten(),
                ) else {
                    continue;
                };
                if let Some(options) = self.ann_options
                    && let Some((slots, saturated)) = self.ann.candidates(
                        self.store(),
                        (query.record, query_vector),
                        field,
                        threshold,
                        options,
                        (query.slot, query.min_slot),
                    )
                {
                    result.approximate = true;
                    result.truncated |= saturated;
                    result.lanes.semantic += slots.len();
                    candidates.extend(slots);
                    continue;
                }
                if let Some(dense) = query.dense {
                    result.lanes.semantic += dense[position].len();
                    candidates.extend(dense[position].iter().map(|&slot| slot as usize));
                    continue;
                }
                result.lanes.semantic += extend_within(
                    field_vectors,
                    query_vector,
                    // Partitions list slots in increasing order: all-pairs
                    // queries scan only the records after them.
                    partition
                        [partition.partition_point(|&slot| (slot as usize) < query.min_slot)..]
                        .iter()
                        .map(|&slot| slot as usize)
                        .filter(|&slot| self.store().is_active(slot)),
                    threshold,
                    &mut candidates,
                );
            }
        }

        for index in &self.values {
            let mut read = 0usize;
            'lists: for slots in index.lists(self.store(), query.record) {
                for &slot in slots {
                    if read == options.max_entries {
                        result.truncated = true;
                        break 'lists;
                    }
                    read += 1;
                    result.lanes.values += 1;
                    candidates.push(slot as usize);
                }
            }
        }

        if threshold <= ACRONYM_SIMILARITY {
            for index in &self.acronyms {
                let mut read = 0usize;
                'lists: for slots in index.lists(self.store(), query.record) {
                    for &slot in slots {
                        if read == options.max_entries {
                            result.truncated = true;
                            break 'lists;
                        }
                        read += 1;
                        result.lanes.acronyms += 1;
                        candidates.push(slot as usize);
                    }
                }
            }
        }

        candidates.sort_unstable();
        candidates.dedup();
        Ok(candidates)
    }
}

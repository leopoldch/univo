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
            let Some(lane) = query.lane(index.field(), threshold) else {
                continue;
            };
            let found = index.search_from(
                self.store(),
                query.record,
                lane,
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
                let (Some(field_vectors), Some(query_vector), Some(lane)) = (
                    vectors.field(field),
                    query.vectors.get(field).copied().flatten(),
                    query.lane(field, threshold),
                ) else {
                    continue;
                };
                if let Some(options) = self.ann_options
                    && let Some((slots, saturated)) = self.ann.candidates(
                        self.store(),
                        (query.record, query_vector),
                        field,
                        lane,
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
                    // Found at the search's threshold: at least what the
                    // field's own would find.
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
                    lane,
                    &mut candidates,
                );
            }
        }

        for index in &self.values {
            if query.lane(index.field(), threshold).is_some() {
                let lists = index.lists(self.store(), query.record);
                let (read, stopped) = read_lists(&lists, options.max_entries, &mut candidates);
                result.lanes.values += read;
                result.truncated |= stopped;
            }
        }

        for index in &self.acronyms {
            if query
                .lane(index.field(), threshold)
                .is_some_and(|lane| lane <= ACRONYM_SIMILARITY)
            {
                let lists = index.lists(self.store(), query.record);
                let (read, stopped) = read_lists(&lists, options.max_entries, &mut candidates);
                result.lanes.acronyms += read;
                result.truncated |= stopped;
            }
        }

        candidates.sort_unstable();
        candidates.dedup();
        Ok(candidates)
    }
}

/// Adds the slots of posting `lists` to `candidates`, `max_entries` at
/// most. Returns the slots read and whether the limit stopped the reading.
fn read_lists(lists: &[&[u32]], max_entries: usize, candidates: &mut Vec<usize>) -> (usize, bool) {
    let mut read = 0usize;
    for slots in lists {
        for &slot in *slots {
            if read == max_entries {
                return (read, true);
            }
            read += 1;
            candidates.push(slot as usize);
        }
    }
    (read, false)
}

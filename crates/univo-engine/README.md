# univo-engine

Local indexing and search for [Univo](https://github.com/leopoldch/univo):

- exact, lexical, semantic, value and acronym candidate lanes, whose union is complete for the scoring rule and checked against an exhaustive oracle;
- optional partitioned HNSW retrieval;
- parallel all-pairs reports and conservative groups;
- mutations by ID;
- versioned snapshots.

See [search](https://github.com/leopoldch/univo/blob/main/docs/design/search.md). This crate is pre-alpha and not published yet.

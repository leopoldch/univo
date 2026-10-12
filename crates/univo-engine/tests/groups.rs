//! Exact and conservative groups: sound by construction, never a transitive
//! closure, checked against pairwise scoring.

use univo_core::{
    Conflict, ErrorCode, Exact, FieldPath, Map, PreparedRecord, Schema, Text, Value, compare,
    is_exact_match, prepare,
};
use univo_engine::{GroupOptions, GroupReport, SearchIndex};

fn path(dotted: &str) -> FieldPath {
    FieldPath::parse(dotted).unwrap()
}

fn schema() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant"))
        .text(path("name"), Text::default())
        .exact(
            path("kind"),
            Exact {
                conflict: Conflict::Reject,
                ..Exact::default()
            },
        )
        .build()
        .unwrap()
}

fn record(schema: &Schema, id: i64, tenant: &str, name: &str, kind: &str) -> PreparedRecord {
    let entries = vec![
        ("id".to_owned(), Value::Int(id)),
        ("tenant".to_owned(), Value::Str(tenant.into())),
        ("name".to_owned(), Value::Str(name.into())),
        ("kind".to_owned(), Value::Str(kind.into())),
    ];
    prepare(schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
}

fn options(min_score: f32) -> GroupOptions {
    GroupOptions {
        min_score,
        max_entries: usize::MAX,
        exhaustive: false,
        max_group_size: 100,
        max_pairs: 1_000_000,
        max_exhaustive_pairs: usize::MAX,
        max_semantic_comparisons: usize::MAX,
    }
}

fn indexed(names: &[(&str, &str, &str)]) -> SearchIndex {
    let schema = schema();
    let mut index = SearchIndex::new(schema.clone(), 10_000, None).unwrap();
    let records = names
        .iter()
        .enumerate()
        .map(|(id, (tenant, name, kind))| {
            record(&schema, i64::try_from(id).unwrap(), tenant, name, kind)
        })
        .collect();
    index.add(records).unwrap();
    index
}

fn members(report: &GroupReport) -> Vec<Vec<usize>> {
    report
        .groups
        .iter()
        .map(|group| group.members.clone())
        .collect()
}

#[test]
fn equal_records_form_exact_groups_within_partitions() {
    let index = indexed(&[
        ("t1", "Acme", "a"),
        ("t1", "ACME", "a"),
        ("t2", "Acme", "a"),
        ("t1", "acme", "a"),
        ("t1", "Globex", "a"),
    ]);
    let report = index.groups(options(0.8)).unwrap();
    assert_eq!(report.exact, [vec![0, 1, 3]]);
    assert_eq!(members(&report), [vec![0, 1, 3]]);
    assert_eq!(report.groups[0].min_score, 1.0);
}

#[test]
fn a_chain_is_not_closed_transitively() {
    // With the equal kind as evidence, A-B and B-C score 0.94 and A-C
    // 0.875: at 0.9, A and C are both close to B, not to each other.
    let index = indexed(&[
        ("t1", "abcdefgh", "a"),
        ("t1", "abcdefgx", "a"),
        ("t1", "abcdefxx", "a"),
    ]);
    let report = index.groups(options(0.9)).unwrap();
    assert_eq!(report.qualifying_pairs, 2);
    assert_eq!(members(&report), [vec![0, 1]]);
    assert_eq!(report.split_pairs, 1);
    assert!(
        report
            .groups
            .iter()
            .all(|group| !(group.members.contains(&0) && group.members.contains(&2)))
    );
}

#[test]
fn cliques_group_whole_and_the_size_limit_splits_them() {
    let names = [
        ("t1", "Acme Corporation", "a"),
        ("t1", "Acme Corporatio", "a"),
        ("t1", "Acme Corporaton", "a"),
        ("t1", "Acme Corpration", "a"),
    ];
    let index = indexed(&names);
    let report = index.groups(options(0.8)).unwrap();
    assert_eq!(members(&report), [vec![0, 1, 2, 3]]);
    assert_eq!(report.split_pairs, 0);
    assert!(report.groups[0].min_score >= 0.8);

    let limited = index
        .groups(GroupOptions {
            max_group_size: 2,
            ..options(0.8)
        })
        .unwrap();
    assert!(limited.groups.iter().all(|group| group.members.len() == 2));
    assert_eq!(limited.groups.len(), 2);
    assert_eq!(limited.split_pairs, 4);
}

#[test]
fn vetoed_pairs_never_group() {
    let index = indexed(&[("t1", "Acme", "prod"), ("t1", "Acme", "test")]);
    let report = index.groups(options(0.5)).unwrap();
    assert_eq!(report.exact, Vec::<Vec<usize>>::new());
    assert_eq!(report.groups, []);
    assert_eq!(report.qualifying_pairs, 0);
}

#[test]
fn an_exact_group_joins_as_one_node() {
    let index = indexed(&[
        ("t1", "Acme Corporation", "a"),
        ("t1", "Acme Corporation", "a"),
        ("t1", "Acme Corporation", "a"),
        ("t1", "Acme Corporatoin", "a"),
    ]);
    let report = index.groups(options(0.8)).unwrap();
    assert_eq!(report.exact, [vec![0, 1, 2]]);
    assert_eq!(members(&report), [vec![0, 1, 2, 3]]);
    assert_eq!(report.qualifying_pairs, 1);
}

#[test]
fn limits_and_thresholds_are_enforced() {
    let index = indexed(&[
        ("t1", "Acme Corporation", "a"),
        ("t1", "Acme Corporatio", "a"),
        ("t1", "Acme Corporaton", "a"),
    ]);
    let error = index
        .groups(GroupOptions {
            max_pairs: 1,
            ..options(0.8)
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::GroupPairLimit);
    for min_score in [0.0, 1.5, f32::NAN] {
        let error = index.groups(options(min_score)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidThreshold);
    }
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % bound as u64).unwrap()
    }
}

const BASES: &[&str] = &[
    "acme corporation",
    "globex industries",
    "initech",
    "umbrella",
    "société générale",
    "zz",
];

fn random_index(seed: u64) -> (SearchIndex, Vec<PreparedRecord>) {
    let schema = schema();
    let mut rng = Rng(seed);
    let records: Vec<PreparedRecord> = (0..300)
        .map(|id| {
            let mut name: Vec<char> = BASES[rng.below(BASES.len())].chars().collect();
            for _ in 0..rng.below(3) {
                let position = rng.below(name.len());
                name[position] = ['a', 'e', 'x'][rng.below(3)];
            }
            let name: String = name.into_iter().collect();
            let tenant = ["t1", "t2"][rng.below(2)];
            let kind = ["a", "a", "a", "b"][rng.below(4)];
            record(&schema, id, tenant, &name, kind)
        })
        .collect();
    let mut index = SearchIndex::new(schema, 10_000, None).unwrap();
    index.add(records.clone()).unwrap();
    (index, records)
}

#[test]
fn groups_satisfy_their_contract_on_random_data() {
    let schema = schema();
    for seed in [3, 17, 99] {
        let (index, records) = random_index(seed);
        for min_score in [0.6, 0.8, 0.95] {
            let report = index.groups(options(min_score)).unwrap();
            assert!(!report.truncated);
            let mut seen = std::collections::HashSet::new();
            for group in &report.groups {
                assert!(group.members.len() >= 2 && group.members.len() <= 100);
                assert!(group.members.windows(2).all(|pair| pair[0] < pair[1]));
                for (index_a, &a) in group.members.iter().enumerate() {
                    assert!(seen.insert(a), "groups overlap at {a}");
                    for &b in &group.members[index_a + 1..] {
                        let comparison =
                            compare(&schema, &records[a], &records[b]).unwrap().unwrap();
                        assert!(!comparison.vetoed, "{a} {b}");
                        let score = comparison.score.unwrap();
                        assert!(score >= min_score, "{a} {b} {score}");
                        assert!(score >= group.min_score);
                    }
                }
            }
            for group in &report.exact {
                for &member in &group[1..] {
                    assert!(is_exact_match(&schema, &records[group[0]], &records[member]).unwrap());
                }
                for (slot, record) in records.iter().enumerate() {
                    if !group.contains(&slot) {
                        assert!(!is_exact_match(&schema, &records[group[0]], record).unwrap());
                    }
                }
            }
            assert_eq!(index.groups(options(min_score)).unwrap(), report);
            let exhaustive = index
                .groups(GroupOptions {
                    exhaustive: true,
                    ..options(min_score)
                })
                .unwrap();
            assert_eq!(exhaustive.groups, report.groups);
            assert_eq!(exhaustive.qualifying_pairs, report.qualifying_pairs);
        }
    }
}

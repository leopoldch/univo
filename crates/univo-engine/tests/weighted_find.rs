//! Reports by learned weights: what they find on records with planted
//! duplicates, and what must hold whatever was learned: the same pairs with
//! or without leaving lanes out, on any number of threads, within vetoes,
//! partitions and removals.

use std::collections::HashSet;

use univo_core::{
    Conflict, Exact, FieldPath, Map, PreparedRecord, RecordId, Schema, Text, Value, prepare,
};
use univo_engine::{SearchIndex, WeightedOptions, WeightedReport};

struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % bound as u64).unwrap()
    }

    fn word(&mut self, syllables: usize) -> String {
        const SYLLABLES: &[&str] = &[
            "ka", "lo", "mi", "ne", "ra", "to", "vi", "su", "dor", "pel", "ban", "tri", "gon",
            "zu", "fa", "re", "qui", "mo", "sel", "an",
        ];
        (0..syllables)
            .map(|_| SYLLABLES[self.below(SYLLABLES.len())])
            .collect()
    }
}

fn path(dotted: &str) -> FieldPath {
    FieldPath::parse(dotted).unwrap()
}

/// A name, a birth date, a city from a short list, a trade from a shorter
/// one and a sex that never differs inside a person.
fn schema() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("country"))
        .text(path("name"), Text::default())
        .text(path("birth"), Text::default())
        .text(path("city"), Text::default())
        .exact(path("trade"), Exact::default())
        .exact(
            path("sex"),
            Exact {
                conflict: Conflict::Reject,
                ..Exact::default()
            },
        )
        .build()
        .unwrap()
}

/// `persons` persons in two countries, each with one to four records: a
/// typo in the name or the date now and then, values missing, the city or
/// the trade sometimes another. Returns the records and the person of each.
fn records(schema: &Schema, persons: usize, seed: u64) -> (Vec<PreparedRecord>, Vec<usize>) {
    const CITIES: &[&str] = &["lyon", "paris", "lille", "nantes", "brest", "nice", "metz"];
    const TRADES: &[&str] = &["baker", "smith", "clerk"];
    let mut rng = Rng(seed);
    let (mut prepared, mut person_of) = (Vec::new(), Vec::new());
    for person in 0..persons {
        let name = format!("{} {}", rng.word(2), rng.word(3));
        let birth = format!(
            "{}-{:02}-{:02}",
            1800 + rng.below(150),
            1 + rng.below(12),
            1 + rng.below(28)
        );
        let (city, trade) = (CITIES[rng.below(CITIES.len())], TRADES[rng.below(3)]);
        let (country, sex) = (["fr", "be"][person % 2], ["f", "m"][rng.below(2)]);
        for _ in 0..=rng.below(3) {
            let typo = |text: &str, rng: &mut Rng| {
                let mut letters: Vec<char> = text.chars().collect();
                if rng.below(3) == 0 {
                    let at = rng.below(letters.len());
                    letters[at] = ['x', '7', 'e'][rng.below(3)];
                }
                letters.into_iter().collect::<String>()
            };
            let mut entries = vec![
                (
                    "id".to_owned(),
                    Value::Int(i64::try_from(prepared.len()).unwrap()),
                ),
                ("country".to_owned(), Value::Str(country.into())),
                ("sex".to_owned(), Value::Str(sex.into())),
                ("name".to_owned(), Value::Str(typo(&name, &mut rng))),
            ];
            if rng.below(5) != 0 {
                entries.push(("birth".to_owned(), Value::Str(typo(&birth, &mut rng))));
            }
            if rng.below(4) != 0 {
                let city = if rng.below(8) == 0 {
                    CITIES[rng.below(CITIES.len())]
                } else {
                    city
                };
                entries.push(("city".to_owned(), Value::Str(city.into())));
            }
            if rng.below(4) != 0 {
                entries.push(("trade".to_owned(), Value::Str(trade.into())));
            }
            prepared.push(prepare(schema, &Value::Map(Map::new(entries).unwrap())).unwrap());
            person_of.push(person);
        }
    }
    (prepared, person_of)
}

fn index(persons: usize, seed: u64) -> (SearchIndex, Vec<usize>) {
    let schema = schema();
    let (prepared, person_of) = records(&schema, persons, seed);
    let mut index = SearchIndex::new(schema, 100_000, None).unwrap();
    index.add(prepared).unwrap();
    (index, person_of)
}

fn options(min_probability: f64) -> WeightedOptions {
    WeightedOptions {
        min_probability,
        limit: usize::MAX,
        max_entries: usize::MAX,
        every_lane: false,
    }
}

fn find(index: &SearchIndex, options: WeightedOptions) -> WeightedReport {
    index
        .find_weighted(options, false, usize::MAX, usize::MAX)
        .unwrap()
}

fn on<T: Send>(threads: usize, work: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(work)
}

#[test]
fn planted_duplicates_are_found_and_strangers_are_not() {
    let (index, person_of) = index(1500, 3);
    let report = on(4, || find(&index, options(0.9)));
    let truth: usize = {
        let mut sizes = vec![0usize; 1500];
        for &person in &person_of {
            sizes[person] += 1;
        }
        sizes.iter().map(|size| size * (size - 1) / 2).sum()
    };
    let correct = report
        .matches
        .iter()
        .filter(|found| person_of[found.left] == person_of[found.right])
        .count();
    assert_eq!(report.total_matches, report.matches.len());
    assert!(
        correct * 100 >= report.matches.len() * 97,
        "{correct} of {}",
        report.matches.len()
    );
    // A value in four is missing and one in three has a typo: not every
    // pair can be told from a stranger.
    assert!(correct * 100 >= truth * 65, "{correct} of {truth} pairs");
    // What was learned reads as expected: agreement adds, a difference
    // removes, and a city or a trade shared by many says less than a name.
    let bits: Vec<[f64; 4]> = report
        .weights
        .fields
        .iter()
        .map(|field| field.bits)
        .collect();
    for (field, learned) in bits.iter().take(4).enumerate() {
        assert!(learned[0] > 0.0 && learned[3] < 0.0, "{field}: {learned:?}");
    }
    assert!(bits[0][0] > 5.0 && bits[1][0] > 5.0, "{bits:?}");
    assert!(
        bits[2][0] < bits[0][0] && bits[3][0] < bits[2][0],
        "{bits:?}"
    );
    assert!(report.weights.prior < -5.0);
}

#[test]
fn leaving_lanes_out_loses_no_pair() {
    let (index, _) = index(700, 11);
    let mut saved = 0;
    for min_probability in [0.01, 0.3, 0.9, 0.999, 1.0] {
        let report = on(4, || find(&index, options(min_probability)));
        let every = on(4, || {
            find(
                &index,
                WeightedOptions {
                    every_lane: true,
                    ..options(min_probability)
                },
            )
        });
        assert_eq!(report.matches, every.matches, "{min_probability}");
        assert_eq!(report.total_matches, every.total_matches);
        assert_eq!(report.weights, every.weights);
        assert!(report.candidates <= every.candidates);
        saved += every.candidates - report.candidates;
    }
    assert!(saved > 50_000, "lanes must also be left out: {saved}");
}

#[test]
fn the_report_does_not_depend_on_the_threads() {
    let (index, _) = index(900, 5);
    let alone = find(&index, options(0.5));
    assert!(alone.total_matches > 300, "{}", alone.total_matches);
    for threads in [1, 3, 8] {
        let report = on(threads, || find(&index, options(0.5)));
        assert_eq!(report, alone, "{threads}");
    }
}

#[test]
fn matches_are_the_heaviest_within_the_limit() {
    let (index, _) = index(600, 9);
    let all = find(&index, options(0.2));
    assert!(all.matches.len() > 200);
    for found in &all.matches {
        let bits: f64 = found.bits.iter().flatten().sum();
        assert!((bits - found.weight).abs() < 1e-9);
        assert!((0.2..=1.0).contains(&found.probability));
        assert_eq!(found.probability, all.weights.probability(found.weight));
        assert!(found.left < found.right);
    }
    assert!(
        all.matches
            .windows(2)
            .all(|pair| pair[0].weight >= pair[1].weight)
    );
    for limit in [0, 1, 50] {
        let limited = on(4, || {
            find(
                &index,
                WeightedOptions {
                    limit,
                    ..options(0.2)
                },
            )
        });
        assert_eq!(limited.matches, all.matches[..limit]);
        assert_eq!(limited.total_matches, all.total_matches);
    }
}

#[test]
fn vetoes_partitions_and_removals_hold() {
    let (mut index, person_of) = index(800, 21);
    let removed: Vec<RecordId> = (0..i64::try_from(person_of.len()).unwrap())
        .step_by(7)
        .map(RecordId::Int)
        .collect();
    index.remove(&removed).unwrap();
    let records = index.store().records();
    let report = on(4, || find(&index, options(0.01)));
    assert!(report.total_matches > 200);
    let mut seen = HashSet::new();
    for found in &report.matches {
        let (left, right) = (&records[found.left], &records[found.right]);
        assert!(index.store().is_active(found.left) && index.store().is_active(found.right));
        assert_eq!(left.partition(), right.partition());
        // The sex is the last field: a difference is a veto.
        assert!(!found.comparison.fields[4].veto && !found.comparison.vetoed);
        assert!(seen.insert((found.left, found.right)));
    }
}

#[test]
fn a_minimum_outside_the_unit_interval_is_refused() {
    let (index, _) = index(20, 1);
    for min_probability in [0.0, -0.5, 1.5, f64::NAN] {
        let error = index
            .find_weighted(options(min_probability), false, usize::MAX, usize::MAX)
            .unwrap_err();
        assert_eq!(error.code(), univo_core::ErrorCode::InvalidThreshold);
    }
    // The budgets of `find` apply before any work.
    let error = index
        .find_weighted(options(0.5), true, 10, usize::MAX)
        .unwrap_err();
    assert_eq!(error.code(), univo_core::ErrorCode::PairLimit);
}

#[test]
fn an_exhaustive_report_weighs_every_pair() {
    let (index, person_of) = index(150, 13);
    let report = index
        .find_weighted(options(0.9), true, usize::MAX, usize::MAX)
        .unwrap();
    let correct = report
        .matches
        .iter()
        .filter(|found| person_of[found.left] == person_of[found.right])
        .count();
    assert!(correct > 100 && correct * 100 >= report.matches.len() * 95);
    // Every pair of each partition was scored, vetoed ones included.
    let half = |country: usize| person_of.iter().filter(|&&p| p % 2 == country).count();
    let pairs = |records: usize| records * (records - 1) / 2;
    assert_eq!(report.candidates, pairs(half(0)) + pairs(half(1)));
    assert!(index.is_empty() || !report.weights.fields.is_empty());
}

#[test]
fn an_empty_index_gives_an_empty_report() {
    let index = SearchIndex::new(schema(), 10, None).unwrap();
    let report = find(&index, options(0.5));
    assert!(report.matches.is_empty() && report.total_matches == 0);
    assert_eq!(report.weights.fields.len(), 5);
}

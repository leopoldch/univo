use univo_core::{FieldPath, Map, Schema, Text, Value, prepare};
use univo_engine::{SearchIndex, SearchOptions};

#[test]
fn pairs_keep_global_ranking_and_count_beyond_the_limit() {
    let schema = Schema::builder()
        .id_field(FieldPath::parse("id").unwrap())
        .text(FieldPath::parse("name").unwrap(), Text::default())
        .build()
        .unwrap();
    let records = ["z", "y", "x", "w"]
        .iter()
        .map(|id| {
            let value = Value::Map(
                Map::new(vec![
                    ("id".to_owned(), Value::Str((*id).to_owned())),
                    ("name".to_owned(), Value::Str("Amazon".to_owned())),
                ])
                .unwrap(),
            );
            prepare(&schema, &value).unwrap()
        })
        .collect();
    let mut index = SearchIndex::new(schema, 100, None).unwrap();
    index.add(records).unwrap();
    for limit in [0, 1, 2, 6, 10] {
        let options = SearchOptions {
            min_score: 0.8,
            limit,
            max_entries: usize::MAX,
        };
        let report = index.find(options, false, 100).unwrap();
        let oracle = index.find(options, true, 100).unwrap();
        assert_eq!(report.matches, oracle.matches);
        assert_eq!(report.total_matches, 6);
        assert_eq!(oracle.candidates, 6);
        assert_eq!(report.matches.len(), limit.min(6));
        if limit > 0 {
            assert_eq!((report.matches[0].left, report.matches[0].right), (0, 3));
        }
    }
}

#[test]
fn empty_find_validates_options() {
    let index = SearchIndex::new(Schema::strings(), 100, None).unwrap();
    assert!(
        index
            .find(
                SearchOptions {
                    min_score: 0.0,
                    limit: 0,
                    max_entries: 0
                },
                false,
                100
            )
            .is_err()
    );
}

#[test]
fn exhaustive_pairs_are_budgeted_before_scoring() {
    let schema = Schema::strings();
    let records = ["a", "a", "a"]
        .iter()
        .map(|v| prepare(&schema, &Value::Str((*v).to_owned())).unwrap())
        .collect();
    let mut index = SearchIndex::new(schema, 100, None).unwrap();
    index.add(records).unwrap();
    let options = SearchOptions {
        min_score: 0.8,
        limit: 1,
        max_entries: usize::MAX,
    };
    assert_eq!(
        index.find(options, true, 2).unwrap_err().code().as_str(),
        "limit.pair_count"
    );
    assert_eq!(index.find(options, true, 3).unwrap().total_matches, 3);
    assert_eq!(index.find(options, false, 0).unwrap().total_matches, 3);
}

#[derive(Debug)]
struct EmptyVocabulary(univo_embed::ModelDescriptor);
impl univo_embed::Embedder for EmptyVocabulary {
    fn descriptor(&self) -> &univo_embed::ModelDescriptor {
        &self.0
    }
    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<univo_embed::Embeddings> {
        univo_embed::Embeddings::new("synthetic", vec![0.0; texts.len() * 2], 2)
    }
}

#[test]
fn exact_unknown_semantic_text_survives_without_vectors() {
    let schema = Schema::builder()
        .text(
            FieldPath::parse("text").unwrap(),
            Text {
                mode: univo_core::TextMode::Semantic,
                ..Text::default()
            },
        )
        .build()
        .unwrap();
    let model = EmptyVocabulary(univo_embed::ModelDescriptor {
        source: univo_embed::ModelSource::Local {
            path: "synthetic".into(),
            files: vec![],
        },
        dimension: 2,
        backend: "test",
    });
    let records = ["unknown", "unknown", "different"]
        .iter()
        .map(|text| {
            prepare(
                &schema,
                &Value::Map(
                    Map::new(vec![("text".to_owned(), Value::Str((*text).to_owned()))]).unwrap(),
                ),
            )
            .unwrap()
        })
        .collect();
    let mut index = SearchIndex::new(schema, 100, Some(std::sync::Arc::new(model))).unwrap();
    index.add(records).unwrap();
    let options = SearchOptions {
        min_score: 1.0,
        limit: 10,
        max_entries: usize::MAX,
    };
    let union = index.find(options, false, 100).unwrap();
    let oracle = index.find(options, true, 100).unwrap();
    assert_eq!(union.matches, oracle.matches);
    assert_eq!(union.total_matches, 1);
    assert_eq!(union.matches[0].comparison.score, Some(1.0));
    assert_eq!(union.matches[0].comparison.coverage, 1.0);
    assert_eq!(
        union.matches[0].comparison.fields[0].signal,
        univo_core::Signal::Exact
    );
}

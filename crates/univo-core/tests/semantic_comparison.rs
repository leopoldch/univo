//! Text fields compared with embeddings: semantic fields use them, hybrid
//! and lexical fields are scored lexically.

use univo_core::{
    DISTINCT_WORDS_FACTOR, FieldPath, Map, Schema, Signal, Text, TextMode, Value, compare,
    compare_with_vectors, prepare,
};

fn schema(mode: TextMode) -> Schema {
    Schema::builder()
        .text(
            FieldPath::parse("description").unwrap(),
            Text {
                mode,
                ..Text::default()
            },
        )
        .build()
        .unwrap()
}

fn record(schema: &Schema, text: &str) -> univo_core::PreparedRecord {
    let value =
        Value::Map(Map::new(vec![("description".into(), Value::Str(text.into()))]).unwrap());
    prepare(schema, &value).unwrap()
}

const A: [f32; 2] = [0.6, 0.8];
const B: [f32; 2] = [0.8, 0.6];
const OPPOSITE: [f32; 2] = [-0.6, -0.8];

#[test]
fn semantic_fields_use_the_cosine_of_their_embeddings() {
    let schema = schema(TextMode::Semantic);
    let (left, right) = (record(&schema, "cloud hosting"), record(&schema, "hosting"));

    let result = compare_with_vectors(&schema, &left, &[Some(&A)], &right, &[Some(&B)])
        .unwrap()
        .unwrap();
    let field = &result.fields[0];
    assert_eq!(field.signal, Signal::Semantic);
    assert!((field.similarity.unwrap() - 0.96).abs() < 1e-6);
    assert!((result.score.unwrap() - 0.96).abs() < 1e-6);
    assert_eq!(result.coverage, 1.0);

    let opposite = compare_with_vectors(&schema, &left, &[Some(&A)], &right, &[Some(&OPPOSITE)])
        .unwrap()
        .unwrap();
    assert_eq!(opposite.score, Some(0.0));
}

#[test]
fn texts_with_distinct_words_score_less_and_contradict() {
    // "cloud" and "web" are each a word of their own on one template.
    for mode in [TextMode::Semantic, TextMode::Lexical] {
        let schema = schema(mode);
        let (left, right) = (
            record(&schema, "cloud hosting"),
            record(&schema, "web hosting"),
        );
        let plain = compare(&schema, &left, &record(&schema, "hosting"))
            .unwrap()
            .unwrap();
        let result = compare_with_vectors(&schema, &left, &[Some(&A)], &right, &[Some(&B)])
            .unwrap()
            .unwrap();
        let field = &result.fields[0];
        assert!(field.contradiction, "{mode:?}");
        assert!(!field.veto);
        assert!(!plain.fields[0].contradiction);
        if mode == TextMode::Semantic {
            assert_eq!(field.signal, Signal::Semantic);
            assert!((field.similarity.unwrap() - 0.96 * DISTINCT_WORDS_FACTOR).abs() < 1e-6);
        }
    }
}

#[test]
fn semantic_fields_without_both_embeddings_give_no_evidence() {
    let schema = schema(TextMode::Semantic);
    let (left, right) = (record(&schema, "cloud"), record(&schema, "zzz"));
    for (left_vectors, right_vectors) in [
        (&[Some(&A[..])][..], &[None][..]),
        (&[][..], &[][..]),
        (&[Some(&A[..])][..], &[Some(&[1.0, 0.0, 0.0][..])][..]),
    ] {
        let result = compare_with_vectors(&schema, &left, left_vectors, &right, right_vectors)
            .unwrap()
            .unwrap();
        assert_eq!(result.score, None);
        assert_eq!(result.coverage, 0.0);
        assert_eq!(result.fields[0].signal, Signal::Semantic);
        assert!(!result.fields[0].incomplete);
    }
}

#[test]
fn lexical_and_hybrid_fields_ignore_embeddings() {
    for mode in [TextMode::Lexical, TextMode::Hybrid] {
        let schema = schema(mode);
        let (left, right) = (record(&schema, "car"), record(&schema, "automobile"));
        let plain = compare(&schema, &left, &right).unwrap();
        let embedded =
            compare_with_vectors(&schema, &left, &[Some(&A)], &right, &[Some(&A)]).unwrap();
        assert_eq!(plain, embedded);
        assert_ne!(plain.unwrap().fields[0].signal, Signal::Semantic);
    }
}

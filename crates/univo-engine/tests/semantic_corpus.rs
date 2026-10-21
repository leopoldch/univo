//! Ranking quality on the critical corpus once semantic and hybrid text
//! fields are compared with the default model's embeddings. Runs only when
//! `UNIVO_TEST_MODEL_DIR` names the default model's files. The corpus is a
//! regression set: nothing here tunes a weight or a threshold.

use univo_core::{compare, compare_with_vectors};
use univo_embed::StaticEmbedder;
use univo_engine::embed_records;
use univo_testkit::corpus::{cases, prepare_pair, schema};

/// The AUC with embeddings was 0.776 when this guard was set (U-010), 0.763
/// without; the floor leaves 0.02 for deliberate changes, and a lower value
/// is a regression to explain.
const SEMANTIC_AUC_FLOOR: f64 = 0.756;

/// Mann-Whitney AUC with ties counted as one half.
#[allow(clippy::cast_precision_loss)]
fn auc(positives: &[f32], negatives: &[f32]) -> f64 {
    let mut wins = 0.0;
    for p in positives {
        for n in negatives {
            wins += match p.total_cmp(n) {
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => 0.5,
                std::cmp::Ordering::Less => 0.0,
            };
        }
    }
    wins / (positives.len() as f64 * negatives.len() as f64)
}

#[test]
fn embeddings_rank_duplicates_above_distinct_pairs() {
    let Some(directory) = std::env::var_os("UNIVO_TEST_MODEL_DIR") else {
        eprintln!("skipped: UNIVO_TEST_MODEL_DIR is not set");
        return;
    };
    let model = StaticEmbedder::load_directory(std::path::Path::new(&directory)).unwrap();
    let (mut lexical, mut semantic) = ((Vec::new(), Vec::new()), (Vec::new(), Vec::new()));
    for case in cases() {
        if case["label"] == "invalid" || !case["constraint"].is_null() {
            continue;
        }
        let schema = schema(case["profile"].as_str().unwrap());
        let (left, right) = prepare_pair(&case, &schema);
        let records = [left.unwrap(), right.unwrap()];
        let vectors = embed_records(&schema, &records, &model).unwrap();
        let plain = compare(&schema, &records[0], &records[1]).unwrap();
        let embedded = compare_with_vectors(
            &schema,
            &records[0],
            &vectors.record(0),
            &records[1],
            &vectors.record(1),
        )
        .unwrap();
        for (result, (positives, negatives)) in [(plain, &mut lexical), (embedded, &mut semantic)] {
            let Some(score) = result.and_then(|result| result.score) else {
                continue;
            };
            match case["label"].as_str().unwrap() {
                "duplicate" => positives.push(score),
                "distinct" => negatives.push(score),
                _ => {}
            }
        }
    }
    let lexical_auc = auc(&lexical.0, &lexical.1);
    let semantic_auc = auc(&semantic.0, &semantic.1);
    eprintln!(
        "AUC without embeddings {lexical_auc:.3} ({} + {} pairs), with {semantic_auc:.3} ({} + {} pairs)",
        lexical.0.len(),
        lexical.1.len(),
        semantic.0.len(),
        semantic.1.len()
    );
    assert!(
        semantic_auc >= SEMANTIC_AUC_FLOOR,
        "AUC with embeddings {semantic_auc:.3} below {SEMANTIC_AUC_FLOOR}"
    );
}

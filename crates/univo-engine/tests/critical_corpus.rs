#[path = "../../../tests/rust/corpus.rs"]
mod corpus;

use corpus::{BATCH_LEVEL_CASES, cases, error_name, prepare_pair, schema};
use univo_core::exact_match;
use univo_engine::ExactIndex;

#[test]
fn indexed_search_reproduces_the_versioned_critical_corpus() {
    let cases = cases();
    assert!(cases.len() >= 200);
    let mut batch_errors = 0;
    for case in &cases {
        let id = case["case_id"].as_str().unwrap();
        let schema = schema(case["profile"].as_str().unwrap());
        let (left, right) = prepare_pair(case, &schema);
        let mut index = ExactIndex::new(schema.clone(), 2);
        if case["label"] == "invalid" {
            let expected = case["expected_error"].as_str().unwrap();
            if BATCH_LEVEL_CASES.contains(&id) {
                let error = index.add(vec![left.unwrap(), right.unwrap()]).unwrap_err();
                assert_eq!(error_name(&error), expected, "{id}");
                assert!(index.is_empty(), "{id}");
                batch_errors += 1;
            } else {
                let errors: Vec<_> = [&left, &right]
                    .into_iter()
                    .filter_map(|record| record.as_ref().err().map(error_name))
                    .collect();
                assert_eq!(errors, [expected], "{id}");
            }
            continue;
        }
        let left = left.unwrap();
        let right = right.unwrap();
        let oracle = exact_match(&schema, &left, &right).unwrap();
        assert_eq!(oracle.is_some(), case["expected_kind"] == "exact", "{id}");
        index.add(vec![left.clone(), right]).unwrap();
        let result = index.search(&left, usize::MAX).unwrap();
        // Strings have no identifier, so the query's indexed copy remains eligible.
        let self_match = if left.id().is_none() {
            exact_match(&schema, &left, &left).unwrap()
        } else {
            None
        };
        let expected: Vec<_> = [(0, self_match), (1, oracle)]
            .into_iter()
            .filter_map(|(slot, evidence)| evidence.map(|evidence| (slot, evidence)))
            .collect();
        let actual: Vec<_> = result
            .matches
            .into_iter()
            .map(|hit| (hit.slot, hit.evidence))
            .collect();
        assert_eq!(actual, expected, "{id}");
        assert_eq!(result.total_matches, expected.len(), "{id}");
    }
    assert_eq!(batch_errors, BATCH_LEVEL_CASES.len());
}

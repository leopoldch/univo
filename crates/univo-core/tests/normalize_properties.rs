//! Invariants of the normalizers on arbitrary input.

use proptest::prelude::*;
use univo_core::{Transforms, normalize_hostname, normalize_text};

fn text(input: &str, case_fold: bool) -> (String, Transforms) {
    let mut out = String::new();
    let transforms = normalize_text(input, case_fold, &mut out);
    (out, transforms)
}

fn hostname(input: &str) -> String {
    let mut out = String::new();
    normalize_hostname(input, &mut out);
    out
}

/// Strings biased towards the characters that exercise normalization.
fn interesting() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            any::<char>(),
            prop::char::range('A', 'z'),
            prop::char::range('\u{c0}', '\u{17f}'),
            prop::char::range('\u{300}', '\u{36f}'),
            Just('ß'),
            Just('İ'),
            Just('\u{fb01}'),
            Just('Σ'),
            Just('.'),
        ],
        0..24,
    )
    .prop_map(|characters| characters.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn text_normalization_is_idempotent(input in interesting(), case_fold: bool) {
        let (once, _) = text(&input, case_fold);
        let (twice, transforms) = text(&once, case_fold);
        prop_assert_eq!(&twice, &once);
        prop_assert_eq!(transforms, Transforms::NONE);
    }

    #[test]
    fn transforms_are_empty_iff_text_is_unchanged(input in interesting(), case_fold: bool) {
        let (out, transforms) = text(&input, case_fold);
        prop_assert_eq!(transforms.is_empty(), out == input);
    }

    #[test]
    fn ascii_fast_path_agrees_with_unicode_path(input in "[ -~]{0,32}") {
        // Prefixing a non-ASCII letter that folds to itself forces the
        // general path; the ASCII remainder must be folded identically.
        let (fast, _) = text(&input, true);
        let (general, _) = text(&format!("\u{e9}{input}"), true);
        prop_assert_eq!(general.strip_prefix('\u{e9}'), Some(fast.as_str()));
    }

    #[test]
    fn hostname_normalization_is_idempotent_except_repeated_dots(input in interesting()) {
        let once = hostname(&input);
        let twice = hostname(&once);
        // Only one trailing dot is removed per pass, by definition.
        prop_assert!(twice == once || (once.ends_with('.') && twice == once[..once.len() - 1]));
    }

    #[test]
    fn hostname_never_changes_non_ascii_letters(input in interesting()) {
        let out = hostname(&input);
        let folded_non_ascii = out.chars().filter(|c| !c.is_ascii()).count();
        let composed: String = {
            use unicode_normalization::UnicodeNormalization;
            input.nfc().collect()
        };
        prop_assert_eq!(folded_non_ascii, composed.chars().filter(|c| !c.is_ascii()).count());
    }
}

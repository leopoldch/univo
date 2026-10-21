//! Distinct words: two names built on one template, each with a word of its
//! own, such as "National Institute of Epidemiology" and "National Institute
//! of Oncology". Embeddings and trigrams rate such pairs highly, yet their
//! own words say they name different organizations (ADR 0023).

use std::hash::Hasher;

use rustc_hash::FxHasher;

use crate::acronym::{MAX_INITIALS_SPACES, is_skippable, words};
use crate::lexical::MAX_EDIT_CHARS;

/// Factor applied to the similarity of two names with distinct words.
pub const DISTINCT_WORDS_FACTOR: f32 = 0.8;
/// Shortest word that may stand for a longer one it begins: "univ".
const MIN_PREFIX_CHARS: usize = 3;

/// Whether two names share a content word and each has a content word
/// unrelated to every content word of the other. Words related by a prefix
/// of [`MIN_PREFIX_CHARS`] characters or more, or by an edit similarity of
/// 0.8 or more ([`crate::edit_similarity`]), are not distinct; articles and
/// prepositions are not content words. Descriptions never have distinct
/// words: texts with [`MAX_INITIALS_SPACES`] spaces or more, or more than
/// [`MAX_WORDS`] content words.
pub fn distinct_words(left: &str, right: &str) -> bool {
    let (mut left_buffer, mut right_buffer) = ([""; MAX_WORDS], [""; MAX_WORDS]);
    let (Some(left), Some(right)) = (
        name_words(left, &mut left_buffer),
        name_words(right, &mut right_buffer),
    ) else {
        return false;
    };
    left.iter().any(|word| right.contains(word))
        && has_own_word(left, right)
        && has_own_word(right, left)
}

/// Content words compared per name, whatever separates them: bounds a
/// comparison to `MAX_WORDS²` word pairs.
const MAX_WORDS: usize = 16;

/// The content words of a name, or `None` for a description.
fn name_words<'a, 'b>(
    text: &'a str,
    buffer: &'b mut [&'a str; MAX_WORDS],
) -> Option<&'b [&'a str]> {
    if is_description(text) {
        return None;
    }
    let mut len = 0;
    for word in content_words(text) {
        *buffer.get_mut(len)? = word;
        len += 1;
    }
    Some(&buffer[..len])
}

/// 64 bits summarizing the content words of a name, the bit of each
/// word's hash: two names share a content word only if they share a bit.
/// Zero for descriptions, which never have distinct words.
pub(crate) fn word_bits(text: &str) -> u64 {
    let mut buffer = [""; MAX_WORDS];
    name_words(text, &mut buffer).map_or(0, |words| {
        words.iter().fold(0, |bits, word| {
            let mut hasher = FxHasher::default();
            hasher.write(word.as_bytes());
            bits | 1 << (hasher.finish() >> 58)
        })
    })
}

fn is_description(text: &str) -> bool {
    text.bytes().filter(|&byte| byte == b' ').count() >= MAX_INITIALS_SPACES
}

/// Words other than articles, prepositions and conjunctions.
fn content_words(text: &str) -> impl Iterator<Item = &str> {
    words(text).filter(|word| !is_skippable(word))
}

/// Whether one of `words` is related to none of `others`.
fn has_own_word(words: &[&str], others: &[&str]) -> bool {
    words
        .iter()
        .any(|word| !others.iter().any(|other| related(word, other)))
}

fn related(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    // Lengths in characters: a word may have fewer bytes and more
    // characters than another ("aaaa" and "ééé").
    let (a_chars, b_chars) = (chars(a), chars(b));
    if (a.starts_with(b) && b_chars >= MIN_PREFIX_CHARS)
        || (b.starts_with(a) && a_chars >= MIN_PREFIX_CHARS)
    {
        return true;
    }
    let (short, long) = (a_chars.min(b_chars), a_chars.max(b_chars));
    // An edit similarity of 0.8 is at most `long / 5` edits, the exact
    // integer form of the comparator's threshold for words of up to
    // MAX_EDIT_CHARS characters; longer words have no edit similarity.
    let edits = long / 5;
    long <= MAX_EDIT_CHARS && long - short <= edits && edits > 0 && within_edits(a, b, edits)
}

/// Whether the Levenshtein distance of `a` and `b`, of at most
/// [`MAX_EDIT_CHARS`] characters, is at most `limit`. Stops as soon as
/// every entry of a row exceeds the limit.
fn within_edits(a: &str, b: &str, limit: usize) -> bool {
    let mut b_chars = ['\0'; MAX_EDIT_CHARS];
    let mut m = 0;
    for c in b.chars() {
        b_chars[m] = c;
        m += 1;
    }
    let b_chars = &b_chars[..m];
    let mut previous = [0usize; MAX_EDIT_CHARS + 1];
    let mut current = [0usize; MAX_EDIT_CHARS + 1];
    for (j, entry) in previous.iter_mut().enumerate().take(m + 1) {
        *entry = j;
    }
    for (i, ca) in a.chars().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, &cb) in b_chars.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
            row_min = row_min.min(current[j + 1]);
        }
        if row_min > limit {
            return false;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[m] <= limit
}

fn chars(word: &str) -> usize {
    if word.is_ascii() {
        word.len()
    } else {
        word.chars().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_on_one_template_with_their_own_words_are_distinct() {
        for (a, b) in [
            (
                "national institute of epidemiology",
                "national institute of oncology",
            ),
            (
                "école supérieure de gestion",
                "école supérieure des affaires",
            ),
            ("grimsby institute", "grimsby college"),
        ] {
            assert!(distinct_words(a, b), "{a} / {b}");
            assert!(distinct_words(b, a), "{b} / {a}");
        }
    }

    #[test]
    fn related_or_one_sided_words_are_not_distinct() {
        for (a, b) in [
            // One side only adds words.
            (
                "instituto federal goiano",
                "instituto federal de educação ciência e tecnologia goiano",
            ),
            ("baidu", "baidu china"),
            // A prefix abbreviation and a spelling variant.
            ("univ of tokyo", "university of tokyo"),
            (
                "technische universitat munchen",
                "technische universität münchen",
            ),
            // No shared content word: other evidence decides.
            ("missouri botanical garden", "shaw nature reserve"),
            ("of the", "of the"),
            ("", "national institute"),
            // Equal names.
            (
                "national institute of oncology",
                "national institute of oncology",
            ),
        ] {
            assert!(!distinct_words(a, b), "{a} / {b}");
            assert!(!distinct_words(b, a), "{b} / {a}");
        }
    }

    #[test]
    fn a_short_prefix_and_stop_words_do_not_relate_words() {
        // "ai" begins "aid" but has two characters: not an abbreviation.
        assert!(distinct_words("cancer ai", "cancer aid"));
        assert!(!distinct_words("cancer bio", "cancer biology"));
        // Articles and prepositions alone do not make names distinct.
        assert!(!distinct_words("the university", "university of"));
    }

    #[test]
    fn word_bits_never_hide_distinct_words() {
        let names = [
            "national institute of epidemiology",
            "national institute of oncology",
            "école supérieure de gestion",
            "école supérieure des affaires",
            "grimsby institute",
            "grimsby college",
            "the university",
            "university of tokyo",
            "univ of tokyo",
            "cancer ai",
            "cancer aid",
            "of the",
            "",
        ];
        let mut distinct = 0;
        for a in names {
            for b in names {
                if distinct_words(a, b) {
                    distinct += 1;
                    assert_ne!(word_bits(a) & word_bits(b), 0, "{a} / {b}");
                }
            }
        }
        assert!(distinct >= 10);
        assert_eq!(word_bits("of the"), 0);
        assert_eq!(word_bits(&"many words ".repeat(MAX_INITIALS_SPACES)), 0);
    }

    #[test]
    fn bounded_edits_agree_with_the_edit_similarity_threshold() {
        let words = [
            "",
            "a",
            "ab",
            "abc",
            "abcd",
            "abcde",
            "abdce",
            "bcdea",
            "university",
            "universite",
            "universität",
            "universitat",
            "munchen",
            "münchen",
            "kolej",
            "college",
            "inst",
            "institute",
            "institut",
            "polytechnic",
            "polytechnique",
            "aaaa",
            "ééé",
            "éééé",
            "aéaé",
            "aeae",
        ];
        let long_word = "x".repeat(MAX_EDIT_CHARS + 1);
        let mut variants = 0;
        for a in words.iter().copied().chain([long_word.as_str()]) {
            for b in words.iter().copied().chain([long_word.as_str()]) {
                let expected = a == b
                    || crate::edit_similarity(a, b).is_some_and(|similarity| similarity >= 0.8);
                let shorter = chars(a).min(chars(b));
                let prefix = (a.starts_with(b) || b.starts_with(a)) && shorter >= MIN_PREFIX_CHARS;
                if !prefix {
                    assert_eq!(related(a, b), expected, "{a} / {b}");
                    variants += usize::from(expected && a != b);
                }
            }
        }
        assert!(variants >= 6, "{variants}");
    }

    #[test]
    fn many_words_make_a_description_whatever_the_separators() {
        let words: Vec<String> = (0..10_000).map(|i| format!("w{i:04}")).collect();
        let forward = words.join("-");
        let backward: Vec<&str> = words.iter().rev().map(String::as_str).collect();
        assert!(!distinct_words(&forward, &backward.join("\t")));
        assert_eq!(word_bits(&forward), 0);
        let template = |count: usize| {
            (0..count)
                .map(|i| format!("w{i}"))
                .collect::<Vec<_>>()
                .join("-")
        };
        let shared = template(MAX_WORDS - 1);
        assert!(distinct_words(
            &format!("{shared}-epidemiology"),
            &format!("{shared}/oncology")
        ));
        let shared = template(MAX_WORDS);
        assert!(!distinct_words(
            &format!("{shared}-epidemiology"),
            &format!("{shared}/oncology")
        ));
    }

    #[test]
    fn descriptions_have_no_distinct_words() {
        let description =
            "a large public research university with campuses in many cities ".repeat(2);
        assert!(!distinct_words(
            &description,
            "a large public research college"
        ));
    }
}

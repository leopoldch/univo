//! Lexical similarity of normalized text, as defined in `docs/scoring.md`.
//!
//! Character trigrams are extracted once per prepared field and kept sorted,
//! so that comparing two fields is a linear merge. Edit distance is computed
//! at comparison time and only on short strings, where it is cheap and where
//! trigrams are least informative.

/// Strings with more characters than this are compared by trigrams only.
pub const MAX_EDIT_CHARS: usize = 64;

/// Padding added at both ends so that short strings and word boundaries
/// produce trigrams.
const PADDING: char = ' ';

/// Sorted, deduplicated character trigrams of `text`, padded with two spaces
/// at each end. Each trigram is encoded exactly in a `u64`: three 21-bit code
/// points, so equal codes mean equal trigrams.
pub fn trigrams(text: &str) -> Box<[u64]> {
    let padded = [PADDING, PADDING]
        .into_iter()
        .chain(text.chars())
        .chain([PADDING, PADDING]);
    let mut window = [0u64; 3];
    let mut codes = Vec::with_capacity(text.len() + 2);
    for (index, character) in padded.enumerate() {
        window = [window[1], window[2], u64::from(u32::from(character))];
        if index >= 2 {
            codes.push((window[0] << 42) | (window[1] << 21) | window[2]);
        }
    }
    codes.sort_unstable();
    codes.dedup();
    codes.into_boxed_slice()
}

/// Jaccard index of two sorted, deduplicated sets; 0 when both are empty.
pub fn jaccard(left: &[u64], right: &[u64]) -> f32 {
    let (mut i, mut j, mut shared) = (0, 0, 0usize);
    while i < left.len() && j < right.len() {
        match left[i].cmp(&right[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let union = left.len() + right.len() - shared;
    if union == 0 {
        return 0.0;
    }
    ratio(shared, union)
}

/// `1 - levenshtein(a, b) / max(len)` over characters, or `None` when either
/// string is longer than [`MAX_EDIT_CHARS`].
pub fn edit_similarity(left: &str, right: &str) -> Option<f32> {
    if left.is_ascii() && right.is_ascii() {
        if left.len() > MAX_EDIT_CHARS || right.len() > MAX_EDIT_CHARS {
            return None;
        }
        let longest = left.len().max(right.len());
        if longest == 0 {
            return Some(1.0);
        }
        return Some(
            1.0 - ratio(
                levenshtein_ascii(left.as_bytes(), right.as_bytes()),
                longest,
            ),
        );
    }
    let mut a = [char::MIN; MAX_EDIT_CHARS];
    let mut b = [char::MIN; MAX_EDIT_CHARS];
    let a = fill(&mut a, left)?;
    let b = fill(&mut b, right)?;
    let longest = a.len().max(b.len());
    if longest == 0 {
        return Some(1.0);
    }
    Some(1.0 - ratio(levenshtein(a, b), longest))
}

fn fill<'a>(buffer: &'a mut [char; MAX_EDIT_CHARS], text: &str) -> Option<&'a [char]> {
    let mut length = 0;
    for character in text.chars() {
        *buffer.get_mut(length)? = character;
        length += 1;
    }
    Some(&buffer[..length])
}

/// Levenshtein distance by Myers' bit-parallel algorithm, in Hyyro's
/// formulation for the global distance: one machine word holds a column of
/// the dynamic-programming matrix, so the cost is linear in the text length.
/// The pattern has at most [`MAX_EDIT_CHARS`] = 64 characters.
fn levenshtein(pattern: &[char], text: &[char]) -> usize {
    if pattern.is_empty() {
        return text.len();
    }
    let masks = PatternMasks::new(pattern);
    bit_parallel(
        pattern.len(),
        text.iter().map(|&character| masks.get(character)),
    )
}

/// [`levenshtein`] for ASCII text, with the position masks in a table
/// indexed by byte instead of a search per character.
fn levenshtein_ascii(pattern: &[u8], text: &[u8]) -> usize {
    if pattern.is_empty() {
        return text.len();
    }
    let mut masks = [0u64; 128];
    for (position, &byte) in pattern.iter().enumerate() {
        masks[usize::from(byte & 0x7F)] |= 1 << position;
    }
    bit_parallel(
        pattern.len(),
        text.iter().map(|&byte| masks[usize::from(byte & 0x7F)]),
    )
}

/// The recurrence of Myers' algorithm over the position masks of each text
/// character in the pattern, for a pattern of `length` <= 64 characters.
fn bit_parallel(length: usize, text_masks: impl Iterator<Item = u64>) -> usize {
    let last = 1u64 << (length - 1);
    let mut positive = if length == 64 {
        u64::MAX
    } else {
        (1u64 << length) - 1
    };
    let mut negative = 0u64;
    let mut distance = length;
    for equal in text_masks {
        let vertical = equal | negative;
        let horizontal = ((equal & positive).wrapping_add(positive) ^ positive) | equal;
        let mut horizontal_positive = negative | !(horizontal | positive);
        let mut horizontal_negative = positive & horizontal;
        if horizontal_positive & last != 0 {
            distance += 1;
        }
        if horizontal_negative & last != 0 {
            distance -= 1;
        }
        horizontal_positive = (horizontal_positive << 1) | 1;
        horizontal_negative <<= 1;
        positive = horizontal_negative | !(vertical | horizontal_positive);
        negative = horizontal_positive & vertical;
    }
    distance
}

/// For each pattern character, the bit mask of its positions: a table for
/// ASCII characters, the bulk of most text, and a short list for the others.
struct PatternMasks {
    ascii: [u64; 128],
    others: [(char, u64); MAX_EDIT_CHARS],
    len: usize,
}

impl PatternMasks {
    fn new(pattern: &[char]) -> Self {
        let mut table = Self {
            ascii: [0; 128],
            others: [(char::MIN, 0); MAX_EDIT_CHARS],
            len: 0,
        };
        for (position, &character) in pattern.iter().enumerate() {
            let bit = 1 << position;
            if character.is_ascii() {
                table.ascii[character as usize] |= bit;
                continue;
            }
            if let Some((_, mask)) = table.others[..table.len]
                .iter_mut()
                .find(|(known, _)| *known == character)
            {
                *mask |= bit;
            } else {
                table.others[table.len] = (character, bit);
                table.len += 1;
            }
        }
        table
    }

    fn get(&self, character: char) -> u64 {
        if character.is_ascii() {
            return self.ascii[character as usize];
        }
        self.others[..self.len]
            .iter()
            .find_map(|&(known, mask)| (known == character).then_some(mask))
            .unwrap_or(0)
    }
}

/// `numerator / denominator` for counts bounded by the input sizes.
#[allow(clippy::cast_precision_loss)]
fn ratio(numerator: usize, denominator: usize) -> f32 {
    numerator as f32 / denominator as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigrams_are_padded_sorted_and_unique() {
        let codes = trigrams("aaa");
        // "  aaa  ": "  a", " aa", "aaa", "aa ", "a  "; "aaa" appears once.
        assert_eq!(codes.len(), 5);
        assert!(codes.windows(2).all(|pair| pair[0] < pair[1]));
        // "    " has the single trigram "   ".
        assert_eq!(trigrams("").len(), 1);
    }

    #[test]
    fn jaccard_counts_shared_codes() {
        assert!((jaccard(&[1, 2, 3], &[2, 3, 4]) - 0.5).abs() < 1e-6);
        assert!((jaccard(&[], &[])).abs() < f32::EPSILON);
        assert!((jaccard(&[1], &[1]) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn edit_similarity_matches_known_distances() {
        let similarity = |a, b| edit_similarity(a, b).unwrap();
        assert!((similarity("amazon", "amiazon") - (1.0 - 1.0 / 7.0)).abs() < 1e-6);
        assert!((similarity("kitten", "sitting") - (1.0 - 3.0 / 7.0)).abs() < 1e-6);
        assert!((similarity("", "") - 1.0).abs() < f32::EPSILON);
        assert!(similarity("abc", "").abs() < f32::EPSILON);
        assert!((similarity("é", "e") - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn long_strings_skip_edit_distance() {
        let long = "a".repeat(MAX_EDIT_CHARS + 1);
        assert_eq!(edit_similarity(&long, "a"), None);
        assert!(edit_similarity(&"a".repeat(MAX_EDIT_CHARS), "a").is_some());
    }
}

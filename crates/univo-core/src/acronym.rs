//! Acronym evidence for text: one word that spells the initials of the
//! other text's words, such as "niosh" for "National Institute for
//! Occupational Safety and Health".
//!
//! Short acronyms name many organizations: on the ROR validation split, a
//! three-letter acronym matched another organization's name about as often
//! as its own. Only acronyms of [`MIN_ACRONYM_CHARS`] or more characters
//! count, and their similarity stays below an exact match.

use std::hash::Hasher;

use rustc_hash::FxHasher;

/// Similarity of a text and the acronym of its words.
pub const ACRONYM_SIMILARITY: f32 = 0.9;
/// Fewest characters an acronym must have to give evidence.
pub const MIN_ACRONYM_CHARS: usize = 5;
/// Most characters an acronym may have; longer single words are names.
pub const MAX_ACRONYM_CHARS: usize = 10;
/// Texts with this many spaces or more, descriptions rather than names,
/// have no initials.
pub(crate) const MAX_INITIALS_SPACES: usize = 2 * MAX_ACRONYM_CHARS;

/// Words an acronym may skip, case-folded and sorted: articles, prepositions
/// and conjunctions of the languages most common in organization names.
const SKIPPABLE: &[&str] = &[
    "a", "an", "and", "at", "au", "aux", "d", "da", "das", "de", "degli", "dei", "del", "della",
    "der", "des", "di", "die", "do", "dos", "du", "e", "en", "et", "for", "fur", "für", "i", "in",
    "l", "la", "le", "les", "och", "of", "og", "on", "per", "pour", "the", "und", "van", "von",
    "y", "zu", "à",
];

/// Characters removed inside a single-word acronym: "N.I.O.S.H." and
/// "AT&T" are written with them.
fn is_acronym_punctuation(c: char) -> bool {
    matches!(c, '.' | '-' | '&' | '\'' | '’')
}

/// The characters of the acronym `text` can stand for, before lowercasing:
/// one word of letters and digits once [`is_acronym_punctuation`]
/// characters are removed. `None` for several words or another character.
fn acronym_chars(text: &str) -> Option<impl Iterator<Item = char>> {
    let text = text.trim();
    // Rejects the common case, several words or a long text, at once.
    if text.is_empty() || text.len() > MAX_ACRONYM_CHARS * 8 || text.contains(char::is_whitespace) {
        return None;
    }
    let mut chars = text.chars().filter(|&c| !is_acronym_punctuation(c));
    chars
        .clone()
        .all(char::is_alphanumeric)
        .then_some(std::iter::from_fn(move || chars.next()))
}

/// The words of `text`: runs of letters and digits.
pub(crate) fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
}

pub(crate) fn is_skippable(word: &str) -> bool {
    if word.len() > 6 {
        return false;
    }
    if word.is_ascii() {
        return SKIPPABLE
            .binary_search_by(|skippable| {
                skippable
                    .bytes()
                    .cmp(word.bytes().map(|byte| byte.to_ascii_lowercase()))
            })
            .is_ok();
    }
    SKIPPABLE
        .binary_search_by(|skippable| {
            skippable
                .chars()
                .cmp(word.chars().flat_map(char::to_lowercase))
        })
        .is_ok()
}

/// Lowercased characters, collected as a string or as a hash.
trait Spelling: Default {
    fn push(&mut self, c: char);
    fn char_count(&self) -> usize;
    fn spells_an_acronym(&self) -> bool {
        (MIN_ACRONYM_CHARS..=MAX_ACRONYM_CHARS).contains(&self.char_count())
    }
}

impl Spelling for String {
    fn push(&mut self, c: char) {
        self.extend(c.to_lowercase());
    }
    fn char_count(&self) -> usize {
        self.chars().count()
    }
}

#[derive(Default)]
struct Hashed {
    hasher: FxHasher,
    chars: usize,
}

impl Spelling for Hashed {
    fn push(&mut self, c: char) {
        for lower in c.to_lowercase() {
            self.hasher.write_u32(u32::from(lower));
            self.chars += 1;
        }
    }
    fn char_count(&self) -> usize {
        self.chars
    }
}

impl Hashed {
    /// A nonzero key, or zero outside the acronym length range.
    fn key(&self) -> u32 {
        // Truncation keeps 32 bits of the hash: a collision only costs a
        // comparison of the texts.
        #[allow(clippy::cast_possible_truncation)]
        let key = self.hasher.finish() as u32;
        if self.spells_an_acronym() { key | 1 } else { 0 }
    }
}

fn spell_acronym<S: Spelling>(text: &str) -> Option<S> {
    let mut spelling = S::default();
    for c in acronym_chars(text)? {
        spelling.push(c);
    }
    spelling.spells_an_acronym().then_some(spelling)
}

/// The initials of every word and of the words that are not [`SKIPPABLE`],
/// stopping as soon as both are longer than an acronym can be.
fn spell_initials<S: Spelling>(text: &str) -> [S; 2] {
    let (mut all, mut content) = (S::default(), S::default());
    // A vectorized count rejects long texts before any word is decoded.
    if text.bytes().filter(|&byte| byte == b' ').count() >= MAX_INITIALS_SPACES {
        return [all, content];
    }
    if text.is_ascii() {
        // Most texts: words found byte by byte, without decoding.
        let bytes = text.as_bytes();
        let mut start = None;
        for (at, byte) in bytes.iter().chain([&b' ']).enumerate() {
            match (byte.is_ascii_alphanumeric(), start) {
                (true, None) => start = Some(at),
                (false, Some(from)) => {
                    start = None;
                    all.push(char::from(bytes[from]));
                    if !is_skippable(&text[from..at]) {
                        content.push(char::from(bytes[from]));
                    }
                    if content.char_count() > MAX_ACRONYM_CHARS {
                        break;
                    }
                }
                _ => {}
            }
        }
        return [all, content];
    }
    for word in words(text) {
        let Some(first) = word.chars().next() else {
            continue;
        };
        all.push(first);
        if !is_skippable(word) {
            content.push(first);
        }
        if content.char_count() > MAX_ACRONYM_CHARS {
            break;
        }
    }
    [all, content]
}

/// The acronym `text` can stand for: one word of letters and digits, once
/// [`is_acronym_punctuation`] characters are removed, of
/// [`MIN_ACRONYM_CHARS`] to [`MAX_ACRONYM_CHARS`] characters, lowercased.
pub fn acronym(text: &str) -> Option<String> {
    spell_acronym(text)
}

/// The initials an acronym of `text` can spell: of every word, and of the
/// words that are not [`SKIPPABLE`], lowercased. A word is a run of letters
/// and digits. Initials outside the acronym length range are left out, and
/// a text of [`MAX_INITIALS_SPACES`] spaces or more has none.
pub fn initials(text: &str) -> Vec<String> {
    let mut initials: Vec<String> = spell_initials(text).into();
    initials.dedup();
    initials.retain(Spelling::spells_an_acronym);
    initials
}

/// Hashes of a text's [`acronym`] and [`initials`], computed once when the
/// text is prepared, without allocating, so that comparing two texts that
/// cannot match costs a few integer comparisons. Zero means none; a match
/// of hashes is confirmed on the texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AcronymKeys {
    pub acronym: u32,
    pub initials: [u32; 2],
}

impl AcronymKeys {
    pub fn of(text: &str) -> Self {
        Self {
            acronym: spell_acronym::<Hashed>(text).map_or(0, |spelling| spelling.key()),
            initials: spell_initials::<Hashed>(text).map(|spelling| spelling.key()),
        }
    }

    /// Whether one text may be the acronym of the other's words.
    pub fn may_match(&self, other: &Self) -> bool {
        let spells = |short: &Self, long: &Self| {
            short.acronym != 0 && long.initials.contains(&short.acronym)
        };
        spells(self, other) || spells(other, self)
    }
}

/// [`ACRONYM_SIMILARITY`] when one text is the acronym of the other's words,
/// in either direction; `None` otherwise.
pub fn acronym_similarity(left: &str, right: &str) -> Option<f32> {
    let spells = |short: &str, long: &str| {
        acronym(short).is_some_and(|acronym| initials(long).contains(&acronym))
    };
    (spells(left, right) || spells(right, left)).then_some(ACRONYM_SIMILARITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skippable_words_are_sorted_for_binary_search() {
        assert!(SKIPPABLE.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(SKIPPABLE.iter().all(|word| word.len() <= 5));
    }

    #[test]
    fn an_acronym_is_one_word_of_five_to_ten_characters() {
        assert_eq!(acronym("NIOSH").as_deref(), Some("niosh"));
        assert_eq!(acronym(" n.i.o.s.h. ").as_deref(), Some("niosh"));
        assert_eq!(acronym("APsaA-2").as_deref(), Some("apsaa2"));
        assert_eq!(acronym("IOR"), None);
        assert_eq!(acronym("ACPL"), None);
        assert_eq!(acronym("ABCDEFGHIJK"), None);
        assert_eq!(acronym("NIOSH Lab"), None);
        assert_eq!(acronym("NIO/SH"), None);
        assert_eq!(acronym(""), None);
    }

    #[test]
    fn initials_include_and_skip_function_words() {
        assert_eq!(
            initials("National Institute for Occupational Safety and Health"),
            ["nifosah", "niosh"]
        );
        assert_eq!(initials("Allen County Public Library"), [] as [String; 0]);
        assert_eq!(
            initials("Instituto de Biodiversidad y Biología Experimental y Aplicada"),
            ["idbybeya", "ibbea"]
        );
        assert_eq!(
            initials("Shanghai Nanotechnology Promotion Center Co"),
            ["snpcc"]
        );
        // Twenty spaces: a description, whatever its function words.
        let long = format!("{}alpha beta gamma delta epsilon", "of the ".repeat(8));
        assert_eq!(long.matches(' ').count(), 20);
        assert_eq!(initials(&long), [] as [String; 0]);
        assert_eq!(initials(&long[3..]), ["abgde"]);
    }

    #[test]
    fn similarity_needs_one_side_to_spell_the_others_initials() {
        let long = "National Institute for Occupational Safety and Health";
        assert_eq!(acronym_similarity("NIOSH", long), Some(ACRONYM_SIMILARITY));
        assert_eq!(acronym_similarity(long, "niosh"), Some(ACRONYM_SIMILARITY));
        assert_eq!(acronym_similarity("NIOSHA", long), None);
        assert_eq!(acronym_similarity(long, long), None);
        assert_eq!(acronym_similarity("NIOSH", "NIOSH"), None);
        assert_eq!(acronym_similarity("Bureau of Land Management", "BLM"), None);
    }

    #[test]
    fn keys_filter_exactly_the_pairs_that_can_match() {
        let texts = [
            "National Institute for Occupational Safety and Health",
            "NIOSH",
            "n.i.o.s.h",
            "NIOSHA",
            "Allen County Public Library",
            "acme",
            "",
        ];
        for left in texts {
            for right in texts {
                let keys = AcronymKeys::of(left).may_match(&AcronymKeys::of(right));
                assert_eq!(
                    keys,
                    acronym_similarity(left, right).is_some(),
                    "{left} / {right}"
                );
            }
        }
        assert_eq!(AcronymKeys::of("acme corp"), AcronymKeys::default());
    }

    #[test]
    fn letters_are_compared_without_case() {
        assert_eq!(
            acronym_similarity("ÉCOLE", "école centrale ondes lumière electronique"),
            Some(ACRONYM_SIMILARITY)
        );
        // Accents are letters: "e" does not spell "é".
        assert_eq!(
            acronym_similarity("Ecole", "école centrale ondes lumière electronique"),
            None
        );
    }
}

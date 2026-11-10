//! The BERT `WordPiece` pipeline of Hugging Face `tokenizers` 0.21, as
//! `Model2Vec` models configure it: added tokens split out of the raw text,
//! `BertNormalizer`, `BertPreTokenizer`, then greedy longest-match
//! `WordPiece`. Tokenization yields exactly the ids of `tokenizers` without
//! special tokens; the predicates use the same Unicode tables, those of the
//! `unicode_categories` and `unicode-normalization-alignments` crates.
//!
//! [`Tokenizer::parse`] returns `None` for any other configuration, which
//! the caller hands to `tokenizers` itself.

use std::fmt;

use serde::Deserialize;
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use unicode_categories::UnicodeCategories;
use unicode_normalization_alignments::UnicodeNormalization;

use super::vocab::{Builder, Vocab};

#[derive(Debug)]
pub(crate) struct Tokenizer {
    normalizer: BertNormalizer,
    /// Every vocabulary key, matched at the start of a word.
    words: Vocab,
    /// Keys carrying the continuing-subword prefix, without it.
    continuations: Vocab,
    unk: u32,
    max_chars: usize,
    /// Added tokens matched in the raw text, longest first at a position.
    added: Vec<(Box<str>, u32)>,
    /// Bitmap of the first bytes of the added tokens.
    added_starts: [u64; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
struct BertNormalizer {
    clean_text: bool,
    handle_chinese_chars: bool,
    strip_accents: bool,
    lowercase: bool,
}

/// Reusable buffers of one encoding thread.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    normalized: String,
    decomposed: String,
}

impl Tokenizer {
    /// The pipeline described by `json`, or `None` when it is not the BERT
    /// `WordPiece` pipeline this module reproduces, or not valid.
    pub(crate) fn parse(json: &[u8]) -> Option<Self> {
        let file: File = serde_json::from_slice(json).ok()?;
        let normalizer = file.normalizer?.into_bert()?;
        if !file.pre_tokenizer?.is_bert() {
            return None;
        }
        let Model {
            kind,
            vocab,
            unk_token,
            continuing_subword_prefix,
            max_input_chars_per_word,
        } = file.model;
        if kind != "WordPiece" {
            return None;
        }
        let words = vocab.0?;
        let continuations = words.with_prefix(continuing_subword_prefix.as_bytes());
        let unk = words.get(unk_token.as_bytes())?;

        // `tokenizers` gives an added token the id of the vocabulary entry
        // with its content. One absent from the vocabulary gets an id that
        // depends on the others: not reproduced.
        let mut added: Vec<(Box<str>, u32)> = Vec::new();
        for token in file.added_tokens {
            if token.single_word || token.lstrip || token.rstrip || token.normalized {
                return None;
            }
            if token.content.is_empty() || added.iter().any(|(c, _)| **c == *token.content) {
                continue;
            }
            let id = words.get(token.content.as_bytes())?;
            added.push((token.content.into_boxed_str(), id));
        }
        // At one position, the longest token wins.
        added.sort_by_key(|(content, _)| std::cmp::Reverse(content.len()));
        let mut added_starts = [0u64; 4];
        for (content, _) in &added {
            let byte = content.as_bytes()[0];
            added_starts[usize::from(byte >> 6)] |= 1 << (byte & 63);
        }
        Some(Self {
            normalizer,
            words,
            continuations,
            unk,
            max_chars: max_input_chars_per_word,
            added,
            added_starts,
        })
    }

    pub(crate) fn unk(&self) -> u32 {
        self.unk
    }

    /// Every id tokenization can produce.
    pub(crate) fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.words.ids().chain(self.added.iter().map(|&(_, id)| id))
    }

    /// Appends the ids of `text` to `ids`.
    pub(crate) fn tokenize(&self, text: &str, scratch: &mut Scratch, ids: &mut Vec<u32>) {
        let mut rest = 0;
        if !self.added.is_empty() {
            let bytes = text.as_bytes();
            let mut position = 0;
            while position < bytes.len() {
                let byte = bytes[position];
                if self.added_starts[usize::from(byte >> 6)] & (1 << (byte & 63)) != 0
                    && let Some(&(ref content, id)) = self
                        .added
                        .iter()
                        .find(|(content, _)| bytes[position..].starts_with(content.as_bytes()))
                {
                    self.piece(&text[rest..position], scratch, ids);
                    ids.push(id);
                    position += content.len();
                    rest = position;
                } else {
                    position += 1;
                }
            }
        }
        self.piece(&text[rest..], scratch, ids);
    }

    /// A piece of text between added tokens: normalized, split into words,
    /// each word split into pieces.
    fn piece(&self, piece: &str, scratch: &mut Scratch, ids: &mut Vec<u32>) {
        if piece.is_empty() {
            return;
        }
        let normalized = self.normalize(piece, scratch);
        let mut start = None;
        for (index, c) in normalized.char_indices() {
            if c.is_whitespace() {
                if let Some(begin) = start.take() {
                    self.word(&normalized[begin..index], ids);
                }
            } else if is_bert_punctuation(c) {
                if let Some(begin) = start.take() {
                    self.word(&normalized[begin..index], ids);
                }
                self.word(&normalized[index..index + c.len_utf8()], ids);
            } else if start.is_none() {
                start = Some(index);
            }
        }
        if let Some(begin) = start {
            self.word(&normalized[begin..], ids);
        }
    }

    fn normalize<'a>(&self, piece: &str, scratch: &'a mut Scratch) -> &'a str {
        let options = self.normalizer;
        let out = &mut scratch.normalized;
        out.clear();
        if piece.is_ascii() {
            // Same steps restricted to ASCII: no Chinese character, mark or
            // decomposition, and only ASCII case mappings.
            for &byte in piece.as_bytes() {
                let c = char::from(byte);
                if options.clean_text && (byte == 0 || is_control(c)) {
                    continue;
                }
                let c = if options.clean_text && is_whitespace(c) {
                    ' '
                } else if options.lowercase {
                    c.to_ascii_lowercase()
                } else {
                    c
                };
                out.push(c);
            }
            return out;
        }

        for c in piece.chars() {
            if options.clean_text && (c == '\0' || c == '\u{fffd}' || is_control(c)) {
                continue;
            }
            let c = if options.clean_text && is_whitespace(c) {
                ' '
            } else {
                c
            };
            if options.handle_chinese_chars && is_chinese(c) {
                out.push(' ');
                out.push(c);
                out.push(' ');
            } else {
                out.push(c);
            }
        }
        if options.strip_accents {
            let decomposed = &mut scratch.decomposed;
            decomposed.clear();
            decomposed.extend(
                out.as_str()
                    .nfd()
                    .map(|(c, _)| c)
                    .filter(|c| !c.is_mark_nonspacing()),
            );
            std::mem::swap(out, decomposed);
        }
        if options.lowercase {
            let decomposed = &mut scratch.decomposed;
            decomposed.clear();
            decomposed.extend(out.chars().flat_map(char::to_lowercase));
            std::mem::swap(out, decomposed);
        }
        out
    }

    /// Greedy longest-match-first `WordPiece`; a word with a part outside
    /// the vocabulary, or too long, is one unknown token.
    fn word(&self, word: &str, ids: &mut Vec<u32>) {
        if word.len() > self.max_chars && word.chars().count() > self.max_chars {
            ids.push(self.unk);
            return;
        }
        let first = ids.len();
        let mut start = 0;
        while start < word.len() {
            let table = if start == 0 {
                &self.words
            } else {
                &self.continuations
            };
            // No key is longer than the table's longest: shorter ends only.
            let mut end = word.len().min(start + table.longest());
            while !word.is_char_boundary(end) {
                end -= 1;
            }
            let found = loop {
                if end <= start {
                    break None;
                }
                if let Some(id) = table.get(&word.as_bytes()[start..end]) {
                    break Some(id);
                }
                end -= 1;
                while !word.is_char_boundary(end) {
                    end -= 1;
                }
            };
            let Some(id) = found else {
                ids.truncate(first);
                ids.push(self.unk);
                return;
            };
            ids.push(id);
            start = end;
        }
    }
}

/// BERT whitespace: tab, newline and carriage return, which Unicode counts
/// as controls, and every `White_Space` character.
fn is_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || c.is_whitespace()
}

/// Cc, Cf and Co in the tables `tokenizers` uses, except whitespace.
fn is_control(c: char) -> bool {
    !matches!(c, '\t' | '\n' | '\r') && c.is_other()
}

fn is_chinese(c: char) -> bool {
    matches!(
        u32::from(c),
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B920..=0x2CEAF
            | 0xF900..=0xFAFF
            | 0x2F800..=0x2FA1F
    )
}

fn is_bert_punctuation(c: char) -> bool {
    c.is_ascii_punctuation() || c.is_punctuation()
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    added_tokens: Vec<AddedToken>,
    normalizer: Option<NormalizerConfig>,
    pre_tokenizer: Option<PreTokenizerConfig>,
    model: Model,
}

#[derive(Deserialize)]
#[allow(clippy::struct_excessive_bools)]
struct AddedToken {
    content: String,
    #[serde(default)]
    single_word: bool,
    #[serde(default)]
    lstrip: bool,
    #[serde(default)]
    rstrip: bool,
    #[serde(default)]
    normalized: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum NormalizerConfig {
    BertNormalizer {
        clean_text: bool,
        handle_chinese_chars: bool,
        strip_accents: Option<bool>,
        lowercase: bool,
    },
    #[serde(other)]
    Other,
}

impl NormalizerConfig {
    fn into_bert(self) -> Option<BertNormalizer> {
        match self {
            Self::BertNormalizer {
                clean_text,
                handle_chinese_chars,
                strip_accents,
                lowercase,
            } => Some(BertNormalizer {
                clean_text,
                handle_chinese_chars,
                strip_accents: strip_accents.unwrap_or(lowercase),
                lowercase,
            }),
            Self::Other => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum PreTokenizerConfig {
    BertPreTokenizer,
    #[serde(other)]
    Other,
}

impl PreTokenizerConfig {
    fn is_bert(&self) -> bool {
        matches!(self, Self::BertPreTokenizer)
    }
}

/// The model section, read as a struct: an internally tagged enum would
/// buffer the whole vocabulary before reading it.
#[derive(Deserialize)]
struct Model {
    #[serde(rename = "type")]
    kind: String,
    vocab: VocabField,
    #[serde(default = "default_unk")]
    unk_token: String,
    #[serde(default = "default_prefix")]
    continuing_subword_prefix: String,
    #[serde(default = "default_max_chars")]
    max_input_chars_per_word: usize,
}

fn default_unk() -> String {
    "[UNK]".to_owned()
}

fn default_prefix() -> String {
    "##".to_owned()
}

fn default_max_chars() -> usize {
    100
}

/// A `WordPiece` vocabulary read straight into a [`Vocab`], with no string
/// allocated per key; `None` when it is not a JSON object of ids.
struct VocabField(Option<Vocab>);

impl<'de> Deserialize<'de> for VocabField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(VocabVisitor)
    }
}

struct VocabVisitor;

impl<'de> Visitor<'de> for VocabVisitor {
    type Value = VocabField;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a vocabulary")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<VocabField, A::Error> {
        let mut builder = Builder::default();
        while let Some(start) = map.next_key_seed(KeySeed(builder.arena()))? {
            let id: u32 = map.next_value()?;
            builder.push_last(start, id);
        }
        Ok(VocabField(Some(builder.finish())))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<VocabField, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(VocabField(None))
    }
}

/// Appends a key to the arena and returns where it starts.
struct KeySeed<'a>(&'a mut Vec<u8>);

impl<'de> DeserializeSeed<'de> for KeySeed<'_> {
    type Value = usize;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<usize, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for KeySeed<'_> {
    type Value = usize;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a token")
    }

    fn visit_str<E: serde::de::Error>(self, key: &str) -> Result<usize, E> {
        let start = self.0.len();
        self.0.extend_from_slice(key.as_bytes());
        Ok(start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bert(lowercase: bool, extra_added: &str) -> Vec<u8> {
        format!(
            r###"{{"version":"1.0","truncation":null,"padding":null,
            "added_tokens":[{{"id":0,"content":"[UNK]","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}}{extra_added}],
            "normalizer":{{"type":"BertNormalizer","clean_text":true,"handle_chinese_chars":true,"strip_accents":null,"lowercase":{lowercase}}},
            "pre_tokenizer":{{"type":"BertPreTokenizer"}},
            "post_processor":null,"decoder":null,
            "model":{{"type":"WordPiece","unk_token":"[UNK]","continuing_subword_prefix":"##","max_input_chars_per_word":100,
            "vocab":{{"[UNK]":0,"[CLS]":1,"un":2,"##able":3,"##a":4,"e":5,"!":6,"中":7,"cafe":8,"\"q":9}}}}}}"###
        )
        .into_bytes()
    }

    fn ids(tokenizer: &Tokenizer, text: &str) -> Vec<u32> {
        let mut out = Vec::new();
        tokenizer.tokenize(text, &mut Scratch::default(), &mut out);
        out
    }

    #[test]
    fn reproduces_the_bert_pipeline() {
        let added = r#",{"id":1,"content":"[CLS]","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}"#;
        let tokenizer = Tokenizer::parse(&bert(true, added)).unwrap();
        assert_eq!(ids(&tokenizer, "Unable!"), [2, 3, 6]);
        assert_eq!(ids(&tokenizer, "unablex e"), [0, 5]);
        assert_eq!(ids(&tokenizer, "Café\u{7}中"), [8, 7]);
        assert_eq!(ids(&tokenizer, "e[CLS]e [cls]"), [5, 1, 5, 0, 0, 0]);
        assert_eq!(ids(&tokenizer, " \t\n"), [] as [u32; 0]);
    }

    #[test]
    fn other_pipelines_are_not_claimed() {
        let base = String::from_utf8(bert(true, "")).unwrap();
        for (from, to) in [
            ("BertNormalizer", "Lowercase"),
            ("BertPreTokenizer", "Whitespace"),
            (r#""type":"WordPiece""#, r#""type":"WordLevel""#),
            (r#""normalized":false"#, r#""normalized":true"#),
            (r#""content":"[UNK]""#, r#""content":"[NEW]""#),
        ] {
            assert!(
                Tokenizer::parse(base.replace(from, to).as_bytes()).is_none(),
                "{to}"
            );
        }
        assert!(Tokenizer::parse(b"not json").is_none());
    }
}

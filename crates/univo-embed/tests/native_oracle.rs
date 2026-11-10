//! The native encoder against `model2vec-rs` 0.3.0 and `tokenizers` 0.21,
//! the backend it replaces: vectors must agree bit for bit.
//!
//! The real tokenizer comes from `UNIVO_TEST_MODEL_DIR` (the default
//! model's files); without it, these tests are skipped. Every tokenizer
//! option, embedding type, weight and mapping is exercised on that
//! vocabulary with tables built here. Release builds only: debug builds
//! are too slow for the sweep over every code point.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]

mod support;

use std::path::{Path, PathBuf};

use model2vec_rs::model::StaticModel;
use support::TempDir;
use univo_embed::{Embedder, StaticEmbedder};

fn model_dir() -> Option<PathBuf> {
    let directory = PathBuf::from(std::env::var_os("UNIVO_TEST_MODEL_DIR")?);
    directory
        .join("tokenizer.json")
        .is_file()
        .then_some(directory)
}

/// Deterministic pseudo-random numbers (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        #[allow(clippy::cast_possible_truncation)]
        let value = (self.next() % n as u64) as usize;
        value
    }
}

/// Characters that exercise every step of the BERT pipeline.
fn alphabet() -> Vec<char> {
    let mut chars: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
        .chars()
        .collect();
    chars.extend(
        " \t\n\r\u{b}\u{c}\u{0}\u{7f}\u{85}\u{a0}\u{2028}\u{3000}\u{200b}\u{200d}\u{feff}\u{fffd}"
            .chars(),
    );
    chars.extend("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~¡¿«»„“”‘’—–…·•§¶†‡※".chars());
    chars.extend("éèêëÉÈàâäÀçÇñÑöÖüÜßẞøØåÅæÆœŒıİşŞğĞłŁžŽčČřŘőŐ".chars());
    // Combining marks, alone and after letters, to test NFD reordering.
    chars.extend(
        "\u{300}\u{301}\u{302}\u{308}\u{30a}\u{323}\u{327}\u{328}\u{338}\u{20d7}\u{1dc0}".chars(),
    );
    chars.extend("中文日本語한국어ひらがなカタカナ𠀀𪚥丂".chars());
    chars.extend(
        "Ελληνικάκόσμεςσ Кириллица ЁёЇї עברית العربية ﷺ हिन्दी क्ष ไทยภาษา ກາ ქართული አማርኛ".chars(),
    );
    chars.extend("ﬁﬂﬃ℃Ⅻ①ｶﾞＡｂ²½™Ω".chars());
    chars.extend("😀👍🏽👨‍👩‍👧🇫🇷\u{e000}\u{f8ff}\u{10ffff}\u{e0001}\u{1f9ff}".chars());
    chars
}

fn random_texts(count: usize, seed: u64) -> Vec<String> {
    let alphabet = alphabet();
    let pieces = [
        "[CLS]",
        "[SEP]",
        "[UNK]",
        "[MASK]",
        "[PAD]",
        "[CL",
        "S]",
        "[[CLS]]",
        "##",
        "##ing",
        "unbelievable",
        "Société Générale",
        "Université",
        "İstanbul",
        "ǅ",
        "ΣΑΣ",
    ];
    let mut rng = Rng(seed);
    (0..count)
        .map(|_| {
            let length = rng.below(40);
            let mut text = String::new();
            for _ in 0..length {
                if rng.below(8) == 0 {
                    text.push_str(pieces[rng.below(pieces.len())]);
                } else {
                    text.push(alphabet[rng.below(alphabet.len())]);
                }
            }
            text
        })
        .collect()
}

/// Every Unicode scalar value, alone, inside a word and between words.
fn code_point_texts() -> Vec<String> {
    let chars: Vec<char> = (0..=0x10_ffffu32).filter_map(char::from_u32).collect();
    let mut texts = Vec::new();
    for group in chars.chunks(8) {
        let mut inside = String::from("ab");
        let mut spaced = String::new();
        for &c in group {
            inside.push(c);
            spaced.push(c);
            spaced.push(' ');
        }
        inside.push_str("cd");
        texts.push(inside);
        texts.push(spaced);
    }
    for c in chars.iter().step_by(7) {
        texts.push(c.to_string());
    }
    texts
}

/// Names and descriptions from the repository's fixtures, plus any extra
/// file named by `UNIVO_ORACLE_TEXTS` (JSON lines with `left`/`right`).
fn corpus_texts() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut texts = Vec::new();
    let mut files = vec![
        root.join("unicode/strings.jsonl"),
        root.join("critical/cases.jsonl"),
    ];
    if let Some(extra) = std::env::var_os("UNIVO_ORACLE_TEXTS") {
        files.push(PathBuf::from(extra));
    }
    for file in files {
        for line in std::fs::read_to_string(&file).unwrap().lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            collect_strings(&value, &mut texts);
        }
    }
    texts.sort();
    texts.dedup();
    texts
}

fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => out.push(text.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| collect_strings(v, out)),
        _ => {}
    }
}

/// A tensor: name, dtype, shape and little-endian bytes.
type TensorSpec<'a> = (&'a str, &'a str, Vec<usize>, Vec<u8>);

/// A model directory with `tokenizer` and a `safetensors` file of
/// `tensors`.
fn write_model(label: &str, tokenizer: &[u8], tensors: &[TensorSpec<'_>]) -> TempDir {
    let directory = TempDir::new(label);
    let mut header = serde_json::Map::new();
    let mut data = Vec::new();
    for (name, dtype, shape, bytes) in tensors {
        header.insert(
            (*name).to_owned(),
            serde_json::json!({"dtype": dtype, "shape": shape, "data_offsets": [data.len(), data.len() + bytes.len()]}),
        );
        data.extend_from_slice(bytes);
    }
    let mut header = serde_json::to_vec(&header).unwrap();
    header.resize(header.len().next_multiple_of(8), b' ');
    let mut file = (header.len() as u64).to_le_bytes().to_vec();
    file.extend_from_slice(&header);
    file.extend_from_slice(&data);
    std::fs::write(directory.path().join("model.safetensors"), file).unwrap();
    std::fs::write(directory.path().join("tokenizer.json"), tokenizer).unwrap();
    std::fs::write(
        directory.path().join("config.json"),
        br#"{"model_type": "model2vec", "normalize": true}"#,
    )
    .unwrap();
    directory
}

/// Both backends on `directory`; panics on the first differing vector.
fn assert_same_vectors(directory: &Path, texts: &[String], what: &str) {
    assert_same_vectors_on(directory, texts, what, true);
}

/// Univo against `model2vec-rs`, with Univo expected to run natively or
/// to hand the model to `model2vec-rs`.
fn assert_same_vectors_on(directory: &Path, texts: &[String], what: &str, natively: bool) {
    let native = StaticEmbedder::load_directory(directory).unwrap();
    assert_eq!(
        native.runs_natively(),
        natively,
        "{what}: unexpected backend"
    );
    let read = |file: &str| std::fs::read(directory.join(file)).unwrap();
    // As Univo's `model2vec-rs` path: truncation and padding removed.
    let mut tokenizer: serde_json::Value = serde_json::from_slice(&read("tokenizer.json")).unwrap();
    tokenizer["truncation"] = serde_json::Value::Null;
    tokenizer["padding"] = serde_json::Value::Null;
    let oracle = StaticModel::from_bytes(
        serde_json::to_vec(&tokenizer).unwrap(),
        read("model.safetensors"),
        read("config.json"),
        Some(true),
    )
    .unwrap();

    let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
    let ours = native.encode_batch(&borrowed).unwrap();
    let dimension = ours.dimension();
    for (start, chunk) in texts.chunks(4096).enumerate() {
        let theirs = oracle.encode_with_args(chunk, None, 1024);
        for (offset, (text, expected)) in chunk.iter().zip(&theirs).enumerate() {
            let index = start * 4096 + offset;
            let row = &ours.as_slice()[index * dimension..(index + 1) * dimension];
            let same = row
                .iter()
                .zip(expected)
                .all(|(a, b)| a.to_bits() == b.to_bits());
            if !same {
                let shown: String = text.chars().take(60).collect();
                panic!(
                    "{what}: vectors differ for {shown:?} ({} chars): {:?} against {:?}",
                    text.chars().count(),
                    &row[..4],
                    &expected[..4]
                );
            }
        }
    }
}

fn int8_table(rows: usize, dimension: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed);
    (0..rows * dimension)
        .map(|_| {
            #[allow(clippy::cast_possible_truncation)]
            let byte = rng.next() as u8;
            byte
        })
        .collect()
}

fn vocab_size(tokenizer: &[u8]) -> usize {
    let json: serde_json::Value = serde_json::from_slice(tokenizer).unwrap();
    json["model"]["vocab"].as_object().unwrap().len()
}

#[test]
#[cfg_attr(debug_assertions, ignore = "release builds only")]
fn the_default_model_encodes_every_text_as_model2vec_rs() {
    let Some(directory) = model_dir() else {
        eprintln!("skipped: UNIVO_TEST_MODEL_DIR is not set");
        return;
    };
    let mut texts = code_point_texts();
    texts.extend(random_texts(200_000, 1));
    texts.extend(corpus_texts());
    texts.push("word ".repeat(40_000));
    texts.push("e ".repeat(200_000));
    texts.push("x".repeat(101));
    texts.push("a".repeat(100));
    texts.push(String::new());
    assert_same_vectors(&directory, &texts, "default model");
}

#[test]
#[cfg_attr(debug_assertions, ignore = "release builds only")]
fn every_normalizer_option_matches() {
    let Some(directory) = model_dir() else {
        eprintln!("skipped: UNIVO_TEST_MODEL_DIR is not set");
        return;
    };
    let tokenizer = std::fs::read(directory.join("tokenizer.json")).unwrap();
    let rows = vocab_size(&tokenizer);
    let table = int8_table(rows, 16, 7);
    let texts = random_texts(20_000, 2);
    for clean_text in [true, false] {
        for handle_chinese_chars in [true, false] {
            for lowercase in [true, false] {
                for strip_accents in [None, Some(true), Some(false)] {
                    let mut json: serde_json::Value = serde_json::from_slice(&tokenizer).unwrap();
                    json["normalizer"] = serde_json::json!({
                        "type": "BertNormalizer",
                        "clean_text": clean_text,
                        "handle_chinese_chars": handle_chinese_chars,
                        "strip_accents": strip_accents,
                        "lowercase": lowercase,
                    });
                    let what = format!(
                        "clean {clean_text}, chinese {handle_chinese_chars}, lowercase {lowercase}, accents {strip_accents:?}"
                    );
                    let model = write_model(
                        "options",
                        &serde_json::to_vec(&json).unwrap(),
                        &[("embeddings", "I8", vec![rows, 16], table.clone())],
                    );
                    assert_same_vectors(model.path(), &texts, &what);
                }
            }
        }
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "release builds only")]
fn every_table_layout_matches() {
    let Some(directory) = model_dir() else {
        eprintln!("skipped: UNIVO_TEST_MODEL_DIR is not set");
        return;
    };
    let tokenizer = std::fs::read(directory.join("tokenizer.json")).unwrap();
    let vocab = vocab_size(&tokenizer);
    let dimension = 12;
    let rows = 5000;
    let mut rng = Rng(3);
    let mapping: Vec<u8> = (0..vocab)
        .flat_map(|_| (rng.below(rows) as i64).to_le_bytes())
        .collect();
    let weights: Vec<u8> = (0..vocab)
        .flat_map(|_| (0.25 + rng.below(1000) as f32 / 300.0).to_le_bytes())
        .collect();
    let f32_table: Vec<u8> = (0..rows * dimension)
        .flat_map(|_| ((rng.below(20001) as f32 - 10000.0) / 997.0).to_le_bytes())
        .collect();
    let f16_table: Vec<u8> = (0..rows * dimension)
        .flat_map(|_| {
            // Finite halves only: exponent below 31.
            let bits = (rng.next() as u16) & 0xfbff;
            bits.to_le_bytes()
        })
        .collect();
    let int8 = int8_table(rows, dimension, 4);
    let mut texts = random_texts(20_000, 5);
    texts.push("e ".repeat(200_000));
    let shape = vec![rows, dimension];
    let cases: Vec<(&str, Vec<TensorSpec<'_>>)> = vec![
        (
            "int8 mapped",
            vec![
                ("embeddings", "I8", shape.clone(), int8.clone()),
                ("mapping", "I64", vec![vocab], mapping.clone()),
            ],
        ),
        (
            "int8 weighted",
            vec![
                ("embeddings", "I8", shape.clone(), int8.clone()),
                ("mapping", "I64", vec![vocab], mapping.clone()),
                ("weights", "F32", vec![vocab], weights.clone()),
            ],
        ),
        (
            "f32 mapped",
            vec![
                ("embeddings", "F32", shape.clone(), f32_table.clone()),
                ("mapping", "I64", vec![vocab], mapping.clone()),
            ],
        ),
        (
            "f32 weighted",
            vec![
                ("0", "F32", shape.clone(), f32_table),
                ("mapping", "I64", vec![vocab], mapping.clone()),
                ("weights", "F32", vec![vocab], weights),
            ],
        ),
        (
            "f16 mapped",
            vec![
                ("embedding.weight", "F16", shape, f16_table),
                ("mapping", "I64", vec![vocab], mapping),
            ],
        ),
    ];
    for (what, tensors) in cases {
        let model = write_model("tables", &tokenizer, &tensors);
        assert_same_vectors(model.path(), &texts, what);
    }
}

#[test]
fn a_token_without_a_row_is_refused_at_load() {
    let tokenizer = br###"{"added_tokens":[],"normalizer":{"type":"BertNormalizer","clean_text":true,"handle_chinese_chars":true,"strip_accents":null,"lowercase":true},"pre_tokenizer":{"type":"BertPreTokenizer"},"model":{"type":"WordPiece","unk_token":"[UNK]","continuing_subword_prefix":"##","max_input_chars_per_word":100,"vocab":{"[UNK]":0,"a":1,"b":2}}}"###;
    let model = write_model(
        "rows",
        tokenizer,
        &[("embeddings", "F32", vec![2, 2], vec![0; 16])],
    );
    let error = StaticEmbedder::load_directory(model.path()).unwrap_err();
    assert_eq!(error.code(), univo_core::ErrorCode::CorruptModel);

    let model = write_model(
        "rows",
        tokenizer,
        &[(
            "embeddings",
            "F32",
            vec![3, 2],
            [1.0f32, 0.0, 0.0, 1.0, 0.6, 0.8]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect(),
        )],
    );
    let embedder = StaticEmbedder::load_directory(model.path()).unwrap();
    assert!(embedder.runs_natively());
    let vectors = embedder.encode_batch(&["a", "b", "c", "ab"]).unwrap();
    assert_eq!(vectors.row(0), [0.0, 1.0]);
    assert_eq!(vectors.row(1), [0.6, 0.8]);
    assert_eq!(vectors.absent(), [2, 3]);
}

/// A BERT tokenizer over a few words, with `post_processor`.
fn small_bert(post_processor: &str) -> Vec<u8> {
    format!(
        r###"{{"added_tokens":[{{"id":0,"content":"[UNK]","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}},{{"id":3,"content":"[CLS]","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true}}],
        "normalizer":{{"type":"BertNormalizer","clean_text":true,"handle_chinese_chars":true,"strip_accents":null,"lowercase":true}},
        "pre_tokenizer":{{"type":"BertPreTokenizer"}},"post_processor":{post_processor},
        "decoder":{{"type":"WordPiece","prefix":"##","cleanup":true}},
        "model":{{"type":"WordPiece","unk_token":"[UNK]","continuing_subword_prefix":"##","max_input_chars_per_word":100,"vocab":{{"[UNK]":0,"a":1,"b":2,"[CLS]":3}}}}}}"###
    )
    .into_bytes()
}

fn rows(values: &[[i8; 2]]) -> Vec<u8> {
    values.iter().flatten().map(|v| v.cast_unsigned()).collect()
}

#[test]
fn post_processors_keep_their_meaning() {
    let table = rows(&[[0, 0], [3, 4], [-4, 3], [1, 1]]);
    let cls = r#"{"SpecialToken":{"id":"[CLS]","type_id":0}}"#;
    let a = r#"{"Sequence":{"id":"A","type_id":0}}"#;
    let b = r#"{"Sequence":{"id":"B","type_id":1}}"#;
    let special = r#"{"[CLS]":{"id":"[CLS]","ids":[3],"tokens":["[CLS]"]}}"#;
    let template = |single: &str| {
        format!(
            r#"{{"type":"TemplateProcessing","single":[{single}],"pair":[{cls},{a},{b}],"special_tokens":{special}}}"#
        )
    };
    let texts: Vec<String> = ["a", "b a", "a [CLS] b", "", "zz"]
        .map(str::to_owned)
        .to_vec();
    for (processor, natively) in [
        ("null".to_owned(), true),
        (
            r#"{"type":"BertProcessing","sep":["[CLS]",3],"cls":["[CLS]",3]}"#.to_owned(),
            true,
        ),
        (template(&format!("{cls},{a},{cls}")), true),
        // Drops the text: every vector is absent with `tokenizers`.
        (template(cls), false),
        // Repeats the text.
        (template(&format!("{a},{a}")), false),
    ] {
        let model = write_model(
            "processor",
            &small_bert(&processor),
            &[("embeddings", "I8", vec![4, 2], table.clone())],
        );
        assert_same_vectors_on(model.path(), &texts, &processor, natively);
    }
    let model = write_model(
        "processor",
        &small_bert(&template(cls)),
        &[("embeddings", "I8", vec![4, 2], table)],
    );
    let vectors = StaticEmbedder::load_directory(model.path())
        .unwrap()
        .encode_batch(&["a"])
        .unwrap();
    assert_eq!(vectors.absent(), [0]);
}

#[test]
fn int8_sums_round_as_f32_beyond_the_exact_bound() {
    // -128 · 131,072 = -2^24 is the last exact partial sum.
    let table = rows(&[[0, 0], [-128, 1], [-1, 1], [0, 0]]);
    let model = write_model(
        "bound",
        &small_bert("null"),
        &[("embeddings", "I8", vec![4, 2], table)],
    );
    let exact = 1 << 17;
    let texts: Vec<String> = [exact - 1, exact, exact + 1, exact + 2]
        .iter()
        .flat_map(|&n| {
            [
                format!("{}b b", "a ".repeat(n)),
                format!("{}b", "a ".repeat(n)),
                "a ".repeat(n),
            ]
        })
        .collect();
    assert_same_vectors(model.path(), &texts, "int8 bound");
}

#[test]
fn a_shape_too_large_to_address_is_refused_at_load() {
    for shape in [
        vec![2, 1 << 63],
        vec![1 << 32, 1 << 32],
        vec![4, (1 << 62) + 1],
    ] {
        let model = write_model(
            "shape",
            &small_bert("null"),
            &[("embeddings", "I8", shape.clone(), Vec::new())],
        );
        let error = StaticEmbedder::load_directory(model.path()).unwrap_err();
        assert_eq!(
            error.code(),
            univo_core::ErrorCode::CorruptModel,
            "{shape:?}"
        );
    }
}
